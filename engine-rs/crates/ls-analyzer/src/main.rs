//! ls-analyzer sidecar（T-327/T-331）：监听 127.0.0.1:<port>，一次性认证令牌经启动参数传入
//! （--token <secret>，Python 父进程生成），防本机其它进程未授权调用。
//!
//! 数据面（T-331/T-325）：ApplyConfig 下发全量快照 → StartCamera 启动 ffmpeg 取流
//! （ls-ingest）→ YOLO 检测（tract 纯 Rust；模型缺失时回退 NullDetector）→ ByteTrack
//! → 区域迟滞/规则冷却（pipeline）→ EventCandidate 经 gRPC 事件流推送；
//! Status 返回实时 fps/帧龄/重连/错误。
//! 启动即打印 "READY port=<p>" 行，供父进程健康判定。
#![allow(clippy::result_large_err)]

mod golden;
mod pipeline;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use ls_infer::yolo::YoloModel;
use ls_ingest::{Ingest, IngestConfig};
use ls_proto::analyzer::analyzer_server::{Analyzer, AnalyzerServer};
use ls_proto::analyzer::{
    Ack, AnalyzerStatus, CameraRef, CameraStatus, Config, Empty, EventCandidate, EventsRequest,
    EvidenceRequest, EvidenceResult, Jpeg,
};

use pipeline::{CameraPipeline, CameraSpec, Candidate, Detector, NullDetector, RuleSpec, ZoneSpec};

#[derive(Clone, Debug)]
struct CamState {
    state: String, // running | stopped | error
    fps_now: f32,
    frame_age_s: f32,
    reconnect_count: u64,
    last_error: String,
    infer_ms_p50: f32,
    queue_depth: u64,
    last_frame_at: Option<Instant>,
}

impl Default for CamState {
    fn default() -> Self {
        CamState {
            state: "stopped".into(),
            fps_now: 0.0,
            frame_age_s: 0.0,
            reconnect_count: 0,
            last_error: String::new(),
            infer_ms_p50: 0.0,
            queue_depth: 0,
            last_frame_at: None,
        }
    }
}

struct AppStateInner {
    token: String,
    config_version: i64,
    cameras_cfg: HashMap<i64, CameraSpec>,
    zones_cfg: Vec<ZoneSpec>,
    rules_cfg: Vec<RuleSpec>,
    calendar: HashMap<String, String>,
    cams: HashMap<i64, Arc<Mutex<CamState>>>,
    running: HashMap<i64, Arc<AtomicBool>>,
    /// 每路 ffmpeg 进程句柄（控制面 stop_camera 可直接强杀，打断阻塞读）
    ffmpeg: HashMap<i64, ls_ingest::ChildSlot>,
    /// 模型路径（空串=不启用检测；按实例注入，避免进程级环境变量竞态）
    model_path: Option<PathBuf>,
    /// 每个事件流订阅者的发送端（数据面线程 try_send 非阻塞广播）
    event_txs: Vec<mpsc::Sender<Result<EventCandidate, Status>>>,
}

type AppState = Arc<Mutex<AppStateInner>>;

/// 单路作业快照（避免数据面线程长期持锁）。
struct CameraJob {
    spec: CameraSpec,
    zones: Vec<ZoneSpec>,
    rules: Vec<RuleSpec>,
    calendar: HashMap<String, String>,
    stop: Arc<AtomicBool>,
    /// 外部可强杀 ffmpeg 的共享句柄：stop_camera 立刻杀进程 → 读管道 EOF
    ffmpeg: ls_ingest::ChildSlot,
    /// 模型路径（None=空检测）
    model_path: Option<PathBuf>,
    cam: Arc<Mutex<CamState>>,
}

