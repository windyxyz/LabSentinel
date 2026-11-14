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
//! 告警首发派发（Python dispatcher.dispatch → 8 渠道）已在 RA1.3 随 worker
//! 生命周期接入（复用 ls_dispatch::deliver + escalation 既有链路）。

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
        // RA2-S5：AI 复核入队（默认 provider=none 零开销；只写 ai_review 标记）
        let _ = st
            .genai
            .maybe_enqueue(eid, snapshot.as_deref(), cand.level, &st.db_path);
        // WS 广播：行口径与 /api/events 完全一致（绝对路径不外泄——event_json
        // 只出 has_snapshot/has_video 布尔，与 Python 桥 pop(snapshot/video) 等价）
        if let Ok(Some(row)) = store.event_by_id(eid) {
            let _ = st.events_tx.send(event_json(&row).to_string());
        }
        let cam_name = store
            .camera(cand.camera_id)
            .ok()
            .flatten()
            .map(|c| c.name)
            .unwrap_or_default();
        // RA2-S2：MQTT 总线发布（独立于静默/渠道过滤——Python alerts.dispatch
        // 入口同位；连接失败不阻塞事件链）。zones 契约数组：候选不带区域名，空数组。
        if let Some(bus) = &st.mqtt {
            let api_base = std::env::var("LS_MQTT_API_BASE").unwrap_or_else(|_| {
                format!(
                    "http://127.0.0.1:{}",
                    std::env::var("LS_PORT").unwrap_or_else(|_| "8000".into())
                )
            });
            let fields = ls_dispatch::mqtt::EventFields {
                id: eid,
                ts: &cand.ts_iso,
                camera_id: cand.camera_id,
                camera_name: Some(&cam_name),
                type_: &cand.behavior,
                level: cand.level,
                track_id: cand.track_id,
                detail: &cand.detail,
                zones: &[],
                has_snapshot: snapshot.is_some(),
                api_base: &api_base,
            };
            let _ = bus.publish_event(&fields);
        }
        // RA1.3：告警首发派发（外部 8 渠道；静默/过滤/限流复用 ls-jobs 链路，
        // 与 escalation 共享渠道快照和限流器）。投递失败只影响外部通知，
        // 不影响事件已入库/已广播的事实。
        // 性能 P0-2（审计）：dispatch_event 是同步 ureq——单渠道最坏 ~40s
        // （4 次尝试+退避），×8 渠道会把帧循环停摆数分钟（ffmpeg 管道积压）。
        // 改投后台线程（对齐 Python ThreadPoolExecutor 异步语义）。
        let dev = ls_dispatch::DispatchEvent {
            id: eid,
            camera_id: cand.camera_id,
            type_: cand.behavior.clone(),
            level: cand.level,
            detail: cand.detail.clone(),
            ts: cand.ts_iso.clone(),
            camera_name: cam_name,
        };
        let jobs = Arc::clone(&st.jobs);
        let db_path = st.db_path.clone();
        let eid_log = eid;
        let _ = std::thread::Builder::new()
            .name(format!("ls-dispatch-{eid}"))
            .spawn(move || {
                if let Ok(s) = ls_store::Store::open(&db_path) {
                    let submitted = jobs.dispatch_event(&s, &dev);
                    if submitted > 0 {
                        println!("[ls-dispatch] event#{eid_log} 首发派发 {submitted} 渠道");
                    }
                }
            });
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
    /// RA1.3：folder worker 立即扫描唤醒（POST /folders/{cid}/scan）
    pub wake: Arc<AtomicBool>,
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
}

/// 单路 worker 配置（RA1.2：从 DB 组装，不再读进程 env——make_detector 已参数化）。
pub struct WorkerConfig {
    pub camera_id: i64,
    pub spec: ls_analyzer::pipeline::CameraSpec,
    pub zones: Vec<ls_analyzer::pipeline::ZoneSpec>,
    pub rules: Vec<ls_analyzer::pipeline::RuleSpec>,
    pub calendar: HashMap<String, String>,
    /// cameras.analyzer 列原值（python/rust/vlm）——RA5 模型模块化：
    /// "vlm" = 该路强制走外部联网模型判定
    pub analyzer: String,
    pub detector: ls_analyzer::detector::DetectorConfig,
    /// RA2-S1：PPE 级联配置（None = 未配置模型/下载未完成 → 仅人员检测）
    pub ppe: Option<ls_analyzer::ppe::PpeConfig>,
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
                // RuleSpec.weekdays 口径 = "1-5" 或 "1,2,3"，空=全部；
                // RA4.2 起 holiday_only 随行读出（节假日限定规则生效）
                weekdays: r.10.unwrap_or_default(),
                threshold: r.8,
                enabled: r.9 != 0,
                holiday_only: r.11,
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
        // RA5（模型模块化）：app_settings 的 model.active 优先（/api/models/active
        // UI 切换写入）→ 回退 env LS_MODEL_PATH；都空 = NullDetector
        let model_active = s
            .setting_get("model.active")
            .ok()
            .flatten()
            .filter(|p| !p.is_empty());
        let model_env = std::env::var("LS_MODEL_PATH")
            .ok()
            .filter(|v| !v.trim().is_empty());
        detector.model_path = model_active.or(model_env).map(PathBuf::from);
        // RA2-S1：PPE 级联（env LS_PPE_MODEL 若配，过锚点校验；类名/阈值走
        // PpeConfig 默认——与 sidecar 默认口径一致）
        let ppe = std::env::var("LS_PPE_MODEL").ok().and_then(|raw| {
            let p = PathBuf::from(raw.trim());
            ls_analyzer::detector::anchor_model_file(&p).map(|model_path| {
                ls_analyzer::ppe::PpeConfig {
                    model_path,
                    ..Default::default()
                }
            })
        });
        Ok(Some(WorkerConfig {
            camera_id: cid,
            analyzer: c.analyzer.clone(),
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
            ppe,
            evidence_fps: efps,
            max_restarts: DEFAULT_MAX_RESTARTS,
        }))
    }
}

