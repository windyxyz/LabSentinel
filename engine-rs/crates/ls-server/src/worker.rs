//! RA1（ADR-0011）：进程内分析 worker 的事件落地层（RA1.2 首切）。
//!
//! 事件契约 1:1 对齐 Python `sidecar_events.py` 桥：
//! - 去重窗口 4096（evidence_key|behavior|track_id，流重连重放保护）；
//! - 截图 `ev_{cid}_{ts}_{seq}.jpg` 落 data/snapshots，绝对路径入 DB（与桥同口径，
//!   serve_event_media 的 `data_dir.join` 对绝对路径整体替换，两种都可播）；
//! - `add_event` → `event_json` 广播到 `events_tx`（WS；无订阅者时静默丢弃）；
//! - finalize：帧序列 → `videos/ev_{eid}.mp4`（复用 evidence::encode_mp4）→
//!   `set_event_video` 回填。
//!
//! 告警首发派发（Python dispatcher.dispatch → 8 渠道）在 RA1.3 随 worker
//! 生命周期接入（复用 ls_dispatch::deliver + escalation 既有链路）。
#![allow(dead_code)] // RA1.2 骨架先行——RA1.3 worker 生命周期接线后摘除

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::handlers::events::event_json;
use crate::state::Shared;

/// 事件去重窗口容量（Python sidecar_events.SEEEN_MAX 同值）。
const DEDUP_MAX: usize = 4096;

static SNAP_SEQ: AtomicU64 = AtomicU64::new(0);

/// worker 文件系统布局（data_dir 相对约定；测试可注入隔离目录）。
#[derive(Debug, Clone)]
pub struct SinkPaths {
    pub snapshots: PathBuf,
    pub videos: PathBuf,
}

impl SinkPaths {
    pub fn from_data_dir(root: &str) -> Self {
        let root = PathBuf::from(root);
        Self {
            snapshots: root.join("snapshots"),
            videos: root.join("videos"),
        }
    }
}

struct Dedup {
    seen: HashSet<String>,
    order: VecDeque<String>,
}

/// RA1.2 事件落地（同步接口——管线运行在 spawn_blocking 线程，与 Python 桥的
/// 线程模型一致；SQLite 每次调用独立 open，与 handlers 同模式）。
pub struct EventSink {
    st: Shared,
    paths: SinkPaths,
    dedup: Mutex<Dedup>,
}

impl EventSink {
    pub fn new(st: Shared) -> Self {
        Self {
            paths: SinkPaths::from_data_dir(&st.data_dir),
            st,
            dedup: Mutex::new(Dedup {
                seen: HashSet::new(),
                order: VecDeque::new(),
            }),
        }
    }

    #[cfg(test)]
    fn with_paths(st: Shared, paths: SinkPaths) -> Self {
        Self {
            st,
            paths,
            dedup: Mutex::new(Dedup {
                seen: HashSet::new(),
                order: VecDeque::new(),
            }),
        }
    }

    fn dedup_key(cand: &ls_analyzer::pipeline::Candidate) -> String {
        format!("{}|{}|{}", cand.evidence_key, cand.behavior, cand.track_id)
    }