pub struct AnalyzerImpl {
    state: AppState,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AnalyzerImpl {
    fn new(token: String, model_path: Option<PathBuf>) -> Self {
        AnalyzerImpl {
            state: Arc::new(Mutex::new(AppStateInner {
                token,
                config_version: 0,
                cameras_cfg: HashMap::new(),
                zones_cfg: Vec::new(),
                rules_cfg: Vec::new(),
                calendar: HashMap::new(),
                cams: HashMap::new(),
                running: HashMap::new(),
                ffmpeg: HashMap::new(),
                model_path,
                event_txs: vec![],
            })),
        }
    }

    fn check_auth<T>(&self, req: &Request<T>) -> Result<(), Status> {
        let got = req
            .metadata()
            .get("x-ls-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let want = lock(&self.state).token.clone();
        if want.is_empty() || constant_eq(got, &want) {
            Ok(())
        } else {
            Err(Status::unauthenticated("invalid token"))
        }
    }
}

/// 常量时间比较（防时序侧信道）。
fn constant_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|v: &f32| *v > 0.0)
        .unwrap_or(default)
}

/// YOLO 检测器（tract 纯 Rust）。PPE 分类尚未接入 → ppe=Unknown（管线走"未启用"分支）。
struct YoloDetector {
    model: YoloModel,
}

impl Detector for YoloDetector {
    fn detect(&mut self, frame: &ls_ingest::Frame) -> Vec<pipeline::PersonDetection> {
        match self.model.detect(&frame.data, frame.width, frame.height) {
            Ok(dets) => dets
                .into_iter()
                .map(|d| pipeline::PersonDetection {
                    box_: d.box_,
                    score: d.score,
                    ppe: pipeline::PpeStatus::Unknown,
                })
                .collect(),
            Err(e) => {
                eprintln!("[ls-analyzer] 推理失败（本帧跳过）: {e}");
                Vec::new()
            }
        }
    }
}

/// 构造检测器：模型路径由 AnalyzerImpl 实例注入（生产从 LS_MODEL_FP32 / 仓库布局
/// 启动时解析一次；测试可各自指定或设 None，避免进程级 set_var 并发竞态）。
fn make_detector(path: Option<&PathBuf>) -> Box<dyn Detector> {
    let Some(path) = path else {
        println!("[ls-analyzer] 未配置模型——使用空检测器");
        return Box::new(NullDetector);
    };
    let imgsz = env_u32("LS_IMGSZ", 640);
    let conf = env_f32("LS_CONF_THRES", 0.40);
    let iou = env_f32("LS_IOU_THRES", 0.50);
    let min_area = env_f32("LS_MIN_BOX_AREA", 1200.0);
    match YoloModel::load(path, imgsz, conf, iou, min_area, None) {
        Ok(model) => {
            println!(
                "[ls-analyzer] 模型已加载: {}（imgsz={imgsz} conf={conf} iou={iou}）",
                path.display()
            );
            Box::new(YoloDetector { model })
        }
        Err(e) => {
            println!("[ls-analyzer] 模型加载失败（回退空检测）: {e}");
            Box::new(NullDetector)
        }
    }
}

/// 启动/重启一路摄像头数据面线程（无配置时仅记录 running，兼容旧客户端）。
fn spawn_camera(state: AppState, cid: i64) {
    let job = {
        let mut st = lock(&state);
        if let Some(old) = st.running.remove(&cid) {
            old.store(true, Ordering::SeqCst);
        }
        if let Some(slot) = st.ffmpeg.remove(&cid) {
            Ingest::kill_slot(&slot);
        }
        let cam = Arc::new(Mutex::new(CamState {
            state: "running".into(),
            ..Default::default()
        }));
        st.cams.insert(cid, Arc::clone(&cam));
        let Some(spec) = st.cameras_cfg.get(&cid).cloned() else {
            // 配置文件缺失（旧版 Python 客户端）——保持生命周期语义，不启动取流
            return;
        };
        let zones = st
            .zones_cfg
            .iter()
            .filter(|z| z.camera_id == cid)
            .cloned()
            .collect();
        let rules = st
            .rules_cfg
            .iter()
            .filter(|r| r.camera_id == cid)
            .cloned()
            .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let ffmpeg: ls_ingest::ChildSlot = Arc::new(Mutex::new(None));
        st.running.insert(cid, Arc::clone(&stop));
        st.ffmpeg.insert(cid, Arc::clone(&ffmpeg));
        CameraJob {
            spec,
            zones,
            rules,
            calendar: st.calendar.clone(),
            stop,
            ffmpeg,
            model_path: st.model_path.clone(),
            cam,
        }
    };
    tokio::task::spawn_blocking(move || run_camera(state, job));
}

