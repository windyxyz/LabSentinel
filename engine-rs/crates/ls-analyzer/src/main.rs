//! ls-analyzer sidecar（T-327/T-331）：监听 127.0.0.1:<port>，一次性认证令牌经
//! LS_ANALYZER_TOKEN 环境变量传入（R2-109；--token 保留一版兼容），防本机其它进程未授权调用。
//!
//! 数据面（T-331/T-325）：ApplyConfig 下发全量快照 → StartCamera 启动 ffmpeg 取流
//! （ls-ingest）→ YOLO 检测（tract 纯 Rust；模型缺失时回退 NullDetector）→ ByteTrack
//! → 区域迟滞/规则冷却（pipeline）→ EventCandidate 经 gRPC 事件流推送；
//! Status 返回实时 fps/帧龄/重连/错误。
//! 启动即打印 "READY port=<p>" 行，供父进程健康判定。
#![allow(clippy::result_large_err)]

mod golden;

// RA1.0（ADR-0011）：pipeline/evidence 拆入 lib 目标（src/lib.rs），bin 经
// `use` 引用——进程内复用（ls-server worker）与 sidecar 共用同一份实现。
use ls_analyzer::{detector, evidence, pipeline, ppe};

use std::collections::HashMap;
use std::path::PathBuf;
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

use pipeline::{CameraPipeline, CameraSpec, Candidate, Detector, RuleSpec, ZoneSpec};

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
    /// T-326：每路证据环形缓冲/活动录像（数据面写、FinalizeEvidence 处理器读）
    evidence: HashMap<i64, Arc<evidence::Evidence>>,
    /// 模型路径（空串=不启用检测；按实例注入，避免进程级环境变量竞态）
    model_path: Option<PathBuf>,
    /// 每个事件流订阅者的发送端（数据面线程 try_send 非阻塞广播）
    event_txs: Vec<mpsc::Sender<Result<EventCandidate, Status>>>,
    /// R2-305：已解密的告警渠道快照（ApplyConfig 下发；数据面本地派发/escalation
    /// 消费。明文只进内存，绝不落盘/日志）
    alert_channels: Vec<ls_dispatch::ChannelConfig>,
    /// R2-305：渠道名（与 alert_channels 同序对应）
    alert_channel_names: Vec<String>,
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
    /// T-326：存证状态（环 + 活动录像）
    evidence: Option<Arc<evidence::Evidence>>,
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
                evidence: HashMap::new(),
                model_path,
                event_txs: vec![],
                alert_channels: Vec::new(),
                alert_channel_names: Vec::new(),
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