    /// 候选 → 截图 → 入库 → WS 广播。返回 `Ok(Some(eid))`=已落地（供 finalize
    /// 关联），`Ok(None)`=去重命中（Python 桥静默丢弃同口径）。
    pub fn record(
        &self,
        cand: &ls_analyzer::pipeline::Candidate,
        snapshot_jpeg: Option<&[u8]>,
    ) -> Result<Option<i64>, String> {
        {
            let mut d = self.dedup.lock().map_err(|_| "dedup poison")?;
            let key = Self::dedup_key(cand);
            if d.seen.contains(&key) {
                return Ok(None);
            }
            d.seen.insert(key.clone());
            d.order.push_back(key);
            if d.order.len() > DEDUP_MAX {
                if let Some(old) = d.order.pop_front() {
                    d.seen.remove(&old);
                }
            }
        }
        let mut snapshot: Option<String> = None;
        if let Some(jpeg) = snapshot_jpeg {
            std::fs::create_dir_all(&self.paths.snapshots)
                .map_err(|e| format!("创建截图目录失败: {e}"))?;
            let seq = SNAP_SEQ.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "ev_{}_{}_{}{:04}.jpg",
                cand.camera_id,
                cand.ts_iso.replace([':', '-', '.'], ""),
                std::process::id(),
                seq
            );
            let path = self.paths.snapshots.join(name);
            std::fs::write(&path, jpeg).map_err(|e| format!("截图写入失败: {e}"))?;
            snapshot = Some(path.to_string_lossy().into_owned());
        }
        let st = &self.st;
        let store = ls_store::Store::open(&st.db_path).map_err(|e| e.to_string())?;
        let ev = ls_store::NewEvent {
            camera_id: cand.camera_id,
            type_: &cand.behavior,
            level: cand.level,
            track_id: cand.track_id,
            detail: &cand.detail,
            snapshot: snapshot.as_deref(),
            ts: &cand.ts_iso,
        };
        let eid = store.add_event(&ev).map_err(|e| e.to_string())?;
        // WS 广播：行口径与 /api/events 完全一致（绝对路径不外泄——event_json
        // 只出 has_snapshot/has_video 布尔，与 Python 桥 pop(snapshot/video) 等价）
        if let Ok(Some(row)) = store.event_by_id(eid) {
            let _ = st.events_tx.send(event_json(&row).to_string());
        }
        Ok(Some(eid))
    }

    /// 存证收尾：帧序列 → MP4 → video 回填（先入库后写视频，与 Python worker
    /// 「失败只影响存证不影响事件」同口径）。`Ok(Some(rel))`=已回填。
    pub fn finalize(
        &self,
        eid: i64,
        frames: &[Vec<u8>],
        fps: f64,
    ) -> Result<Option<String>, String> {
        std::fs::create_dir_all(&self.paths.videos)
            .map_err(|e| format!("创建视频目录失败: {e}"))?;
        let out = self.paths.videos.join(format!("ev_{eid}.mp4"));
        ls_analyzer::evidence::encode_mp4(frames, fps, &out)?;
        let abs = out.to_string_lossy().into_owned();
        let store = ls_store::Store::open(&self.st.db_path).map_err(|e| e.to_string())?;
        store
            .set_event_video(eid, &abs)
            .map_err(|e| e.to_string())?;
        Ok(Some(abs))
    }
}

// ---------------------------------------------------------------------------
// RA1.2：worker 生命周期（start/stop_camera 真实现，摘 503）
//
// 行为契约对齐 Python CameraWorker/CameraManager：
// - ffmpeg 取流失败/断流 → 退避重连（序列 1/5/30/60s，存活 >5s 重置计数），
//   超过 max_restarts → cameras.status='error'（given_up）并退出线程；
// - 文件非循环源播完 → status='stopped' 自然退出；
// - 帧循环内：先喂证据环（与 Python `_feed_recordings` 先于 `_analyze` 同序）、
//   检测 → 管线 → 候选经 EventSink 落地 + 存证收尾线程回填 video。
// 与 Python 的已知粒度差异（RA1.2 接受，台账记录）：Python 是「worker 线程
// 退出 + Manager 外层重启」两级，Rust 侧合并为单线程内重启策略（外部可观测
// 行为一致：反复失败 → 退避 → 最终 error）。
// ---------------------------------------------------------------------------

/// Python CameraManager 重启退避序列（秒）。
const RESTART_BACKOFFS: [u64; 4] = [1, 5, 30, 60];
/// 存活超过该秒数后失败 → 重置重启计数（Python Manager 同口径）。
const RESTART_ALIVE_RESET_S: u64 = 5;
/// 默认重启上限（Python Manager.MAX_RESTARTS 同量级）。
const DEFAULT_MAX_RESTARTS: u32 = 4;

/// 每路 worker 句柄（注册表持有；stop/ffmpeg 供 stop 路径强杀）。
pub struct WorkerHandle {
    pub stop: Arc<AtomicBool>,
    pub ffmpeg: ls_ingest::ChildSlot,
    /// 最新抽帧 JPEG（camera_snapshot 用；evidence stride 命中时更新）
    pub latest_jpeg: Mutex<Option<Vec<u8>>>,
}

/// 全局 worker 注册表（AppState.workers）。
#[derive(Default)]
pub struct Workers {
    map: Mutex<HashMap<i64, Arc<WorkerHandle>>>,
}

impl Workers {
    pub fn get(&self, cid: i64) -> Option<Arc<WorkerHandle>> {
        self.map.lock().ok()?.get(&cid).cloned()
    }

    fn insert(&self, cid: i64, h: Arc<WorkerHandle>) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(cid, h);
        }
    }

    /// 移除并请求停止（kill ffmpeg slot 解除阻塞读）；返回被停句柄。
    pub fn remove_and_stop(&self, cid: i64) -> Option<Arc<WorkerHandle>> {
        let h = self.map.lock().ok()?.remove(&cid)?;
        h.stop.store(true, Ordering::SeqCst);
        ls_ingest::Ingest::kill_slot(&h.ffmpeg);
        Some(h)
    }

    pub fn ids(&self) -> Vec<i64> {
        self.map
            .lock()
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default()
    }
}

