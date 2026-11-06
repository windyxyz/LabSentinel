//! ls-analyzer sidecar（T-327/T-331）：监听 127.0.0.1:<port>，一次性认证令牌经启动参数传入
//! （--token <secret>，Python 父进程生成），防本机其它进程未授权调用。
//!
//! 数据面（T-331）：ApplyConfig 下发全量快照 → StartCamera 启动 ffmpeg 取流（ls-ingest）
//! → Detector（T-325 前为 NullDetector）→ ByteTrack → 区域迟滞/规则冷却（pipeline）
//! → EventCandidate 经 gRPC 事件流推送；Status 返回实时 fps/帧龄/重连/错误。
//! 启动即打印 "READY port=<p>" 行，供父进程健康判定。
#![allow(clippy::result_large_err)]

mod pipeline;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

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
    /// 每个事件流订阅者的发送端（数据面线程 blocking_send 广播）
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
    cam: Arc<Mutex<CamState>>,
}

pub struct AnalyzerImpl {
    state: AppState,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AnalyzerImpl {
    fn new(token: String) -> Self {
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

/// 启动/重启一路摄像头数据面线程（无配置时仅记录 running，兼容旧客户端）。
fn spawn_camera(state: AppState, cid: i64) {
    let job = {
        let mut st = lock(&state);
        if let Some(old) = st.running.remove(&cid) {
            old.store(true, Ordering::SeqCst);
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
        st.running.insert(cid, Arc::clone(&stop));
        CameraJob {
            spec,
            zones,
            rules,
            calendar: st.calendar.clone(),
            stop,
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
    let mut st = lock(state);
    st.event_txs
        .retain(|tx| tx.blocking_send(Ok(msg.clone())).is_ok());
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
    let mut detector: Box<dyn Detector> = Box::new(NullDetector);
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
        if let Err(e) = ing.spawn() {
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
        let st = lock(&self.state);
        if let Some(flag) = st.running.get(&cid) {
            flag.store(true, Ordering::SeqCst);
        }
        if let Some(cam) = st.cams.get(&cid) {
            update_cam(cam, |c| c.state = "stopped".into());
        }
        drop(st);
        let mut st = lock(&self.state);
        st.running.remove(&cid);
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
    let svc = AnalyzerServer::new(AnalyzerImpl::new(token));
    println!("READY port={port}");
    Server::builder().add_service(svc).serve(addr).await?;
    Ok(())
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
        let svc = AnalyzerServer::new(AnalyzerImpl::new("test-token".into()));
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
}