fn update_cam(cam: &Arc<Mutex<CamState>>, f: impl FnOnce(&mut CamState)) {
    if let Ok(mut c) = cam.lock() {
        f(&mut c);
    }
}

fn broadcast(state: &AppState, ev: Candidate) {
    let msg = EventCandidate {
        camera_id: ev.camera_id,
        zone_id: ev.zone_id,
        behavior: ev.behavior,
        level: ev.level as i32,
        track_id: ev.track_id,
        detail: ev.detail,
        snapshot_jpeg: Vec::new(), // T-325：检测层接入后由解码器输出 JPEG
        ts_iso: ev.ts_iso,
        evidence_key: ev.evidence_key,
    };
    // 先克隆订阅者列表再发送：绝不在持 state 锁时发送（慢订阅者满队列会卡住
    // 整个控制面，stop_camera 拿不到锁 → 死锁）。满队列按 WS 同口径丢弃最旧之外
    // 的事件（try_send 非阻塞；事件消费方是 Python 入库链，不允许反压数据面）。
    let txs: Vec<_> = lock(state).event_txs.clone();
    for tx in &txs {
        let _ = tx.try_send(Ok(msg.clone()));
    }
}

/// 数据面主循环：取流 → 检测 → 管线 → 事件广播；断流指数退避重连。
fn run_camera(state: AppState, job: CameraJob) {
    let enter = env_u32("LS_ZONE_ENTER_FRAMES", 2);
    let exit = env_u32("LS_ZONE_EXIT_FRAMES", 3);
    let mut pipe = CameraPipeline::new(
        job.spec.clone(),
        job.zones.clone(),
        job.rules.clone(),
        job.calendar.clone(),
        enter,
        exit,
    );
    let mut detector: Box<dyn Detector> = make_detector(job.model_path.as_ref());
    let mut ing = Ingest::new(IngestConfig {
        source: job.spec.source.clone(),
        analysis_fps: job.spec.analysis_fps,
        width: 1280,
        height: 720,
        file_loop: job.spec.file_loop,
        rtsp_transport_tcp: true,
    });
    let mut failures = 0u32;
    let mut window_frames = 0u32;
    let mut window_start = Instant::now();
    'outer: loop {
        if job.stop.load(Ordering::SeqCst) {
            break;
        }
        if let Err(e) = ing.spawn_with_slot(Arc::clone(&job.ffmpeg)) {
            failures += 1;
            update_cam(&job.cam, |c| {
                c.last_error = format!("spawn: {e}");
                c.reconnect_count += 1;
            });
            if !sleep_interruptible(&job.stop, Ingest::backoff(failures)) {
                break;
            }
            continue;
        }
        loop {
            if job.stop.load(Ordering::SeqCst) {
                break 'outer;
            }
            match ing.read_frame() {
                Ok(Some(frame)) => {
                    failures = 0;
                    let dets = detector.detect(&frame);
                    let ts = pipeline::frame_ts_for(frame.index, job.spec.analysis_fps);
                    for ev in pipe.process(&dets, &ts) {
                        broadcast(&state, ev);
                    }
                    window_frames += 1;
                    let elapsed = window_start.elapsed();
                    update_cam(&job.cam, |c| {
                        c.last_frame_at = Some(Instant::now());
                        c.frame_age_s = 0.0;
                        c.queue_depth = 0;
                        if elapsed >= Duration::from_secs(1) {
                            c.fps_now = window_frames as f32 / elapsed.as_secs_f32();
                        }
                        if c.state != "running" {
                            c.state = "running".into();
                        }
                    });
                    if elapsed >= Duration::from_secs(1) {
                        window_frames = 0;
                        window_start = Instant::now();
                    }
                }
                Ok(None) => break, // 流结束（文件源 EOF）
                Err(e) => {
                    update_cam(&job.cam, |c| c.last_error = format!("read: {e}"));
                    break;
                }
            }
        }
        ing.stop();
        if job.stop.load(Ordering::SeqCst) {
            break;
        }
        if job.spec.type_ == "file" && !job.spec.file_loop {
            update_cam(&job.cam, |c| c.state = "stopped".into());
            return;
        }
        failures += 1;
        update_cam(&job.cam, |c| c.reconnect_count += 1);
        if !sleep_interruptible(&job.stop, Ingest::backoff(failures)) {
            break;
        }
    }
    ing.stop();
    update_cam(&job.cam, |c| c.state = "stopped".into());
}