/// 单路 worker 配置（RA1.2：从 DB 组装，不再读进程 env——make_detector 已参数化）。
pub struct WorkerConfig {
    pub camera_id: i64,
    pub spec: ls_analyzer::pipeline::CameraSpec,
    pub zones: Vec<ls_analyzer::pipeline::ZoneSpec>,
    pub rules: Vec<ls_analyzer::pipeline::RuleSpec>,
    pub calendar: HashMap<String, String>,
    pub detector: ls_analyzer::detector::DetectorConfig,
    pub evidence_fps: f64,
    pub max_restarts: u32,
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|v: &f64| *v > 0.0)
        .unwrap_or(default)
}

/// zones.polygon JSON（`[[x,y],…]`）→ 顶点；解析失败/点数 <3 返回 None（管线同滤）。
fn parse_polygon(polygon_json: &str) -> Option<Vec<(i64, i64)>> {
    let raw: Vec<Vec<i64>> = serde_json::from_str(polygon_json).ok()?;
    Some(
        raw.into_iter()
            .filter_map(|p| p.first().zip(p.get(1)).map(|(x, y)| (*x, *y)))
            .collect(),
    )
}

impl WorkerConfig {
    /// 从 DB 组装（cameras/zones/rules/calendar）。全局量（analysis_fps/
    /// file_loop/evidence_fps）与 Python 侧同源：进程 env（LS_ANALYSIS_FPS 等，
    /// 默认 5.0/False/10.0）——DB app_settings 只承载 alert.*（渠道），口径一致。
    pub fn from_db(s: &ls_store::Store, cid: i64) -> Result<Option<WorkerConfig>, String> {
        let Some(c) = s.camera(cid).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let afps = env_f64("LS_ANALYSIS_FPS", 5.0) as f32;
        let file_loop = std::env::var("LS_FILE_LOOP")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let efps = env_f64("LS_EVIDENCE_FPS", 10.0);
        let zones = s
            .zones_all()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|(_, zcid, ..)| *zcid == cid)
            .filter_map(|(id, _, name, poly)| {
                parse_polygon(&poly).map(|points| ls_analyzer::pipeline::ZoneSpec {
                    id,
                    camera_id: cid,
                    name,
                    points,
                })
            })
            .filter(|z| z.points.len() >= 3)
            .collect();
        let rules = s
            .rules_all()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|r| r.1 == cid && r.9 != 0)
            .map(|r| ls_analyzer::pipeline::RuleSpec {
                id: r.0,
                camera_id: r.1,
                zone_id: r.2.unwrap_or(0),
                behavior: r.3,
                level: r.4,
                cooldown_s: r.5,
                time_start: r.6.unwrap_or_default(),
                time_end: r.7.unwrap_or_default(),
                // RA1.1 起 rules_all 携带 days_of_week（CSV "1,2,3,4,5,6,7"）；
                // RuleSpec.weekdays 口径 = "1-5" 或 "1,2,3"，空=全部
                weekdays: r.10.unwrap_or_default(),
                threshold: r.8,
                enabled: r.9 != 0,
                holiday_only: false, // proto v1/ls-store 列未随规则读出（RA2 随模型管理补）
            })
            .collect();
        let calendar = s
            .calendar_exceptions()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|c| (c.date, c.mode))
            .collect();
        // 模型路径：进程 env LS_MODEL_PATH（未设 = NullDetector，仅人员检测为空）。
        // RA2 接模型管理（assets.lock/网页端许可确认）后改从 settings 组装。
        let mut detector = ls_analyzer::detector::DetectorConfig::default();
        if let Ok(p) = std::env::var("LS_MODEL_PATH") {
            if !p.trim().is_empty() {
                detector.model_path = Some(PathBuf::from(p.trim()));
            }
        }
        Ok(Some(WorkerConfig {
            camera_id: cid,
            spec: ls_analyzer::pipeline::CameraSpec {
                id: c.id,
                name: c.name,
                source: c.source,
                type_: c.type_,
                analysis_fps: afps,
                file_loop,
            },
            zones,
            rules,
            calendar,
            detector,
            evidence_fps: efps,
            max_restarts: DEFAULT_MAX_RESTARTS,
        }))
    }
}