/// 启动 worker（handler 入口）：组装配置 → 注册句柄 → 起分析线程。
/// folder 源走扫描登记线程（RA1.3），rtsp/file/usb 走 ffmpeg 帧循环。
pub fn start_worker(st: &Shared, cfg: WorkerConfig) -> Result<(), String> {
    let cid = cfg.camera_id;
    if st.workers.get(cid).is_some() {
        return Err("已在运行".into());
    }
    let handle = Arc::new(WorkerHandle {
        stop: Arc::new(AtomicBool::new(false)),
        ffmpeg: Arc::new(Mutex::new(None)),
        latest_jpeg: Mutex::new(None),
        wake: Arc::new(AtomicBool::new(false)),
    });
    st.workers.insert(cid, Arc::clone(&handle));
    let st2 = Arc::clone(st);
    let is_folder = cfg.spec.type_ == "folder";
    let spawned = if is_folder {
        std::thread::Builder::new()
            .name(format!("ls-folder-{cid}"))
            .spawn(move || folder_loop(st2, cfg, handle))
    } else {
        std::thread::Builder::new()
            .name(format!("ls-worker-{cid}"))
            .spawn(move || worker_loop(st2, cfg, handle))
    };
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
    // RA2-S1：PPE 级联注入管线（未配置模型 → None → 仅人员检测，ppe 恒 Unknown）
    let ppe_runner = cfg.ppe.as_ref().and_then(ls_analyzer::ppe::make_ppe_runner);
    pipe.set_ppe(ppe_runner);
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
    // RA5：外部联网 VLM 视频判定（LS_ANALYZER_BACKEND=vlm）——抽样多帧交
    // 外部模型判定违规；限流默认 6 次/分钟/路（token 成本克制）。
    // 模型模块化：per-camera cameras.analyzer="vlm" 也可插拔启用（优先于全局 env）。
    let mut vlm_cfg = crate::vlm::cfg_from_env();
    if cfg.analyzer == "vlm" {
        vlm_cfg.enabled = true;
    }
    let vlm_interval = std::time::Duration::from_secs(
        (60f64 / env_f64("LS_VLM_MAX_PER_MIN", 6.0).max(0.1)).round() as u64,
    );
    let mut last_vlm = std::time::Instant::now() - vlm_interval; // 首帧即可判
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
                    // RA5：外部联网 VLM 视频判定抽样（限流：证据环多帧 → Gemini
                    // 原生视频 / openai_compat 多图 → violations → 事件）。
                    // 判定在帧循环线程做一次同步 HTTP（间隔由 vlm_interval 限流，
                    // 默认 6 次/分钟/路——单次最坏 30s 不会淹没帧率）。
                    if vlm_cfg.enabled && last_vlm.elapsed() >= vlm_interval {
                        last_vlm = std::time::Instant::now();
                        let ring_frames: Vec<Vec<u8>> = evidence
                            .ring
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .as_ref()
                            .map(|r| r.snapshot().into_iter().map(|f| f.data).collect())
                            .unwrap_or_default();
                        let sampled = crate::vlm::sample_frames(&ring_frames, 8);
                        if !sampled.is_empty() {
                            match crate::vlm::analyze_frames(
                                &vlm_cfg,
                                crate::vlm::VLM_PROMPT,
                                &sampled,
                            ) {
                                Ok(v) => {
                                    let violations: Vec<String> = v["violations"]
                                        .as_array()
                                        .map(|a| {
                                            a.iter()
                                                .filter_map(|x| x.as_str().map(String::from))
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    let summary = v["summary"].as_str().unwrap_or("").to_string();
                                    let persons = v["persons"].as_i64().unwrap_or(0);
                                    let ts = ls_analyzer::pipeline::frame_ts_for(
                                        frame.index,
                                        cfg.spec.analysis_fps,
                                    );
                                    // 违规截图：证据环尾帧（最新画面）
                                    let jpeg = ring_frames.last().cloned().unwrap_or_default();
                                    for behavior in &violations {
                                        let cand = ls_analyzer::pipeline::Candidate {
                                            camera_id: cfg.camera_id,
                                            zone_id: None,
                                            behavior: behavior.clone(),
                                            level: 2,
                                            track_id: 0,
                                            detail: format!(
                                                "VLM 视频判定：{behavior}（{persons}人）— {summary}"
                                            ),
                                            ts_iso: ts.clone(),
                                            evidence_key: format!("vlm:{}:{behavior}", frame.index),
                                        };
                                        if let Ok(Some(veid)) = sink.record(&cand, Some(&jpeg)) {
                                            spawn_finalize(&st, &evidence, veid);
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!(
                                        "[ls-worker] cam-{} VLM 判定失败（降级跳过）: {e}",
                                        cfg.camera_id
                                    );
                                }
                            }
                        }
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

// ---------------------------------------------------------------------------
// RA1.3：folder worker（扫描登记 → folder_images；识别推理 RA2）
// 对齐 Python FolderWorker 的调度语义（轮询 + wake 即扫 + 目录缺失 error 重试
// + stop → stopped）；识别字段如实 0 占位——Python 侧本就声明「folder 源暂不
// 在 Rust 数据面支持范围」（pipeline.py:674），RA2 引图像解码依赖后补推理。
// ---------------------------------------------------------------------------

/// UNIX 秒 → ISO "YYYY-MM-DDTHH:MM:SS"（UTC 口径；std 无本地时区。同一路相机
/// 同一时刻只由一个实现写入，字典序/day 截断与 Python 混写仍成立）。
fn iso_secs(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

/// 可中断等待：stop → false；wake → true（提前醒，调用方消费标志）。
fn sleep_interruptible_wake(stop: &AtomicBool, wake: &AtomicBool, d: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + d;
    while std::time::Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        if wake.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    !stop.load(Ordering::SeqCst)
}

fn folder_loop(st: Shared, cfg: WorkerConfig, handle: Arc<WorkerHandle>) {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return;
    };
    let _ = store.set_camera_status(cfg.camera_id, "starting");
    let root = PathBuf::from(cfg.spec.source.trim());
    let interval = env_f64("LS_FOLDER_POLL_S", 3.0).max(0.5);
    let scan_cfg = ls_ingest::folder::FolderScanConfig::default();
    let internal = Some(PathBuf::from(&st.data_dir)); // 截图/视频数据目录不入库
    let mut vlm_cfg = crate::vlm::cfg_from_env();
    if cfg.analyzer == "vlm" {
        vlm_cfg.enabled = true; // RA5 模型模块化：per-camera 插拔外部联网模型
    }
    let mut marked: Option<String> = None;
    let mut set_status = |v: &str| {
        if marked.as_deref() != Some(v) {
            let _ = store.set_camera_status(cfg.camera_id, v);
            marked = Some(v.into());
        }
    };
    // RA2 反思修正：识别器/管件在循环外构造一次——此前在 loop 体内，
    // 每 tick（3s）都重新加载 YOLO 模型（数百 ms 级开销 × 每 3s）
    let mut detector = ls_analyzer::detector::make_detector(&cfg.detector);
    let mut ppe_runner = cfg.ppe.as_ref().and_then(ls_analyzer::ppe::make_ppe_runner);
    let sink = EventSink::new(Arc::clone(&st));
    loop {
        if handle.stop.load(Ordering::SeqCst) {
            break;
        }
        handle.wake.store(false, Ordering::SeqCst); // 消费上一轮唤醒
        if !root.is_dir() {
            set_status("error"); // Python：目录缺失 → error 状态持续重试
            if !sleep_interruptible_wake(
                &handle.stop,
                &handle.wake,
                std::time::Duration::from_secs_f64(interval.max(5.0)),
            ) {
                break;
            }
            continue;
        }
        set_status("running");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let cands = ls_ingest::folder::scan_candidates(&root, &scan_cfg, now, internal.as_deref());
        let max_per_tick = env_f64("LS_FOLDER_MAX_PER_TICK", 20.0).max(1.0) as usize;
        let mut scanned = 0usize;
        for c in cands.iter().take(max_per_tick) {
            if handle.stop.load(Ordering::SeqCst) {
                break;
            }
            let ts = iso_secs(c.mtime_s as u64); // Python：ts=文件 mtime
            let processed_ts = iso_secs(now as u64);
            let name = c.path.file_name().map(|n| n.to_string_lossy().into_owned());
            let mtime_ns = (c.mtime_s * 1e9) as i64;
            let path_s = c.path.to_string_lossy().into_owned();
            // 先幂等登记（唯一键 camera_id,path）——失败不阻断本轮其它图片
            let iid = match store.add_folder_image(
                cfg.camera_id,
                &path_s,
                name.as_deref(),
                mtime_ns,
                c.size as i64,
                &ts,
                &processed_ts,
                None,
                None,
                0,
                0,
                0,
                0,
                "[]",
                "[]",
                false,
                false,
                None,
                None,
            ) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!(
                        "[ls-folder] cam-{} 登记失败 {}: {e}",
                        cfg.camera_id,
                        c.path.display()
                    );
                    continue;
                }
            };
            // 已识别过的行跳过（width 非空 = 跑过识别；RA1.3 占位行 width=NULL
            // 会被真值回填）——唯一键幂等 + 此判据防重复解码/重复建事件
            if let Ok(Some(row)) = store.folder_image(iid) {
                if row.width.is_some() {
                    continue;
                }
            }
            // RA5：外部联网 VLM 判定路径（LS_ANALYZER_BACKEND=vlm + LS_VLM_*）——
            // 解码后单图交外部模型，persons/violations 真值回填+事件；失败降级
            // 如实落 error（绝不误报）。与 YOLO 路径互斥，continue 隔离。
            if vlm_cfg.enabled {
                let (bgr, w, h) = match ls_ingest::decode_image(&c.path, 1280, 720) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = store.update_folder_image_analysis(
                            iid,
                            0,
                            0,
                            0,
                            0,
                            0,
                            0,
                            "[]",
                            "[]",
                            false,
                            false,
                            None,
                            Some(&format!("无法解码（损坏或非图片文件）: {e}")),
                            &processed_ts,
                        );
                        continue;
                    }
                };
                let jpeg = ls_analyzer::evidence::encode_jpeg_bgr(&bgr, w, h).unwrap_or_default();
                let verdict = if jpeg.is_empty() {
                    Err("JPEG 编码失败".into())
                } else {
                    crate::vlm::analyze_frames(
                        &vlm_cfg,
                        crate::vlm::VLM_PROMPT,
                        std::slice::from_ref(&jpeg),
                    )
                };
                match verdict {
                    Ok(v) => {
                        let persons = v["persons"].as_i64().unwrap_or(0);
                        let violations: Vec<String> = v["violations"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let no_ppe = v["no_ppe"].as_i64().unwrap_or(0);
                        let key = persons > 0;
                        let violation = !violations.is_empty();
                        let (snapshot_path, snapshot_jpeg) = if violation {
                            let snap_dir = PathBuf::from(&st.data_dir).join("snapshots");
                            let _ = std::fs::create_dir_all(&snap_dir);
                            let snap_path = snap_dir.join(format!("folder_{iid}.jpg"));
                            if !jpeg.is_empty() && std::fs::write(&snap_path, &jpeg).is_ok() {
                                (Some(snap_path.to_string_lossy().into_owned()), Some(jpeg))
                            } else {
                                (None, None)
                            }
                        } else {
                            (None, None)
                        };
                        // verdict 全量存 zones 列（诊断：summary+计数字段）
                        let zones_json = v.to_string();
                        let violations_json =
                            serde_json::to_string(&violations).unwrap_or_else(|_| "[]".into());
                        let _ = store.update_folder_image_analysis(
                            iid,
                            w as i64,
                            h as i64,
                            persons,
                            persons,
                            no_ppe,
                            persons.saturating_sub(no_ppe),
                            &zones_json,
                            &violations_json,
                            key,
                            violation,
                            snapshot_path.as_deref(),
                            None,
                            &processed_ts,
                        );
                        if violation {
                            for behavior in &violations {
                                let cand = ls_analyzer::pipeline::Candidate {
                                    camera_id: cfg.camera_id,
                                    zone_id: None,
                                    behavior: behavior.clone(),
                                    level: 2,
                                    track_id: 0,
                                    detail: format!(
                                        "VLM 判定：图片 {} 命中 {}（{}人/未穿服 {}）— {}",
                                        name.as_deref().unwrap_or(""),
                                        behavior,
                                        persons,
                                        no_ppe,
                                        v["summary"].as_str().unwrap_or("")
                                    ),
                                    ts_iso: processed_ts.clone(),
                                    evidence_key: format!("folder:{iid}:{behavior}"),
                                };
                                if let Ok(Some(veid)) = sink.record(&cand, snapshot_jpeg.as_deref())
                                {
                                    let _ = store.set_folder_image_event(iid, veid);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = store.update_folder_image_analysis(
                            iid,
                            0,
                            0,
                            0,
                            0,
                            0,
                            0,
                            "[]",
                            "[]",
                            false,
                            false,
                            None,
                            Some(&format!("VLM 判定失败: {e}")),
                            &processed_ts,
                        );
                    }
                }
                scanned += 1;
                continue;
            }
            // 单图解码 → BGR 1280x720（ffmpeg；中文路径安全）
            let (bgr, w, h) = match ls_ingest::decode_image(&c.path, 1280, 720) {
                Ok(v) => v,
                Err(e) => {
                    let _ = store.update_folder_image_analysis(
                        iid,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        "[]",
                        "[]",
                        false,
                        false,
                        None,
                        Some(&format!("无法解码（损坏或非图片文件）: {e}")),
                        &processed_ts,
                    );
                    continue;
                }
            };
            let frame = ls_ingest::Frame {
                width: w,
                height: h,
                data: bgr.clone(),
                index: 0,
            };
            let mut dets = detector.detect(&frame);
            // PPE 级联：逐人框判定（未配 PPE 模型 → ppe 恒 Unknown）
            if let Some(runner) = ppe_runner.as_mut() {
                let boxes: Vec<[f32; 4]> = dets.iter().map(|d| d.box_).collect();
                let statuses = runner.statuses(&bgr, w, h, &boxes);
                for (d, st) in dets.iter_mut().zip(statuses) {
                    d.ppe = st;
                }
            }
            // 区域锚点 = 人框底边中点（与 pipeline.rs 同口径）；逐区统计
            let mut zone_hits: Vec<serde_json::Value> = Vec::new();
            let mut in_zone_ids: std::collections::HashSet<usize> = Default::default();
            let mut ppe_no = 0i64;
            let mut ppe_wearing = 0i64;
            let mut violations: Vec<String> = Vec::new();
            for z in &cfg.zones {
                let poly: Vec<ls_core::Point> = z.points.clone();
                let mut zone_person = 0i64;
                let mut zone_no = 0i64;
                let mut zone_wear = 0i64;
                for (di, d) in dets.iter().enumerate() {
                    let anchor = (((d.box_[0] + d.box_[2]) / 2.0) as i64, d.box_[3] as i64);
                    if ls_core::point_in_polygon(&poly, anchor) {
                        zone_person += 1;
                        in_zone_ids.insert(di);
                        match d.ppe {
                            ls_analyzer::pipeline::PpeStatus::NoPpe => {
                                zone_no += 1;
                                ppe_no += 1; // 跨区去重按人算，此处按统计近似（同图同人通常只在一个区）
                            }
                            ls_analyzer::pipeline::PpeStatus::Wearing => {
                                zone_wear += 1;
                                ppe_wearing += 1;
                            }
                            ls_analyzer::pipeline::PpeStatus::Unknown => {}
                        }
                    }
                }
                if zone_person > 0 {
                    zone_hits.push(serde_json::json!({
                        "zone_id": z.id, "name": z.name,
                        "persons": zone_person, "no": zone_no, "wearing": zone_wear,
                    }));
                    if zone_no > 0 {
                        violations.push("enter_no_ppe".into());
                    }
                }
            }
            let persons = dets.len() as i64;
            let in_zone = in_zone_ids.len() as i64;
            // occupancy 规则（threshold < 人数 且 zone_id=0 整画面口径）
            for r in &cfg.rules {
                if r.behavior == "occupancy_over" && r.enabled && persons > r.threshold {
                    violations.push("occupancy_over".into());
                }
            }
            violations.sort();
            violations.dedup();
            let key = persons > 0;
            let violation = !violations.is_empty();
            // 违规快照：从解码帧现编 JPEG（q80，与事件截图同口径），落盘
            // data/snapshots 且保留字节供 EventSink 入库（绝对路径入 DB 与 Python 同口径）
            let (snapshot_path, snapshot_jpeg) = if violation {
                let snap_dir = PathBuf::from(&st.data_dir).join("snapshots");
                let _ = std::fs::create_dir_all(&snap_dir);
                let snap_path = snap_dir.join(format!("folder_{iid}.jpg"));
                let jpeg = ls_analyzer::evidence::encode_jpeg_bgr(&bgr, w, h).unwrap_or_default();
                if !jpeg.is_empty() && std::fs::write(&snap_path, &jpeg).is_ok() {
                    (Some(snap_path.to_string_lossy().into_owned()), Some(jpeg))
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };
            let zones_json = serde_json::to_string(&zone_hits).unwrap_or_else(|_| "[]".into());
            let violations_json =
                serde_json::to_string(&violations).unwrap_or_else(|_| "[]".into());
            let _ = store.update_folder_image_analysis(
                iid,
                w as i64,
                h as i64,
                persons,
                in_zone,
                ppe_no,
                ppe_wearing,
                &zones_json,
                &violations_json,
                key,
                violation,
                snapshot_path.as_deref(),
                None,
                &processed_ts,
            );
            // 违规 → 事件（走 EventSink：入库 + WS 广播 + MQTT + 首发派发）
            if violation {
                for behavior in &violations {
                    let cand = ls_analyzer::pipeline::Candidate {
                        camera_id: cfg.camera_id,
                        zone_id: None,
                        behavior: behavior.clone(),
                        level: 2,
                        track_id: 0,
                        detail: format!(
                            "文件夹图片 {} 命中 {}（{}人/{}区内）",
                            name.as_deref().unwrap_or(""),
                            behavior,
                            persons,
                            in_zone
                        ),
                        ts_iso: processed_ts.clone(),
                        evidence_key: format!("folder:{iid}:{behavior}"),
                    };
                    if let Ok(Some(eid)) = sink.record(&cand, snapshot_jpeg.as_deref()) {
                        let _ = store.set_folder_image_event(iid, eid);
                    }
                }
            }
            scanned += 1;
        }
        if !cands.is_empty() {
            println!(
                "[ls-folder] cam-{} 本轮 {} 候选，识别 {} 张（上限 {max_per_tick}）",
                cfg.camera_id,
                cands.len(),
                scanned
            );
        }
        let _ = scanned;
        if !sleep_interruptible_wake(
            &handle.stop,
            &handle.wake,
            std::time::Duration::from_secs_f64(interval),
        ) {
            break;
        }
    }
    let _ = store.set_camera_status(cfg.camera_id, "stopped");
    if let Ok(mut m) = st.workers.map.lock() {
        m.remove(&cfg.camera_id);
    }
}

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

    // ---- RA1.3：folder worker + 首发派发 ----

    /// folder_images 表建表（seed_schema 未含；列与唯一索引对齐 Python
    /// store.MIGRATIONS——四元组 (camera_id,path,mtime_ns,size)，文件覆盖即新图）。
    fn seed_folder_images(db: &std::path::Path) {
        let c = rusqlite::Connection::open(db).unwrap();
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS folder_images(\
             id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, path TEXT, \
             name TEXT, mtime_ns INTEGER, size INTEGER, ts TEXT, processed_ts TEXT, \
             width INTEGER, height INTEGER, persons INTEGER DEFAULT 0, in_zone INTEGER DEFAULT 0, \
             ppe_no INTEGER DEFAULT 0, ppe_wearing INTEGER DEFAULT 0, zones TEXT, \
             violations TEXT, key INTEGER DEFAULT 0, violation INTEGER DEFAULT 0, \
             event_id INTEGER, snapshot TEXT, error TEXT);\
             CREATE UNIQUE INDEX IF NOT EXISTS idx_folder_images_uniq \
             ON folder_images(camera_id, path, mtime_ns, size);",
        )
        .unwrap();
    }

    #[test]
    fn add_folder_image_is_idempotent_on_camera_path() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        seed_folder_images(&db);
        let s = ls_store::Store::open(db.to_str().unwrap()).unwrap();
        let id1 = s
            .add_folder_image(
                1,
                "/d/a.jpg",
                Some("a.jpg"),
                111,
                222,
                "2026-11-13T00:00:00",
                "2026-11-13T00:00:00",
                None,
                None,
                0,
                0,
                0,
                0,
                "[]",
                "[]",
                false,
                false,
                None,
                None,
            )
            .unwrap();
        let id2 = s
            .add_folder_image(
                1,
                "/d/a.jpg",
                Some("a.jpg"),
                111,
                222,
                "2026-11-13T00:00:01",
                "2026-11-13T00:00:01",
                None,
                None,
                0,
                0,
                0,
                0,
                "[]",
                "[]",
                false,
                false,
                None,
                None,
            )
            .unwrap();
        assert_eq!(id1, id2, "唯一键冲突返回既有 id");
        let n: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM folder_images", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn folder_worker_registers_images_and_stops() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        // data_dir 与受监视图片目录同级——worker 的 internal_dir=数据目录
        // （截图/视频不入册），受监视目录不能嵌在它下面
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let st = test_state_custom(
            db.to_str().unwrap(),
            "",
            false,
            data.to_str().unwrap(),
            true,
            "",
        );
        seed_schema(&db);
        seed_folder_images(&db);
        // 图片目录：两张图 + 一个非图片扩展名干扰项
        let imgs = tmp.path().join("imgs");
        std::fs::create_dir_all(&imgs).unwrap();
        std::fs::write(imgs.join("a.jpg"), b"jpeg-a").unwrap();
        std::fs::write(imgs.join("b.png"), b"png-b").unwrap();
        std::fs::write(imgs.join("c.txt"), b"not-image").unwrap();
        let mut cfg = {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            let cid = s
                .add_camera_full("cam-folder", imgs.to_str().unwrap(), "folder", "")
                .unwrap();
            WorkerConfig::from_db(&s, cid).unwrap().unwrap()
        };
        assert_eq!(cfg.spec.type_, "folder");
        cfg.max_restarts = 0;
        start_worker(&st, cfg).unwrap();
        // 轮询：两张图入册（≤5s）
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let s = ls_store::Store::open(&st.db_path).unwrap();
            let n: i64 = s
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM folder_images WHERE camera_id=1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            if n >= 2 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "5s 内应登记 2 张");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let s = ls_store::Store::open(&st.db_path).unwrap();
        let bad: i64 = s
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM folder_images WHERE path LIKE '%c.txt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bad, 0, "非图片扩展名不入册");
        // RA2-S3：假 jpg 无法解码 → error 字段如实（persons=0 且 width=0），
        // 不再是无条件 0 占位
        let (persons, err): (i64, Option<String>) = s
            .conn()
            .query_row(
                "SELECT persons, error FROM folder_images LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(persons, 0);
        assert!(err.is_some(), "解码失败应落 error 字段");
        assert!(stop_worker(&st, 1));
    }

    #[tokio::test]
    async fn folder_scan_wake_or_start_contract() {
        use crate::tests::helpers::{body_json, req, test_state};
        use crate::*;
        use axum::http::StatusCode;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        {
            let s = ls_store::Store::open(db.to_str().unwrap()).unwrap();
            let fid = s
                .add_camera_full(
                    "cam-folder",
                    dir.path().join("imgs").to_str().unwrap(),
                    "folder",
                    "",
                )
                .unwrap();
            let _ = fid;
            let _ = s
                .add_camera_full("cam-rtsp", "rtsp://x", "rtsp", "")
                .unwrap();
        }
        let app = build_router(test_state(db.to_str().unwrap()));
        // 非文件夹源 → 400
        let r = req(&app, "POST", "/api/folders/2/scan", None).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "rtsp 源 scan 拒绝");
        // 文件夹源未运行 → started=true（folder worker 启动）
        let r = req(&app, "POST", "/api/folders/1/scan", None).await;
        assert_eq!(r.status(), StatusCode::OK, "RA1.3：scan 摘 503");
        let j = body_json(r).await;
        assert_eq!(j["ok"], true);
        assert_eq!(j["started"], true, "未运行 → 启动");
        // 再次 scan → started=false（wake 已运行 worker）
        let r = req(&app, "POST", "/api/folders/1/scan", None).await;
        let j = body_json(r).await;
        assert_eq!(j["started"], false, "运行中 → wake");
        assert_eq!(j["msg"], "已触发立即扫描");
        // 不存在的相机 → 404
        let r = req(&app, "POST", "/api/folders/99/scan", None).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn dispatch_event_no_channels_is_noop() {
        // 无渠道配置（env 空 + 未注入）→ 0 投递、不建 delivery 行（不崩即契约）
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        seed_schema(&db);
        let st = test_state_custom(
            db.to_str().unwrap(),
            "",
            false,
            tmp.path().to_str().unwrap(),
            true,
            "",
        );
        let store = ls_store::Store::open(&st.db_path).unwrap();
        let eid = store
            .add_event(&ls_store::NewEvent {
                camera_id: 1,
                type_: "person_enter",
                level: 2,
                track_id: 1,
                detail: "x",
                snapshot: None,
                ts: "2026-11-13T00:00:00",
            })
            .unwrap();
        let dev = ls_dispatch::DispatchEvent {
            id: eid,
            camera_id: 1,
            type_: "person_enter".into(),
            level: 2,
            detail: "x".into(),
            ts: "2026-11-13T00:00:00".into(),
            camera_name: "cam".into(),
        };
        let n = st.jobs.dispatch_event(&store, &dev);
        assert_eq!(n, 0, "无渠道配置 → 0 投递");
        let rows: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM alert_deliveries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "无渠道不建投递行");
    }

    #[tokio::test]
    async fn face_compliance_gates() {
        use crate::tests::helpers::{body_json, req, test_state};
        use crate::*;
        use axum::http::StatusCode;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let app = build_router(test_state(db.to_str().unwrap()));
        // 默认关（LS_FACE_ENABLED 未设）→ 403 合规门（在任何 CompreFace 调用前）
        let r = req(&app, "GET", "/api/face/persons", None).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let j = body_json(r).await;
        assert!(j["detail"].as_str().unwrap().contains("未开启"), "合规文案");
        // /face/status 只读不受门控：enabled=false
        let j = body_json(req(&app, "GET", "/api/face/status", None).await).await;
        assert_eq!(j["enabled"], false);
        assert!(j.get("configured").is_some() && j.get("threshold").is_some());
    }

    #[test]
    fn genai_parse_review_strict_and_status_default() {
        // 严格 JSON：verdict 白名单 / reason 截断 / confidence 夹取 / 围栏剥离
        let ok = crate::genai::parse_review(
            "```json\n{\"verdict\":\"likely_false\",\"reason\":\"无人\",\"confidence\":0.8}\n```",
        )
        .unwrap();
        assert_eq!(ok["verdict"], "likely_false");
        assert_eq!(ok["reason"], "无人");
        assert_eq!(ok["confidence"], 0.8);
        assert!(crate::genai::parse_review("not json").is_err());
        assert!(crate::genai::parse_review("{\"verdict\":\"bogus\"}").is_err());
        let clamped =
            crate::genai::parse_review("{\"verdict\":\"uncertain\",\"confidence\":9.9}").unwrap();
        assert_eq!(clamped["confidence"], 1.0);
        // 默认 provider=none：status 九字段 + 零开销（不入队、不建线程）
        let g = crate::genai::GenAI::from_env();
        let s = g.status();
        assert_eq!(s["provider"], "none");
        assert!(s.get("queue_depth").is_some() && s.get("max_inflight").is_some());
        assert!(!g.maybe_enqueue(1, Some("/x.jpg"), 3, ":memory:"));
    }

    #[test]
    fn set_ai_review_writes_marker_without_status_change() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        seed_schema(&db);
        let s = ls_store::Store::open(db.to_str().unwrap()).unwrap();
        let eid = s
            .add_event(&ls_store::NewEvent {
                camera_id: 1,
                type_: "person_enter",
                level: 2,
                track_id: 1,
                detail: "x",
                snapshot: None,
                ts: "2026-11-13T00:00:00",
            })
            .unwrap();
        s.set_ai_review(
            eid,
            "{\"verdict\":\"likely_true\"}",
            "2026-11-13T00:00:01",
            "llava",
        )
        .unwrap();
        let (review, status): (Option<String>, String) = s
            .conn()
            .query_row(
                "SELECT ai_review, status FROM events WHERE id=?",
                [eid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(review.unwrap().contains("likely_true"));
        assert_eq!(status, "new", "AI 复核绝不改 status");
    }

    #[tokio::test]
    async fn plugins_s7_contract() {
        use crate::tests::helpers::{body_json, req, test_state_custom};
        use crate::*;
        use axum::http::StatusCode;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let data = dir.path().join("data");
        let plug = data.join("plugins");
        let good = plug.join("demo-plug");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(
            good.join("plugin.json"),
            r#"{"id":"demo-plug","name":"Demo","version":"1.0","description":"x",
                "command":["{python}","--version"],"permissions":["read"],"env":{"FOO":"bar"}}"#,
        )
        .unwrap();
        let bad = plug.join("bad-plug");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join("plugin.json"),
            r#"{"id":"../escape","command":[]}"#,
        )
        .unwrap();
        seed_schema(&db);
        let app = build_router(test_state_custom(
            db.to_str().unwrap(),
            "",
            false,
            data.to_str().unwrap(),
            true,
            "",
        ));
        // scan：好插件 + 坏清单 error 项
        let j = body_json(req(&app, "GET", "/api/plugins", None).await).await;
        let arr = j["plugins"].as_array().unwrap();
        assert!(arr
            .iter()
            .any(|p| p["id"] == "demo-plug" && p["name"] == "Demo"));
        assert!(
            arr.iter()
                .any(|p| p["id"] == "bad-plug" && p["error"].is_string()),
            "坏清单生成 error 项"
        );
        // enable → {id,enabled:true}
        let j = body_json(req(&app, "POST", "/api/plugins/demo-plug/enable", None).await).await;
        assert_eq!(j["enabled"], true);
        // 不存在的插件 → 404
        let r = req(&app, "POST", "/api/plugins/nope/enable", None).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        // logs（无日志文件 → 空串，不崩）
        let j = body_json(req(&app, "GET", "/api/plugins/demo-plug/logs", None).await).await;
        assert_eq!(j["id"], "demo-plug");
    }

    #[tokio::test]
    async fn status_endpoints_ra2_shape() {
        use crate::tests::helpers::{body_json, req, test_state};
        use crate::*;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let app = build_router(test_state(db.to_str().unwrap()));
        // S9：model/status 形状对齐 Python（classes/ppe/backend/imgsz/conf/analysis_fps）
        let j = body_json(req(&app, "GET", "/api/model/status", None).await).await;
        assert!(j.get("backend").is_some() && j.get("imgsz").is_some());
        assert!(j.get("classes").is_some() && j["classes"].get("names").is_some());
        assert!(j.get("ppe").is_some() && j["ppe"].get("conf").is_some());
        assert!(j.get("face_enabled").is_some());
        // S2：mqtt/status 九字段（默认关）
        let j = body_json(req(&app, "GET", "/api/mqtt/status", None).await).await;
        assert_eq!(j["enabled"], false);
        assert!(
            j.get("published").is_some()
                && j.get("queue_depth").is_some()
                && j.get("host").is_some()
                && j.get("topic_prefix").is_some()
        );
        // genai/plugins 现状如实占位（S5/S7 待接）
        let j = body_json(req(&app, "GET", "/api/genai/status", None).await).await;
        assert_eq!(j["provider"], "none");
        let j = body_json(req(&app, "GET", "/api/plugins", None).await).await;
        assert!(j["plugins"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn system_contract_today_and_alerts() {
        use crate::tests::helpers::{body_json, req, test_state};
        use crate::*;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        {
            let s = ls_store::Store::open(db.to_str().unwrap()).unwrap();
            s.add_camera_full("cam-1", "rtsp://x", "rtsp", "").unwrap();
            // 两条今日事件（一条 new 一条已处置）+ 一条投递行——ts 用 SQL localtime
            // 生成，与 today_event_counts 的匹配口径同源
            for status in ["new", "acked"] {
                s.conn()
                    .execute(
                        "INSERT INTO events(ts,camera_id,type,level,track_id,detail,status) \
                         VALUES(date('now','localtime')||'T08:00:00',1,'person_enter',2,1,'x',?)",
                        [status],
                    )
                    .unwrap();
            }
            s.add_delivery(1, "webhook").unwrap();
        }
        let app = build_router(test_state(db.to_str().unwrap()));
        let r = req(&app, "GET", "/api/system", None).await;
        let j = body_json(r).await;
        assert_eq!(j["today_counts"]["2"], 2, "今日按级别计数");
        assert_eq!(j["unhandled_today"], 1, "今日未处置（new）");
        assert_eq!(j["alerts"]["pending"], 1, "投递行计数");
        assert_eq!(j["backend"], "rust");
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