/// 可中断睡眠（停止标志每 200ms 检查）。
fn sleep_interruptible(stop: &Arc<AtomicBool>, d: Duration) -> bool {
    let deadline = Instant::now() + d;
    while Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    !stop.load(Ordering::SeqCst)
}

#[tonic::async_trait]
impl Analyzer for AnalyzerImpl {
    async fn apply_config(&self, request: Request<Config>) -> Result<Response<Ack>, Status> {
        self.check_auth(&request)?;
        let cfg = request.into_inner();
        let cameras: HashMap<i64, CameraSpec> = cfg
            .cameras
            .iter()
            .map(|c| {
                (
                    c.id,
                    CameraSpec {
                        id: c.id,
                        name: c.name.clone(),
                        source: c.source.clone(),
                        type_: c.r#type.clone(),
                        analysis_fps: if c.analysis_fps > 0.0 {
                            c.analysis_fps
                        } else {
                            5.0
                        },
                        file_loop: c.file_loop,
                    },
                )
            })
            .collect();
        let zones: Vec<ZoneSpec> = cfg
            .zones
            .iter()
            .map(|z| ZoneSpec {
                id: z.id,
                camera_id: z.camera_id,
                name: z.name.clone(),
                points: z.points.iter().map(|p| (p.x, p.y)).collect(),
            })
            .filter(|z| z.points.len() >= 3)
            .collect();
        let rules: Vec<RuleSpec> = cfg
            .rules
            .iter()
            .map(|r| RuleSpec {
                id: r.id,
                camera_id: r.camera_id,
                zone_id: r.zone_id,
                behavior: r.behavior.clone(),
                level: r.level as i64,
                cooldown_s: r.cooldown_s as i64,
                time_start: r.time_start.clone(),
                time_end: r.time_end.clone(),
                weekdays: r.weekdays.clone(),
                threshold: r.threshold as i64,
                enabled: r.enabled,
                holiday_only: false,
            })
            .collect();
        let calendar = cfg.calendar.map(|c| c.exceptions).unwrap_or_default();
        let running: Vec<i64> = {
            let mut st = lock(&self.state);
            st.config_version = cfg.version;
            st.cameras_cfg = cameras;
            st.zones_cfg = zones;
            st.rules_cfg = rules;
            st.calendar = calendar;
            st.running.keys().copied().collect()
        };
        // 运行中的摄像头按新快照重启（旧线程置停止标志后由 spawn_camera 替换）
        for cid in running {
            spawn_camera(self.state.clone(), cid);
        }
        Ok(Response::new(Ack {
            ok: true,
            message: format!("version={}", cfg.version),
        }))
    }

    async fn start_camera(&self, request: Request<CameraRef>) -> Result<Response<Ack>, Status> {
        self.check_auth(&request)?;
        let cid = request.into_inner().camera_id;
        spawn_camera(self.state.clone(), cid);
        Ok(Response::new(Ack {
            ok: true,
            message: String::new(),
        }))
    }

    async fn stop_camera(&self, request: Request<CameraRef>) -> Result<Response<Ack>, Status> {
        self.check_auth(&request)?;
        let cid = request.into_inner().camera_id;
        let mut st = lock(&self.state);
        if let Some(flag) = st.running.remove(&cid) {
            flag.store(true, Ordering::SeqCst);
        }
        // 立即强杀 ffmpeg：数据面线程若正阻塞在管道 read 上，会收到 EOF 后退出
        if let Some(slot) = st.ffmpeg.remove(&cid) {
            Ingest::kill_slot(&slot);
        }
        if let Some(cam) = st.cams.get(&cid) {
            update_cam(cam, |c| c.state = "stopped".into());
        }
        Ok(Response::new(Ack {
            ok: true,
            message: String::new(),
        }))
    }