/// 启动 worker（handler 入口）：组装配置 → 注册句柄 → 起分析线程。
pub fn start_worker(st: &Shared, cfg: WorkerConfig) -> Result<(), String> {
    let cid = cfg.camera_id;
    if st.workers.get(cid).is_some() {
        return Err("已在运行".into());
    }
    let handle = Arc::new(WorkerHandle {
        stop: Arc::new(AtomicBool::new(false)),
        ffmpeg: Arc::new(Mutex::new(None)),
        latest_jpeg: Mutex::new(None),
    });
    st.workers.insert(cid, Arc::clone(&handle));
    let st2 = Arc::clone(st);
    let spawned = std::thread::Builder::new()
        .name(format!("ls-worker-{cid}"))
        .spawn(move || worker_loop(st2, cfg, handle));
    match spawned {
        Ok(_) => Ok(()),
        Err(e) => {
            st.workers.remove_and_stop(cid);
            Err(format!("起 worker 线程失败: {e}"))
        }
    }
}

/// 停止 worker（幂等）：置停标志 + kill ffmpeg slot + 状态写 stopped。
/// 返回 False = 该路本来就没在运行。
pub fn stop_worker(st: &Shared, cid: i64) -> bool {
    let Some(_) = st.workers.remove_and_stop(cid) else {
        return false;
    };
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        let _ = s.set_camera_status(cid, "stopped");
    }
    true
}

/// 可中断睡眠（停止标志每 200ms 检查；True=完整睡完，False=被停止打断）。
fn sleep_interruptible(stop: &AtomicBool, d: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + d;
    while std::time::Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    !stop.load(Ordering::SeqCst)
}

fn restart_backoff(restarts: u32) -> std::time::Duration {
    std::time::Duration::from_secs(RESTART_BACKOFFS[(restarts - 1).min(3) as usize])
}

/// 重启判定（Python Manager 语义）：存活 >5s 重置计数；超限 → error 并退出。
/// 返回 False = 放弃（线程应退出）。
fn on_restart(
    store: &ls_store::Store,
    cfg: &WorkerConfig,
    restarts: &mut u32,
    alive_since: Option<std::time::Instant>,
    last_error: &str,
) -> bool {
    let alive_ok = alive_since
        .map(|t| t.elapsed().as_secs() >= RESTART_ALIVE_RESET_S)
        .unwrap_or(false);
    if alive_ok {
        *restarts = 0;
    }
    *restarts += 1;
    eprintln!(
        "[ls-worker] cam-{} 重连失败（第 {restarts}/{} 次）: {last_error}",
        cfg.camera_id, cfg.max_restarts
    );
    if *restarts > cfg.max_restarts {
        let _ = store.set_camera_status(cfg.camera_id, "error");
        eprintln!(
            "[ls-worker] cam-{} 超过重启上限，放弃（given_up → error）",
            cfg.camera_id
        );
        return false;
    }
    true
}