/// YOLO 检测器已迁入 lib（RA1.2 参数化：`detector::{YoloDetector, DetectorConfig}`）；
/// bin 侧仅保留 env 组装包装，语义原样。
/// T-325/Q-14：PPE 级联检测器（YoloModel + 类名→索引映射）。
///
/// 与 Python `vision.ppe_boxes` 同口径：违规类一次整图推理（conf=LS_PPE_CONF，
/// 默认 0.30）、穿戴类逐个一次（conf=LS_PPE_WEARING_CONF，默认 0.20）；
/// 单人判定=人框与违规框有交集 → NoPpe；否则与穿戴框有交集 → Wearing；皆无 → Unknown。
/// PPE 级联检测器已迁入 lib（RA2-S1 参数化：`ppe::{YoloPpeRunner, PpeConfig}`）；
/// bin 侧仅保留 env 组装包装（sidecar 进程协议不变）。
fn make_ppe_runner() -> Option<Box<dyn pipeline::PpeRunner>> {
    let raw = std::env::var("LS_PPE_MODEL").ok().map(PathBuf::from)?;
    let Some(path) = detector::anchor_model_file(&raw) else {
        println!(
            "[ls-analyzer] PPE 模型路径无效/不可读（仅人员检测）: {}",
            raw.display()
        );
        return None;
    };
    let cfg = ppe::PpeConfig {
        model_path: path,
        imgsz: env_u32("LS_IMGSZ", 640),
        iou: env_f32("LS_IOU_THRES", 0.50),
        violation_conf: env_f32("LS_PPE_CONF", 0.30),
        wearing_conf: env_f32("LS_PPE_WEARING_CONF", 0.20),
        violation_class_name: std::env::var("LS_PPE_VIOLATION_CLASS")
            .unwrap_or_else(|_| "no lab coat".to_string()),
        wearing_class_names: std::env::var("LS_PPE_CLASS_NAMES")
            .unwrap_or_else(|_| "lab coat".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    };
    ppe::make_ppe_runner(&cfg)
}

/// 构造检测器：模型路径由 AnalyzerImpl 实例注入（生产从 LS_MODEL_FP32 / 仓库布局
/// 启动时解析一次；测试可各自指定或设 None，避免进程级 set_var 并发竞态）。
/// RA1.2：模型构造参数化迁入 lib（`detector::make_detector`）；bin 侧仅保留
/// env → DetectorConfig 的组装包装（sidecar 进程协议不变：LS_IMGSZ 等环境变量照旧）。
fn make_detector(path: Option<&PathBuf>) -> Box<dyn Detector> {
    let cfg = detector::DetectorConfig::from_env(path.cloned());
    detector::make_detector(&cfg)
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
        // T-326：每路证据状态（in_fps=分析帧率——apply_config 已强制 >0）
        let ev = Arc::new(evidence::Evidence::default());
        ev.new_ring(
            spec.analysis_fps.max(0.1) as f64,
            env_f32("LS_EVIDENCE_FPS", 10.0) as f64,
        );
        st.evidence.insert(cid, Arc::clone(&ev));
        CameraJob {
            spec,
            zones,
            rules,
            calendar: st.calendar.clone(),
            stop,
            ffmpeg,
            evidence: Some(ev),
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

fn broadcast(state: &AppState, ev: Candidate, snapshot_jpeg: Vec<u8>) {
    let msg = EventCandidate {
        camera_id: ev.camera_id,
        zone_id: ev.zone_id,
        behavior: ev.behavior,
        level: ev.level as i32,
        track_id: ev.track_id,
        detail: ev.detail,
        snapshot_jpeg,
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
    // T-325/Q-14：PPE 级联（模型未配置/未下载时 None → 仅人员检测，ppe 恒 Unknown）
    pipe.set_ppe(make_ppe_runner());
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
                    // T-326：先喂证据环（与 Python `_feed_recordings` 先于 `_analyze` 同序，
                    // 保证事件帧已在环内），事件帧再编码快照
                    let ev_snapshot: Option<Vec<u8>> = job.evidence.as_ref().and_then(|ev| {
                        ev.feed(frame.index, || {
                            evidence::encode_jpeg_bgr(&frame.data, frame.width, frame.height)
                                .unwrap_or_default()
                        })
                    });
                    let dets = detector.detect(&frame);
                    let ts = pipeline::frame_ts_for(frame.index, job.spec.analysis_fps);
                    // T-325：带原始帧处理——PPE 启用时在疑似进场帧整帧推理一次
                    let cands = pipe.process_with_frame(
                        &dets,
                        &ts,
                        Some((&frame.data, frame.width, frame.height)),
                    );
                    if !cands.is_empty() {
                        // 事件帧快照：优先复用本帧抽帧结果，未命中 stride 则直接现编
                        //（不再走 feed——stride 计数每帧只允许推进一次）
                        let snap = match ev_snapshot {
                            Some(j) if !j.is_empty() => j,
                            _ => evidence::encode_jpeg_bgr(&frame.data, frame.width, frame.height)
                                .unwrap_or_default(),
                        };
                        for ev in cands {
                            broadcast(&state, ev, snap.clone());
                        }
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
            if let Some(ev) = &job.evidence {
                ev.flush(); // T-326：未补齐录像置 done（部分帧落盘）
            }
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
    if let Some(ev) = &job.evidence {
        ev.flush(); // T-326：未补齐录像置 done（部分帧落盘）
    }
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
        // R2-305：渠道快照（已解密明文只进内存；供数据面本地派发/escalation 消费）
        let (alert_channels, alert_channel_names) = match cfg.alert_channels {
            Some(ac) => {
                let mut cfgs = Vec::new();
                let mut names = Vec::new();
                for ch in &ac.channels {
                    names.push(ch.channel.clone());
                    let mut cc = ls_dispatch::ChannelConfig {
                        url: ch.url.clone(),
                        secret: ch.secret.clone(),
                        token: ch.token.clone(),
                        key: ch.key.clone(),
                        allow_http: ch.allow_http,
                        qq: ch.qq.clone(),
                        ..Default::default()
                    };
                    if let Some(em) = &ch.email {
                        cc.email = ls_dispatch::EmailConfig {
                            host: em.host.clone(),
                            port: em.port.max(0) as u16,
                            user: em.user.clone(),
                            pass: em.password.clone(),
                            to: em.to.clone(),
                            security: em.security.clone(),
                        };
                    }
                    cfgs.push(cc);
                }
                (cfgs, names)
            }
            None => (Vec::new(), Vec::new()),
        };
        let running: Vec<i64> = {
            let mut st = lock(&self.state);
            st.config_version = cfg.version;
            st.cameras_cfg = cameras;
            st.zones_cfg = zones;
            st.rules_cfg = rules;
            st.calendar = calendar;
            if !alert_channel_names.is_empty() {
                // 渠道快照存在时整体替换；空=保持既有（避免未启用网页配置时清空）
                st.alert_channels = alert_channels;
                st.alert_channel_names = alert_channel_names.clone();
            }
            st.running.keys().copied().collect()
        };
        if !alert_channel_names.is_empty() {
            println!(
                "[ls-analyzer] ApplyConfig alert_channels={:?}",
                alert_channel_names
            );
        }
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
        // T-326：未补齐的录像立即置 done（在途 RPC 写部分帧落盘），证据条目释放
        if let Some(ev) = st.evidence.remove(&cid) {
            ev.flush();
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
        let cid = request.into_inner().camera_id;
        // T-326：证据环最新帧即实时快照（evidence_fps 抽帧粒度）
        let jpeg = {
            let st = lock(&self.state);
            st.evidence
                .get(&cid)
                .and_then(|ev| {
                    ev.ring
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                        .and_then(|ring| ring.latest().map(|f| f.data.clone()))
                })
                .unwrap_or_default()
        };
        Ok(Response::new(Jpeg { data: jpeg }))
    }

    async fn finalize_evidence(
        &self,
        request: Request<EvidenceRequest>,
    ) -> Result<Response<EvidenceResult>, Status> {
        self.check_auth(&request)?;
        let req = request.into_inner();
        let post_s = if req.post_seconds > 0 {
            req.post_seconds as f64
        } else {
            evidence::VIDEO_POST_S
        };
        // 每路证据状态（stop_camera 移除条目但在途 Arc 仍有效——部分帧落盘语义）
        let ev = {
            let st = lock(&self.state);
            st.evidence.get(&req.camera_id).cloned()
        };
        let Some(ev) = ev else {
            return Ok(Response::new(EvidenceResult {
                ok: false,
                video_relpath: String::new(),
                message: "camera not running（无证据环）".into(),
            }));
        };
        // 并发上限（与 Python MAX_CONCURRENT_RECORDINGS 一致，超限拒收）
        {
            let recs = ev.recs.lock().unwrap_or_else(|e| e.into_inner());
            let live = recs
                .iter()
                .filter(|r| !r.done.load(Ordering::SeqCst))
                .count();
            if live >= evidence::MAX_CONCURRENT_RECORDINGS {
                return Ok(Response::new(EvidenceResult {
                    ok: false,
                    video_relpath: String::new(),
                    message: "recording limit reached".into(),
                }));
            }
        }
        // pre 帧 = 环快照（事件帧应已在队尾）；登记活动录像等数据面补 post 帧
        let rec = {
            let pre: Vec<Vec<u8>> = ev
                .ring
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|ring| ring.snapshot().into_iter().map(|f| f.data).collect())
                .unwrap_or_default();
            let rec = Arc::new(evidence::ActiveRec::new(pre));
            ev.recs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Arc::clone(&rec));
            rec
        };
        // 等待补齐（超时 = post + 3s；到点按已有帧落盘，有比没有强）
        let deadline = Instant::now() + Duration::from_secs_f64(post_s + 3.0);
        while !rec.done.load(Ordering::SeqCst) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let frames: Vec<Vec<u8>> =
            std::mem::take(&mut *rec.frames.lock().unwrap_or_else(|e| e.into_inner()));
        let elapsed = rec.elapsed().as_secs_f64();
        // 回收已完成条目
        ev.recs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|r| !Arc::ptr_eq(r, &rec));
        if frames.is_empty() {
            return Ok(Response::new(EvidenceResult {
                ok: false,
                video_relpath: String::new(),
                message: "no evidence frames".into(),
            }));
        }
        // 写盘帧率 = 实测喂帧率（帧数/实际耗时）：推理/取流低于配置帧率时保证
        // 播放速度正确（debug 构建/CPU 高负载下实际 ~1fps，用配置值会加速播放）。
        let cfg_fps = ev
            .ring
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|r| r.out_fps())
            .unwrap_or(5.0);
        let fps = if elapsed > 0.2 {
            (frames.len() as f64 / elapsed).clamp(1.0, cfg_fps * 1.5)
        } else {
            cfg_fps
        };
        // 写盘目录：LS_VIDEO_DIR（Python 子进程环境注入）；relpath 相对 LS_DATA_DIR
        let Some(video_dir) = std::env::var("LS_VIDEO_DIR").ok().filter(|s| !s.is_empty()) else {
            return Ok(Response::new(EvidenceResult {
                ok: false,
                video_relpath: String::new(),
                message: "LS_VIDEO_DIR not set".into(),
            }));
        };
        let video_dir = PathBuf::from(video_dir);
        std::fs::create_dir_all(&video_dir).map_err(|e| Status::internal(e.to_string()))?;
        // 安全复核（Mimosa medium「疑似跨文件污点」）：文件名由 event_id 经 format!
        // 固定模板生成（ev_<id>.mp4），不存在路径穿越面；相对路径转换
        // （evidence::video_relpath）只做前缀裁剪，不引入外部可控段。Python 侧
        // 打包时另有 _within_data_dir 防穿越校验兜底。
        let file = format!("ev_{}.mp4", req.event_id);
        let out = video_dir.join(&file);
        let frames_len = frames.len();
        let out2 = out.clone();
        let enc = tokio::task::spawn_blocking(move || evidence::encode_mp4(&frames, fps, &out2))
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        match enc {
            Ok(()) => {
                let relpath = evidence::video_relpath(
                    &video_dir,
                    std::env::var("LS_DATA_DIR")
                        .ok()
                        .map(PathBuf::from)
                        .as_deref(),
                    &file,
                );
                println!(
                    "[ls-analyzer] ev#{} 存证 {} 帧 → {}",
                    req.event_id,
                    frames_len,
                    out.display()
                );
                Ok(Response::new(EvidenceResult {
                    ok: true,
                    video_relpath: relpath,
                    message: String::new(),
                }))
            }
            Err(e) => Ok(Response::new(EvidenceResult {
                ok: false,
                video_relpath: String::new(),
                message: e,
            })),
        }
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
    // R2-109/N-16: prefer the environment variable (not visible in the process
    // command line to other users); keep --token for one-version compatibility
    // with older Python parents.
    let mut token = std::env::var("LS_ANALYZER_TOKEN").unwrap_or_default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().ok_or("--port 缺值")?.parse()?,
            "--token" => token = args.next().ok_or("--token 缺值")?,
            other => return Err(format!("未知参数 {other}（--port/--token）").into()),
        }
    }
    if token.is_empty() {
        return Err("必须提供 LS_ANALYZER_TOKEN 或 --token（一次性认证令牌，由父进程生成）".into());
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
            alert_channels: None,
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
            alert_channels: None,
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

    /// L1 轨迹对齐回归门禁：消费 export_real_track_vector.py 的真值向量（检测+轨迹
    /// +KF 状态快照），pipeline 回放后逐帧 diff（id 精确 + 框容差 0.5px）。
    /// 2026-11-07 收敛：322/322 帧逐位一致（轨迹 + KF 状态），此测试为常跑门禁——
    /// 任何 ls-track 语义/精度回归都会在此爆掉。语料/向量缺失自动 skip。
    #[test]
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
                    // 命中：py 行同步移出 py_left；未命中则留在 py_left（=真 PY-only）
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

        // 状态级诊断：逐非空帧比对 (tracked/lost 的 (external_id, mean_x))，
        // 定位 KF 内部状态的最早分歧帧（轨迹输出一致 ≠ KF 状态一致）
        let mut first_state_div: Option<(usize, String)> = None;
        for (i, snap) in pipe.state_log.iter().enumerate() {
            let Some(py_state) = frames[i]["state"].as_object() else {
                continue; // 空帧无状态 dump
            };
            let py_list = |key: &str| -> Vec<(i64, f64)> {
                py_state
                    .get(key)
                    .and_then(|v| v.as_array())
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|r| {
                                let a = r.as_array()?;
                                Some((a[0].as_i64()?, a[1].as_f64()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            // 直接逐列表对比（tracked 与 lost）
            let py_tracked = py_list("tracked");
            let py_lost = py_list("lost");
            let rs_tracked: Vec<(i64, f64)> = snap.0.clone();
            let rs_lost: Vec<(i64, f64)> = snap.1.clone();
            let diff = |py: &[(i64, f64)], rs: &[(i64, f64)], name: &str| -> Option<String> {
                for (pid, px) in py {
                    match rs.iter().find(|(rid, _)| rid == pid) {
                        Some((_, rx)) if (rx - px).abs() > 0.15 => {
                            return Some(format!("{name} id={pid} py_x={px} rs_x={rx}"));
                        }
                        Some(_) => {}
                        None => return Some(format!("{name} id={pid} py_x={px} 仅Python有")),
                    }
                }
                for (rid, rx) in rs {
                    if !py.iter().any(|(pid, _)| pid == rid) {
                        return Some(format!("{name} id={rid} rs_x={rx} 仅Rust有"));
                    }
                }
                None
            };
            if let Some(msg) = diff(&py_tracked, &rs_tracked, "tracked")
                .or_else(|| diff(&py_lost, &rs_lost, "lost"))
            {
                if first_state_div.is_none() {
                    first_state_div = Some((i + 1, msg));
                }
            }
        }
        match first_state_div {
            Some((f, msg)) => println!("[replay] KF 状态最早分歧 frame={f}: {msg}"),
            None => println!("[replay] KF 状态逐帧一致（±0.15px）"),
        }
        if std::env::var("LS_DEBUG_STATE").is_ok() {
            for (i, snap) in pipe.state_log.iter().enumerate().take(10) {
                if frames[i]["state"].is_null() {
                    continue;
                }
                let fmt = |v: &[(i64, f64)]| -> String {
                    v.iter()
                        .map(|(id, x)| format!("({id}, {x:.4})"))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                eprintln!(
                    "[state] f{} tracked=[{}] lost=[{}]",
                    i + 1,
                    fmt(&snap.0),
                    fmt(&snap.1)
                );
            }
        }

        if let Some((f, py, rs)) = first {
            panic!("轨迹分歧 first frame={f}: PY-only={py:?} RS-only={rs:?}（按 supervision 语义继续收敛）");
        }
    }

    #[tokio::test]
    async fn alert_channels_snapshot_parsed() {
        // R2-305：ApplyConfig 渠道快照解析进数据面状态（明文只进内存）
        let imp = AnalyzerImpl::new("tok".into(), None);
        let cfg = Config {
            version: 1,
            cameras: vec![],
            zones: vec![],
            rules: vec![],
            calendar: None,
            alert_channels: Some(ls_proto::analyzer::AlertChannels {
                channels: vec![
                    ls_proto::analyzer::AlertChannel {
                        channel: "wecom".into(),
                        url: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=t".into(),
                        ..Default::default()
                    },
                    ls_proto::analyzer::AlertChannel {
                        channel: "qmsg".into(),
                        key: "k1".into(),
                        qq: "10001".into(),
                        ..Default::default()
                    },
                ],
            }),
        };
        let mut req = tonic::Request::new(cfg);
        req.metadata_mut()
            .insert("x-ls-token", "tok".parse().unwrap());
        imp.apply_config(req).await.expect("apply_config");
        {
            let st = lock(&imp.state);
            assert_eq!(st.alert_channel_names, vec!["wecom", "qmsg"]);
            assert_eq!(st.alert_channels.len(), 2);
            assert!(st.alert_channels[0].url.contains("qyapi.weixin.qq.com"));
            assert_eq!(st.alert_channels[1].qq, "10001");
        }
        // 空快照不覆盖既有渠道（避免未启用网页配置时清空）
        let cfg2 = Config {
            version: 2,
            cameras: vec![],
            zones: vec![],
            rules: vec![],
            calendar: None,
            alert_channels: None,
        };
        let mut req2 = tonic::Request::new(cfg2);
        req2.metadata_mut()
            .insert("x-ls-token", "tok".parse().unwrap());
        imp.apply_config(req2).await.expect("apply_config 2");
        let st = lock(&imp.state);
        assert_eq!(st.alert_channels.len(), 2, "空快照保持既有渠道");
    }
}