    type EventsStream = tokio_stream::wrappers::ReceiverStream<Result<EventCandidate, Status>>;

    async fn events(
        &self,
        request: Request<EventsRequest>,
    ) -> Result<Response<Self::EventsStream>, Status> {
        self.check_auth(&request)?;
        let (tx, rx) = mpsc::channel(64);
        lock(&self.state).event_txs.push(tx);
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    async fn status(&self, request: Request<Empty>) -> Result<Response<AnalyzerStatus>, Status> {
        self.check_auth(&request)?;
        let st = lock(&self.state);
        let mut cams: Vec<CameraStatus> = st
            .cams
            .iter()
            .map(|(cid, cam)| {
                let c = lock(cam);
                CameraStatus {
                    camera_id: *cid,
                    state: c.state.clone(),
                    analysis_fps_now: c.fps_now,
                    frame_age_s: c
                        .last_frame_at
                        .map(|t| t.elapsed().as_secs_f32())
                        .unwrap_or(0.0),
                    reconnect_count: c.reconnect_count,
                    last_error: c.last_error.clone(),
                    infer_ms_p50: c.infer_ms_p50,
                    queue_depth: c.queue_depth,
                }
            })
            .collect();
        cams.sort_by_key(|c| c.camera_id);
        Ok(Response::new(AnalyzerStatus { cameras: cams }))
    }

    async fn latest_frame(&self, request: Request<CameraRef>) -> Result<Response<Jpeg>, Status> {
        self.check_auth(&request)?;
        let _cid = request.into_inner().camera_id;
        // T-325：解码层提供 JPEG 编码后填充
        Ok(Response::new(Jpeg { data: vec![] }))
    }

    async fn finalize_evidence(
        &self,
        request: Request<EvidenceRequest>,
    ) -> Result<Response<EvidenceResult>, Status> {
        self.check_auth(&request)?;
        let _req = request.into_inner();
        // T-326 存证 remux 填充；当前返回 not-implemented 语义（ok=false）
        Ok(Response::new(EvidenceResult {
            ok: false,
            video_relpath: String::new(),
            message: "evidence remux not implemented until T-326".to_string(),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // T-321（L1）：黄金语料离线运行器（--golden <video> --out <jsonl>，独立于 gRPC 服务）
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--golden") {
        let mut fps = 5.0f32;
        let mut model = None;
        let mut no_model = false;
        let mut dump_dets = None;
        let mut it = std::env::args().skip(1);
        let mut video = None;
        let mut out = None;
        while let Some(a) = it.next() {
            match a.as_str() {
                "--video" => {
                    video = Some(std::path::PathBuf::from(it.next().ok_or("--video 缺值")?))
                }
                "--out" => out = Some(std::path::PathBuf::from(it.next().ok_or("--out 缺值")?)),
                "--fps" => fps = it.next().ok_or("--fps 缺值")?.parse()?,
                "--model" => {
                    model = Some(std::path::PathBuf::from(it.next().ok_or("--model 缺值")?))
                }
                "--no-model" => no_model = true,
                "--dump-dets" => {
                    dump_dets = Some(std::path::PathBuf::from(
                        it.next().ok_or("--dump-dets 缺值")?,
                    ))
                }
                "--golden" => {}
                other if video.is_none() && !other.starts_with('-') => {
                    video = Some(std::path::PathBuf::from(other)) // 位置参数 = 视频路径
                }
                other => {
                    return Err(format!(
                        "未知参数 {other}（--golden 模式：--video/--out/--fps/--model/--no-model）"
                    )
                    .into())
                }
            }
        }
        let args = golden::GoldenArgs {
            video: video.ok_or("--golden 需要 --video")?,
            out: out.ok_or("--golden 需要 --out")?,
            fps,
            model,
            no_model,
            dump_dets,
        };
        golden::run_golden(&args)?;
        return Ok(());
    }
    let mut port: u16 = 59001;
    let mut token = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().ok_or("--port 缺值")?.parse()?,
            "--token" => token = args.next().ok_or("--token 缺值")?,
            other => return Err(format!("未知参数 {other}（--port/--token）").into()),
        }
    }
    if token.is_empty() {
        return Err("必须提供 --token（一次性认证令牌，由父进程生成）".into());
    }
    let addr = format!("127.0.0.1:{port}").parse()?;
    let model = resolve_model_path();
    let svc = AnalyzerServer::new(AnalyzerImpl::new(token, model));
    println!("READY port={port}");
    Server::builder().add_service(svc).serve(addr).await?;
    Ok(())
}

/// 启动时解析一次模型路径（LS_MODEL_FP32 优先，缺省尝试仓库布局；None=空检测）。
fn resolve_model_path() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("LS_MODEL_FP32") {
        if !p.trim().is_empty() {
            candidates.push(PathBuf::from(p.trim()));
        }
    }
    candidates.push(PathBuf::from("m0-spike/yolo11n.onnx"));
    candidates.push(PathBuf::from("../m0-spike/yolo11n.onnx"));
    candidates.push(PathBuf::from("../../m0-spike/yolo11n.onnx"));
    candidates.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_proto::analyzer::analyzer_client::AnalyzerClient;
    use ls_proto::analyzer::{CameraConfig, Config};

    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_ok()
    }

    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    fn token_req<T>(msg: T) -> Request<T> {
        let mut r = Request::new(msg);
        r.metadata_mut()
            .insert("x-ls-token", "test-token".parse().unwrap());
        r
    }