/// 数据面主循环（RA1.2：结构对齐 ls-analyzer bin 的 run_camera；事件出口改为
/// EventSink 直写，存证收尾线程回填 video）。运行在专用线程（阻塞 read_frame）。
fn worker_loop(st: Shared, cfg: WorkerConfig, handle: Arc<WorkerHandle>) {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return;
    };
    let _ = store.set_camera_status(cfg.camera_id, "starting");
    let enter = env_f64("LS_ZONE_ENTER_FRAMES", 2.0) as u32;
    let exit = env_f64("LS_ZONE_EXIT_FRAMES", 3.0) as u32;
    let mut pipe = ls_analyzer::pipeline::CameraPipeline::new(
        cfg.spec.clone(),
        cfg.zones.clone(),
        cfg.rules.clone(),
        cfg.calendar.clone(),
        enter,
        exit,
    );
    let mut detector = ls_analyzer::detector::make_detector(&cfg.detector);
    let evidence = Arc::new(ls_analyzer::evidence::Evidence::default());
    evidence.new_ring(cfg.spec.analysis_fps as f64, cfg.evidence_fps);
    let mut ing = ls_ingest::Ingest::new(ls_ingest::IngestConfig {
        source: cfg.spec.source.clone(),
        analysis_fps: cfg.spec.analysis_fps,
        width: 1280,
        height: 720,
        file_loop: cfg.spec.file_loop,
        rtsp_transport_tcp: true,
    });
    let sink = EventSink::new(Arc::clone(&st));
    let mut restarts = 0u32;
    #[allow(unused_assignments)] // 每轮 spawn 成功时重置（None 初始值仅首次可读路径）
    let mut alive_since: Option<std::time::Instant> = None;
    let mut running_marked = false;
    'outer: loop {
        if handle.stop.load(Ordering::SeqCst) {
            break;
        }
        if let Err(e) = ing.spawn_with_slot(Arc::clone(&handle.ffmpeg)) {
            if !on_restart(&store, &cfg, &mut restarts, None, &format!("spawn: {e}")) {
                // given_up：error 状态已写；注册表自清理（否则 stop_worker 永远命中不了）
                if let Ok(mut m) = st.workers.map.lock() {
                    m.remove(&cfg.camera_id);
                }
                return;
            }
            if !sleep_interruptible(&handle.stop, restart_backoff(restarts)) {
                break;
            }
            continue;
        }
        alive_since = Some(std::time::Instant::now());
        loop {
            if handle.stop.load(Ordering::SeqCst) {
                break 'outer;
            }
            match ing.read_frame() {
                Ok(Some(frame)) => {
                    // 存活 >5s 重置重启计数（Python Manager 口径）
                    if alive_since
                        .map(|t| t.elapsed().as_secs() >= RESTART_ALIVE_RESET_S)
                        .unwrap_or(false)
                    {
                        restarts = 0;
                    }
                    // 先喂证据环（事件帧须已在环内；同时自动补活动录像 post 帧）
                    let ev_snapshot: Option<Vec<u8>> = evidence.feed(frame.index, || {
                        ls_analyzer::evidence::encode_jpeg_bgr(
                            &frame.data,
                            frame.width,
                            frame.height,
                        )
                        .unwrap_or_default()
                    });
                    if let Some(j) = &ev_snapshot {
                        if let Ok(mut slot) = handle.latest_jpeg.lock() {
                            *slot = Some(j.clone());
                        }
                    }
                    let dets = detector.detect(&frame);
                    let ts =
                        ls_analyzer::pipeline::frame_ts_for(frame.index, cfg.spec.analysis_fps);
                    let cands = pipe.process_with_frame(
                        &dets,
                        &ts,
                        Some((&frame.data, frame.width, frame.height)),
                    );
                    if !cands.is_empty() {
                        // 事件帧快照：优先复用本帧抽帧结果，未命中 stride 则现编
                        let snap = match ev_snapshot {
                            Some(j) if !j.is_empty() => j,
                            _ => ls_analyzer::evidence::encode_jpeg_bgr(
                                &frame.data,
                                frame.width,
                                frame.height,
                            )
                            .unwrap_or_default(),
                        };
                        for cand in cands {
                            match sink.record(&cand, Some(&snap)) {
                                Ok(Some(eid)) => {
                                    spawn_finalize(&st, &evidence, eid);
                                }
                                Ok(None) => {} // 去重命中
                                Err(e) => {
                                    eprintln!("[ls-worker] cam-{} 事件落地失败: {e}", cfg.camera_id)
                                }
                            }
                        }
                    }
                    if !running_marked {
                        let _ = store.set_camera_status(cfg.camera_id, "running");
                        running_marked = true;
                    }
                }
                Ok(None) => break, // 流结束（文件源 EOF）
                Err(e) => {
                    eprintln!("[ls-worker] cam-{} 读帧失败: {e}", cfg.camera_id);
                    break;
                }
            }
        }
        ing.stop();
        if handle.stop.load(Ordering::SeqCst) {
            break;
        }
        if cfg.spec.type_ == "file" && !cfg.spec.file_loop {
            evidence.flush();
            let _ = store.set_camera_status(cfg.camera_id, "stopped");
            if let Ok(mut m) = st.workers.map.lock() {
                m.remove(&cfg.camera_id);
            }
            return;
        }
        if !on_restart(&store, &cfg, &mut restarts, alive_since, "stream ended") {
            return;
        }
        if !sleep_interruptible(&handle.stop, restart_backoff(restarts)) {
            break;
        }
    }
    ing.stop();
    evidence.flush();
    // given_up 路径已写 error；其余（stop/stop 打断）写 stopped
    let _ = store.set_camera_status(cfg.camera_id, "stopped");
}