    /// 端到端：配置快照 → StartCamera 取流 → Status 反映 fps/运行态（需本机 ffmpeg）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn grpc_data_plane_status_reports_fps() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg 不可用");
            return;
        }
        let dir = std::env::temp_dir().join(format!("ls-analyzer-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("t.mp4");
        let st = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=3:size=320x240:rate=10",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&video)
            .output()
            .unwrap();
        assert!(st.status.success());

        let port = free_port();
        // fps 测试只验证取流/状态机：显式空检测，避免与真实检测测试抢 CPU
        let svc = AnalyzerServer::new(AnalyzerImpl::new("test-token".into(), None));
        let server = tokio::spawn(async move {
            Server::builder()
                .add_service(svc)
                .serve(format!("127.0.0.1:{port}").parse().unwrap())
                .await
                .unwrap();
        });
        let mut client = loop {
            match AnalyzerClient::connect(format!("http://127.0.0.1:{port}")).await {
                Ok(c) => break c,
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        };
        let cfg = Config {
            version: 1,
            cameras: vec![CameraConfig {
                id: 7,
                name: "t".into(),
                r#type: "file".into(),
                source: video.to_string_lossy().to_string(),
                analysis_fps: 10.0,
                file_loop: true,
            }],
            zones: vec![],
            rules: vec![],
            calendar: None,
        };
        client
            .apply_config(token_req(cfg))
            .await
            .expect("apply_config");
        client
            .start_camera(token_req(CameraRef { camera_id: 7 }))
            .await
            .expect("start_camera");
        tokio::time::sleep(Duration::from_millis(1600)).await;
        let status = client
            .status(token_req(Empty {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status.cameras.len(), 1);
        let cam = &status.cameras[0];
        assert_eq!(cam.camera_id, 7);
        assert_eq!(cam.state, "running");
        assert!(
            cam.analysis_fps_now > 0.0,
            "fps 应随时间窗更新，实得 {}",
            cam.analysis_fps_now
        );
        client
            .stop_camera(token_req(CameraRef { camera_id: 7 }))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let after = client
            .status(token_req(Empty {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(after.cameras[0].state, "stopped");
        server.abort();
        let _ = std::fs::remove_file(&video);
    }

    /// 真实检测闭环（T-325/T-331）：lab-walk 语料 → YOLO(tract) → ByteTrack → 区域规则
    /// → gRPC 事件流至少一条 enter。依赖本机 ffmpeg + 仓库模型 + 演示视频，缺一 skip。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn grpc_real_detection_emits_enter_event() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg 不可用");
            return;
        }
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let model = repo.join("m0-spike").join("yolo11n.onnx");
        let demo = repo
            .join("Resources")
            .join("效果演示")
            .join("lab-walk-检测效果_v2.mp4");
        if !model.is_file() || !demo.is_file() {
            eprintln!("skip: 模型或演示视频不存在");
            return;
        }
        // 截取前 6 秒（分析 2fps → 12 帧，debug 构建下推理耗时可接受）
        let dir = std::env::temp_dir().join(format!("ls-det-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("walk.mp4");
        let st = std::process::Command::new("ffmpeg")
            .args(["-y", "-t", "6", "-i"])
            .arg(&demo)
            .args(["-an", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .output()
            .unwrap();
        assert!(st.status.success(), "截取演示视频失败");

        let port = free_port();
        let svc = AnalyzerServer::new(AnalyzerImpl::new("test-token".into(), Some(model.clone())));
        let server = tokio::spawn(async move {
            Server::builder()
                .add_service(svc)
                .serve(format!("127.0.0.1:{port}").parse().unwrap())
                .await
                .unwrap();
        });
        let mut client = loop {
            match AnalyzerClient::connect(format!("http://127.0.0.1:{port}")).await {
                Ok(c) => break c,
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        };
        let mut events = client
            .events(token_req(EventsRequest {}))
            .await
            .unwrap()
            .into_inner();
        let cfg = Config {
            version: 1,
            cameras: vec![CameraConfig {
                id: 9,
                name: "walk".into(),
                r#type: "file".into(),
                source: clip.to_string_lossy().to_string(),
                analysis_fps: 2.0,
                file_loop: false,
            }],
            zones: vec![ls_proto::analyzer::ZoneConfig {
                id: 1,
                camera_id: 9,
                name: "A 区".into(),
                points: vec![
                    ls_proto::analyzer::Point { x: 0, y: 0 },
                    ls_proto::analyzer::Point { x: 1280, y: 0 },
                    ls_proto::analyzer::Point { x: 1280, y: 720 },
                    ls_proto::analyzer::Point { x: 0, y: 720 },
                ],
            }],
            rules: vec![ls_proto::analyzer::RuleConfig {
                id: 1,
                camera_id: 9,
                zone_id: 1,
                behavior: "enter".into(),
                level: 2,
                cooldown_s: 0,
                time_start: "00:00".into(),
                time_end: "23:59".into(),
                weekdays: String::new(),
                threshold: 0,
                enabled: true,
            }],
            calendar: None,
        };
        client.apply_config(token_req(cfg)).await.unwrap();
        client
            .start_camera(token_req(CameraRef { camera_id: 9 }))
            .await
            .unwrap();
        let mut got = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(10), events.message()).await {
                Ok(Ok(Some(ev))) => {
                    if ev.behavior == "enter" || ev.behavior == "enter_no_ppe" {
                        got = Some(ev);
                        break;
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => panic!("事件流错误: {e}"),
                Err(_) => {}
            }
        }
        let ev = got.expect("真实检测应产生进入事件");
        assert_eq!(ev.camera_id, 9);
        assert!(ev.track_id > 0);
        assert!(!ev.ts_iso.is_empty());
        assert!(ev.zone_id == Some(1));
        client
            .stop_camera(token_req(CameraRef { camera_id: 9 }))
            .await
            .unwrap();
        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L1 轨迹对齐（回放诊断，测试专用入口）：消费 export_real_track_vector.py 的
    /// 真值向量（检测+轨迹），pipeline 回放后逐帧 diff（id 精确 + 框容差 0.5px）。
    /// 语料/向量缺失自动 skip。分歧帧数随 ls-track 语义修复持续下降。
    /// L1 对齐收敛（>=98%）后移除 ignore 作为回归门禁。
    #[test]
    #[ignore = "L1 轨迹对齐收敛中（diff_frames>0），达标后转正为回归门禁"]
    fn replay_real_track_vector() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let vector = repo.join("testdata/vectors/real-big.json");
        if !vector.is_file() {
            eprintln!("skip: 真值向量不存在（先跑 tools/export_real_track_vector.py）");
            return;
        }
        let vec_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&vector).unwrap()).unwrap();
        let frames = vec_json["frames"].as_array().unwrap();

        let camera = pipeline::CameraSpec {
            id: 1,
            name: "vec".into(),
            source: String::new(),
            type_: "file".into(),
            analysis_fps: 5.0,
            file_loop: false,
        };
        let zones = vec![pipeline::ZoneSpec {
            id: 1,
            camera_id: 1,
            name: "全画幅".into(),
            points: vec![(0, 0), (1280, 0), (1280, 719), (0, 719)],
        }];
        let rules = ["enter", "exit", "enter_no_ppe"]
            .iter()
            .enumerate()
            .map(|(i, b)| pipeline::RuleSpec {
                id: i as i64 + 1,
                camera_id: 1,
                zone_id: 1,
                behavior: b.to_string(),
                level: 2,
                cooldown_s: 0,
                time_start: "00:00".into(),
                time_end: "23:59".into(),
                weekdays: String::new(),
                threshold: 0,
                enabled: true,
                holiday_only: false,
            })
            .collect();
        let mut pipe =
            pipeline::CameraPipeline::new(camera, zones, rules, Default::default(), 2, 3);
        pipe.log_tracks = true;

        for fr in frames {
            let dets: Vec<pipeline::PersonDetection> = fr["dets"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|row| {
                    let r = row.as_array()?;
                    if r.len() < 5 {
                        return None;
                    }
                    Some(pipeline::PersonDetection {
                        box_: [
                            r[0].as_f64()? as f32,
                            r[1].as_f64()? as f32,
                            r[2].as_f64()? as f32,
                            r[3].as_f64()? as f32,
                        ],
                        score: r[4].as_f64()? as f32,
                        ppe: pipeline::PpeStatus::Unknown,
                    })
                })
                .collect();
            let fnum = fr["frame"].as_u64().unwrap_or(1);
            let ts = pipeline::frame_ts_for(fnum, 5.0);
            let _ = pipe.process(&dets, &ts);
        }

        // 逐帧 diff（id 精确 + 框容差 0.5px）
        let mut diff_frames = 0usize;
        let mut first: Option<(usize, Vec<f64>, Vec<f64>)> = None;
        for (i, tracks) in pipe.track_log.iter().enumerate() {
            let py_rows: Vec<(f64, [f64; 4])> = frames[i]["tracks"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|r| {
                            let a = r.as_array().unwrap();
                            (
                                a[4].as_f64().unwrap(),
                                [
                                    a[0].as_f64().unwrap(),
                                    a[1].as_f64().unwrap(),
                                    a[2].as_f64().unwrap(),
                                    a[3].as_f64().unwrap(),
                                ],
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut py_left: Vec<(f64, [f64; 4])> = py_rows.clone();
            let mut rs_left: Vec<(f64, [f64; 4])> = tracks
                .iter()
                .map(|(id, b)| {
                    (
                        *id as f64,
                        [b[0] as f64, b[1] as f64, b[2] as f64, b[3] as f64],
                    )
                })
                .collect();
            for (py_id, py_box) in py_rows.iter() {
                let hit = rs_left.iter().position(|(rs_id, rs_box)| {
                    rs_id == py_id
                        && rs_box
                            .iter()
                            .zip(py_box.iter())
                            .all(|(r, p)| (r - p).abs() <= 0.5)
                });
                if let Some(pos) = hit {
                    rs_left.remove(pos);
                } else {
                    py_left.retain(|(id, _)| id != py_id);
                }
            }
            if !py_left.is_empty() || !rs_left.is_empty() {
                diff_frames += 1;
                if first.is_none() {
                    let flat = |v: &[(f64, [f64; 4])]| -> Vec<f64> {
                        v.iter()
                            .flat_map(|(id, b)| [*id, b[0], b[1], b[2], b[3]])
                            .collect()
                    };
                    first = Some((i + 1, flat(&py_left), flat(&rs_left)));
                }
            }
        }
        println!(
            "[replay] frames={} diff_frames={diff_frames}",
            pipe.track_log.len()
        );
        if let Some((f, py, rs)) = first {
            panic!("轨迹分歧 first frame={f}: PY-only={py:?} RS-only={rs:?}（按 supervision 语义继续收敛）");
        }
    }
}