/// 存证收尾：candidate 时注册 ActiveRec（pre=环快照），post 帧由 worker 循环
/// 自动补齐（evidence.feed），到期后本线程拼帧 encode_mp4 → set_event_video。
fn spawn_finalize(st: &Shared, evidence: &Arc<ls_analyzer::evidence::Evidence>, eid: i64) {
    // 并发上限（Python MAX_CONCURRENT_RECORDINGS 同口径）：超限拒绝本次回填
    let (rec, out_fps) = {
        let pre: Vec<Vec<u8>> = {
            let ring = evidence.ring.lock().unwrap_or_else(|e| e.into_inner());
            match ring.as_ref() {
                Some(r) => r.snapshot().into_iter().map(|f| f.data).collect(),
                None => return,
            }
        };
        let fps = {
            let ring = evidence.ring.lock().unwrap_or_else(|e| e.into_inner());
            ring.as_ref().map(|r| r.out_fps()).unwrap_or(10.0)
        };
        let rec = Arc::new(ls_analyzer::evidence::ActiveRec::new(pre));
        let mut recs = evidence.recs.lock().unwrap_or_else(|e| e.into_inner());
        if recs.len() >= ls_analyzer::evidence::MAX_CONCURRENT_RECORDINGS {
            eprintln!("[ls-worker] event#{eid} 并发录像超限，跳过视频回填");
            return;
        }
        recs.push(Arc::clone(&rec));
        (rec, fps)
    };
    let st = Arc::clone(st);
    let evidence = Arc::clone(evidence);
    let spawned = std::thread::Builder::new()
        .name(format!("ls-finalize-{eid}"))
        .spawn(move || {
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs_f64(VIDEO_POST_WAIT_S);
            while std::time::Instant::now() < deadline {
                if rec.done.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            let frames = rec.frames.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let sink = EventSink::new(st);
            match sink.finalize(eid, &frames, out_fps) {
                Ok(Some(p)) => println!("[ls-worker] event#{eid} 视频已回填: {p}"),
                Ok(None) => {}
                Err(e) => eprintln!("[ls-worker] event#{eid} 视频回填失败: {e}"),
            }
            // 从活动列表移除（防长跑泄漏）
            if let Ok(mut recs) = evidence.recs.lock() {
                recs.retain(|r| !Arc::ptr_eq(r, &rec));
            }
        });
    if let Err(e) = spawned {
        eprintln!("[ls-worker] event#{eid} 收尾线程创建失败: {e}");
    }
}

/// post 等待 = VIDEO_POST_S + 0.5s 容差（evidence.feed 的 done 判定同值）+ 1s 余量。
const VIDEO_POST_WAIT_S: f64 = ls_analyzer::evidence::VIDEO_POST_S + 1.5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::helpers::{seed_schema, test_state_custom};

    fn cand(cid: i64, key: &str) -> ls_analyzer::pipeline::Candidate {
        ls_analyzer::pipeline::Candidate {
            camera_id: cid,
            zone_id: None,
            behavior: "person_enter".into(),
            level: 2,
            track_id: 7,
            detail: "测试事件".into(),
            ts_iso: "2026-11-13T12:00:00+08:00".into(),
            evidence_key: key.into(),
        }
    }

    #[test]
    fn record_writes_snapshot_event_and_broadcasts() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("test.db").to_string_lossy().into_owned();
        let st = test_state_custom(&db, "", false, tmp.path().to_str().unwrap(), true, "");
        // Store::open 不跑迁移（schema 由 Python 侧拥有，ADR-0008）——先播种
        seed_schema(tmp.path().join("test.db").as_path());
        {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            s.add_camera_full("cam-1", "rtsp://x", "rtsp", "").unwrap();
        }
        let sink = EventSink::with_paths(
            st.clone(),
            SinkPaths::from_data_dir(tmp.path().to_str().unwrap()),
        );
        let mut rx = st.events_tx.subscribe();
        let jpeg: &[u8] = b"\xff\xd8\xff\xe0FAKEJPEG";
        let eid = sink
            .record(&cand(1, "cam:1:100"), Some(jpeg))
            .unwrap()
            .expect("首条不应去重");
        // DB 行 + 截图文件
        let s = ls_store::Store::open(&st.db_path).unwrap();
        let row = s.event_by_id(eid).unwrap().expect("事件行存在");
        assert_eq!(row.type_, "person_enter");
        assert_eq!(row.track_id, 7);
        let snap = row.snapshot.expect("截图路径已入库");
        assert!(std::path::Path::new(&snap).is_file(), "截图文件真实存在");
        // WS 广播行口径与 /api/events 一致（无绝对路径泄漏）
        let payload = rx.try_recv().unwrap();
        assert!(payload.contains("\"has_snapshot\":true"), "{payload}");
        assert!(!payload.contains("FAKEJPEG"));
        assert!(!payload.contains(&snap), "绝对路径不外泄");
    }

    #[test]
    fn record_dedups_on_evidence_key_behavior_track() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("test.db").to_string_lossy().into_owned();
        let st = test_state_custom(&db, "", false, tmp.path().to_str().unwrap(), true, "");
        seed_schema(tmp.path().join("test.db").as_path());
        {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            s.add_camera_full("cam-1", "rtsp://x", "rtsp", "").unwrap();
        }
        let sink = EventSink::with_paths(
            st.clone(),
            SinkPaths::from_data_dir(tmp.path().to_str().unwrap()),
        );
        sink.record(&cand(1, "cam:1:100"), None).unwrap().unwrap();
        let again = sink.record(&cand(1, "cam:1:100"), None).unwrap();
        assert!(again.is_none(), "同 key 重放应去重");
        let s = ls_store::Store::open(&st.db_path).unwrap();
        assert_eq!(s.count_events().unwrap(), 1, "去重后只有一条");
    }

    #[test]
    fn finalize_empty_frames_errors_without_touching_db() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("test.db").to_string_lossy().into_owned();
        let st = test_state_custom(&db, "", false, tmp.path().to_str().unwrap(), true, "");
        seed_schema(tmp.path().join("test.db").as_path());
        let s = ls_store::Store::open(&st.db_path).unwrap();
        let eid = s
            .add_event(&ls_store::NewEvent {
                camera_id: 1,
                type_: "person_enter",
                level: 2,
                track_id: 1,
                detail: "x",
                snapshot: None,
                ts: "2026-11-13T12:00:00+08:00",
            })
            .unwrap();
        let sink = EventSink::with_paths(
            st.clone(),
            SinkPaths::from_data_dir(tmp.path().to_str().unwrap()),
        );
        let err = sink.finalize(eid, &[], 5.0).unwrap_err();
        assert_eq!(err, "no frames");
        let s = ls_store::Store::open(&st.db_path).unwrap();
        assert!(s.event_by_id(eid).unwrap().unwrap().video.is_none());
    }

    // ---- RA1.2：worker 生命周期 ----

    #[test]
    fn parse_polygon_ok() {
        let pts = super::parse_polygon("[[10,20],[30,40],[50,60]]").unwrap();
        assert_eq!(pts, vec![(10, 20), (30, 40), (50, 60)]);
        assert!(super::parse_polygon("not json").is_none());
        // <3 点过滤在 from_db 层（与管线 ZoneSpec 滤网一致），parse 只做形态解析
        assert_eq!(super::parse_polygon("[[1,2]]").unwrap().len(), 1);
    }

    #[test]
    fn worker_config_from_db_assembles_camera_zones_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("test.db").to_string_lossy().into_owned();
        seed_schema(tmp.path().join("test.db").as_path());
        let s = ls_store::Store::open(&db).unwrap();
        let cid = s
            .add_camera_full("cam-1", "rtsp://10.0.0.5/stream", "rtsp", "")
            .unwrap();
        let zid = s
            .add_zone(cid, "桌面", "[[0,0],[100,0],[100,100]]")
            .unwrap();
        let rid = s
            .add_rule(
                cid,
                Some(zid),
                "person_enter",
                2,
                30,
                Some("08:00"),
                Some("18:00"),
                1,
                true,
            )
            .unwrap();
        s.conn()
            .execute(
                "UPDATE rules SET days_of_week='1,2,3,4,5' WHERE id=?",
                [rid],
            )
            .unwrap();
        let cfg = WorkerConfig::from_db(&s, cid).unwrap().expect("配置组装");
        assert_eq!(cfg.camera_id, cid);
        assert_eq!(cfg.spec.source, "rtsp://10.0.0.5/stream");
        assert_eq!(cfg.spec.analysis_fps, 5.0, "默认 LS_ANALYSIS_FPS=5.0");
        assert_eq!(cfg.zones.len(), 1);
        assert_eq!(cfg.zones[0].points, vec![(0, 0), (100, 0), (100, 100)]);
        assert_eq!(cfg.rules.len(), 1);
        assert_eq!(
            cfg.rules[0].weekdays, "1,2,3,4,5",
            "RA1.1 days_of_week 进 RuleSpec"
        );
        assert_eq!(cfg.rules[0].zone_id, zid);
        assert_eq!(cfg.rules[0].time_start, "08:00");
        assert_eq!(cfg.calendar.len(), 0);
        assert!(cfg.detector.model_path.is_none(), "默认 NullDetector");
        // 不存在的相机 → None
        assert!(WorkerConfig::from_db(&s, 999).unwrap().is_none());
    }

    #[test]
    fn start_stop_lifecycle_with_bad_source() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("test.db").to_string_lossy().into_owned();
        let st = test_state_custom(&db, "", false, tmp.path().to_str().unwrap(), true, "");
        seed_schema(tmp.path().join("test.db").as_path());
        let mut cfg = {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            let cid = s
                .add_camera_full("cam-bad", "rtsp://127.0.0.1:9/x", "rtsp", "")
                .unwrap();
            WorkerConfig::from_db(&s, cid).unwrap().unwrap()
        };
        // 阶段 1：max_restarts 大 → worker 驻留注册表（spawn 失败进入 1s 退避），
        // 期间重复启动必须被拒（「已在运行」）
        cfg.max_restarts = 99;
        start_worker(&st, cfg).unwrap();
        assert!(st.workers.get(1).is_some());
        let dup = {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            WorkerConfig::from_db(&s, 1).unwrap().unwrap()
        };
        assert!(start_worker(&st, dup).is_err(), "重复启动必须被拒");
        // 阶段 2：stop 打断退避 → 注册表移除 + 状态最终 stopped
        assert!(stop_worker(&st, 1));
        assert!(st.workers.get(1).is_none(), "停止后注册表移除");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            let status: String = s
                .conn()
                .query_row("SELECT status FROM cameras WHERE id=1", [], |r| r.get(0))
                .unwrap();
            if status == "stopped" {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "线程应尽快落 stopped");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(!stop_worker(&st, 1), "幂等停止");
    }

    #[test]
    fn given_up_after_max_restarts_marks_error_and_cleans_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("test.db").to_string_lossy().into_owned();
        let st = test_state_custom(&db, "", false, tmp.path().to_str().unwrap(), true, "");
        seed_schema(tmp.path().join("test.db").as_path());
        let mut cfg = {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            let cid = s
                .add_camera_full("cam-bad", "rtsp://127.0.0.1:9/x", "rtsp", "")
                .unwrap();
            WorkerConfig::from_db(&s, cid).unwrap().unwrap()
        };
        cfg.max_restarts = 0; // 首次 spawn 失败即 given_up（不等退避序列）
        start_worker(&st, cfg).unwrap();
        // given_up：状态 error + 注册表自清理（≤5s 轮询）
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            let status: String = s
                .conn()
                .query_row("SELECT status FROM cameras WHERE id=1", [], |r| r.get(0))
                .unwrap();
            if status == "error" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "5s 内应 given_up（error）"
            );
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(st.workers.get(1).is_none(), "given_up 后注册表自清理");
        assert!(!stop_worker(&st, 1));
    }

    #[test]
    fn stop_worker_interrupts_backoff() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("test.db").to_string_lossy().into_owned();
        let st = test_state_custom(&db, "", false, tmp.path().to_str().unwrap(), true, "");
        seed_schema(tmp.path().join("test.db").as_path());
        let mut cfg = {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            let cid = s
                .add_camera_full("cam-slow", "rtsp://127.0.0.1:9/x", "rtsp", "")
                .unwrap();
            WorkerConfig::from_db(&s, cid).unwrap().unwrap()
        };
        cfg.max_restarts = 3; // 退避序列中会被 stop 打断
        start_worker(&st, cfg).unwrap();
        assert!(st.workers.get(1).is_some());
        assert!(stop_worker(&st, 1), "运行中停止返回 true");
        assert!(st.workers.get(1).is_none(), "停止后注册表移除");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            let status: String = s
                .conn()
                .query_row("SELECT status FROM cameras WHERE id=1", [], |r| r.get(0))
                .unwrap();
            if status == "stopped" {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "线程应尽快落 stopped");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(!stop_worker(&st, 1), "幂等停止");
    }

    #[tokio::test]
    async fn snapshot_503_when_worker_absent_and_404_when_missing() {
        use crate::tests::helpers::{body_json, req, seed_schema as _seed, test_state};
        use crate::*;
        use axum::http::StatusCode;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        _seed(&db);
        // 播一台相机（exist→404 对照组用 99）
        {
            let s = ls_store::Store::open(db.to_str().unwrap()).unwrap();
            s.add_camera_full("cam-1", "rtsp://x", "rtsp", "").unwrap();
        }
        let app = build_router(test_state(db.to_str().unwrap()));
        let r = req(&app, "GET", "/api/cameras/1/snapshot", None).await;
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        let j = body_json(r).await;
        assert_eq!(j["code"], "no_worker");
        let r = req(&app, "GET", "/api/cameras/99/snapshot", None).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }
}
