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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::handlers::events::event_json;

mod cascade;
mod episodes;
use crate::performance::{FrameTiming, PerformanceWindow};
use crate::state::Shared;
use cascade::{finish_cascade_events, keep_geometry_local, submit_cascade_review, CascadeReview};

/// 事件去重窗口容量（Python sidecar_events.SEEEN_MAX 同值）。
const DEDUP_MAX: usize = 4096;

static SNAP_SEQ: AtomicU64 = AtomicU64::new(0);
static PREVIEW_SEQ: AtomicU64 = AtomicU64::new(0);

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
    legacy_review: bool,
}

struct SavedSnapshot {
    path: String,
    sha256: String,
}

fn detection_overlay(
    width: u32,
    height: u32,
    bbox: Option<[f32; 4]>,
    zone: Option<&ls_analyzer::pipeline::ZoneSpec>,
    visual: Option<serde_json::Value>,
) -> Option<String> {
    (bbox.is_some() || zone.is_some()).then(|| {
        serde_json::json!({
            "width": width, "height": height, "box": bbox, "zone": zone.map(|zone| &zone.points),
            "zone_policy": zone.map(|zone| serde_json::json!({
                "id": zone.id, "name": zone.name, "spatial": zone.spatial,
            })),
            "visual": visual,
        })
        .to_string()
    })
}

impl EventSink {
    pub fn new(st: Shared) -> Self {
        Self {
            paths: SinkPaths::from_data_dir(&st.data_dir),
            st,
            legacy_review: true,
            dedup: Mutex::new(Dedup {
                seen: HashSet::new(),
                order: VecDeque::new(),
            }),
        }
    }

    fn use_cascade(mut self, cascade: bool) -> Self {
        self.legacy_review = !cascade;
        self
    }

    #[cfg(test)]
    fn with_paths(st: Shared, paths: SinkPaths) -> Self {
        Self {
            st,
            paths,
            legacy_review: true,
            dedup: Mutex::new(Dedup {
                seen: HashSet::new(),
                order: VecDeque::new(),
            }),
        }
    }

    fn dedup_key(cand: &ls_analyzer::pipeline::Candidate) -> String {
        format!("{}|{}|{}", cand.evidence_key, cand.behavior, cand.track_id)
    }

    fn save_snapshot(&self, camera: i64, ts: &str, jpeg: &[u8]) -> Result<SavedSnapshot, String> {
        use image::ImageEncoder;
        use sha2::{Digest, Sha256};
        let image = ls_jobs::media::decode_image(jpeg)
            .ok_or("无法解码事件截图")?
            .into_rgb8();
        let mut webp = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|error| format!("事件截图 WebP 编码失败: {error}"))?;
        std::fs::create_dir_all(&self.paths.snapshots)
            .map_err(|e| format!("创建截图目录失败: {e}"))?;
        let seq = SNAP_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "ev_{camera}_{}_{}{:04}.webp",
            ts.replace([':', '-', '.'], ""),
            std::process::id(),
            seq
        );
        let path = self.paths.snapshots.join(name);
        if let Err(error) = std::fs::write(&path, &webp) {
            let _ = std::fs::remove_file(&path);
            return Err(format!("截图写入失败: {error}"));
        }
        Ok(SavedSnapshot {
            path: path.to_string_lossy().into_owned(),
            sha256: format!("{:x}", Sha256::digest(webp)),
        })
    }

    /// 候选 → 截图 → 入库 → WS 广播。返回 `Ok(Some(eid))`=已落地（供 finalize
    /// 关联），`Ok(None)`=去重命中（Python 桥静默丢弃同口径）。
    pub fn record(
        &self,
        cand: &ls_analyzer::pipeline::Candidate,
        snapshot_jpeg: Option<&[u8]>,
    ) -> Result<Option<i64>, String> {
        self.record_ext(cand, snapshot_jpeg, true)
    }

    /// UX2 cascade：`dispatch=false` 时事件入库 + WS 广播，但 **跳过 MQTT 与
    /// 首发派发**——待 VLM 复核（肯定 → `deliver_deferred` 补投递；否定且
    /// on_negative=suppress → `set_event_vlm_suppressed` 标记，永不投递）。
    pub fn record_ext(
        &self,
        cand: &ls_analyzer::pipeline::Candidate,
        snapshot_jpeg: Option<&[u8]>,
        dispatch: bool,
    ) -> Result<Option<i64>, String> {
        self.record_located(cand, snapshot_jpeg, dispatch, None, None, None)
    }

    fn record_located(
        &self,
        cand: &ls_analyzer::pipeline::Candidate,
        snapshot_jpeg: Option<&[u8]>,
        dispatch: bool,
        bbox: Option<[f32; 4]>,
        overlay: Option<&str>,
        episode_id: Option<i64>,
    ) -> Result<Option<i64>, String> {
        let st = &self.st;
        let store = ls_store::Store::open(&st.db_path).map_err(|e| e.to_string())?;
        let key = Self::dedup_key(cand);
        {
            let mut d = self.dedup.lock().map_err(|_| "dedup poison")?;
            if d.seen.contains(&key) {
                return Ok(None);
            }
            d.seen.insert(key.clone());
            d.order.push_back(key.clone());
            if d.order.len() > DEDUP_MAX {
                if let Some(old) = d.order.pop_front() {
                    d.seen.remove(&old);
                }
            }
        }
        let snapshot = snapshot_jpeg.and_then(|jpeg| {
            self.save_snapshot(cand.camera_id, &cand.ts_iso, jpeg)
                .map_err(|error| {
                    crate::logger::warn(&format!("[ls-worker] cam-{} {error}", cand.camera_id));
                })
                .ok()
        });
        let path = snapshot.as_ref().map(|image| image.path.as_str());
        let ev = ls_store::NewEvent {
            camera_id: cand.camera_id,
            type_: &cand.behavior,
            level: cand.level,
            track_id: cand.track_id,
            detail: &cand.detail,
            snapshot: path,
            ts: &cand.ts_iso,
        };
        let evidence = ls_store::EventEvidence {
            bbox: bbox
                .filter(|_| snapshot.is_some() && crate::spatial::spatial_rule(&cand.behavior)),
            detections_json: overlay.filter(|_| snapshot.is_some()),
            snapshot_sha256: snapshot.as_ref().map(|image| image.sha256.as_str()),
            episode_id,
        };
        let eid = store
            .add_event_with_evidence(&ev, evidence)
            .map_err(|error| {
                if let Some(path) = path {
                    let _ = std::fs::remove_file(path);
                }
                if let Ok(mut dedup) = self.dedup.lock() {
                    dedup.seen.remove(&key);
                    dedup.order.retain(|reserved| reserved != &key);
                }
                error.to_string()
            })?;
        // RA2-S5：AI 复核入队（默认 provider=none 零开销；只写 ai_review 标记）
        if self.legacy_review && cand.behavior != "alert_resolved" {
            let _ = st.genai.maybe_enqueue(eid, path, cand.level, &st.db_path);
        }
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
        if dispatch {
            self.deliver_deferred(eid, cand, &cam_name, snapshot.is_some(), path);
        }
        Ok(Some(eid))
    }

    /// 事件外发（MQTT 总线 + 8 渠道首发派发）。record 常规路径立即调用；
    /// cascade 复核肯定时延迟调用（on_negative=keep 也可先派发后回填判定）。
    fn deliver_deferred(
        &self,
        eid: i64,
        cand: &ls_analyzer::pipeline::Candidate,
        cam_name: &str,
        has_snapshot: bool,
        snapshot_path: Option<&str>,
    ) {
        self.deliver(eid, cand, cam_name, has_snapshot, snapshot_path, false);
    }

    fn deliver(
        &self,
        eid: i64,
        cand: &ls_analyzer::pipeline::Candidate,
        cam_name: &str,
        has_snapshot: bool,
        snapshot_path: Option<&str>,
        episode_upgrade: bool,
    ) {
        let st = &self.st;
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
                camera_name: Some(cam_name),
                type_: &cand.behavior,
                level: cand.level,
                track_id: cand.track_id,
                detail: &cand.detail,
                zones: &[],
                has_snapshot,
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
            camera_name: cam_name.to_string(),
            track_id: cand.track_id,
            snapshot_path: snapshot_path.map(String::from),
        };
        let jobs = Arc::clone(&st.jobs);
        let db_path = st.db_path.clone();
        let _ = std::thread::Builder::new()
            .name(format!("ls-dispatch-{eid}"))
            .spawn(move || {
                if let Ok(s) = ls_store::Store::open(&db_path) {
                    let submitted = if episode_upgrade {
                        jobs.dispatch_episode_upgrade(&s, &dev)
                    } else {
                        jobs.dispatch_event(&s, &dev)
                    };
                    if submitted > 0 {
                        println!("[ls-dispatch] event#{eid} 首发派发 {submitted} 渠道");
                    }
                }
            });
    }

    /// cascade 复核线程的放行入口（cam_name 与 snapshot 现查，确保延迟派发保留事件截图）。
    pub(crate) fn deliver_deferred_pub(
        &self,
        eid: i64,
        cand: &ls_analyzer::pipeline::Candidate,
        has_snapshot: bool,
    ) {
        let (cam_name, snap_path) = ls_store::Store::open(&self.st.db_path)
            .ok()
            .map(|s| {
                let cn = s
                    .camera(cand.camera_id)
                    .ok()
                    .flatten()
                    .map(|c| c.name)
                    .unwrap_or_default();
                let sp = s
                    .event_by_id(eid)
                    .ok()
                    .flatten()
                    .and_then(|e| e.snapshot)
                    .filter(|p| !p.trim().is_empty());
                (cn, sp)
            })
            .unwrap_or_default();
        self.deliver_deferred(
            eid,
            cand,
            &cam_name,
            has_snapshot || snap_path.is_some(),
            snap_path.as_deref(),
        );
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
/// 连续收到画面达到该时间后结束断流事件，并重置重连计数。
const RESTART_ALIVE_RESET_S: u64 = 5;
const OUTAGE_DEBOUNCE: Duration = Duration::from_secs(5);
const COMPLIANCE_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
/// 默认重启上限（Python Manager.MAX_RESTARTS 同量级）。
const DEFAULT_MAX_RESTARTS: u32 = 4;

/// 每路 worker 句柄（注册表持有；stop/ffmpeg 供 stop 路径强杀）。
pub struct WorkerHandle {
    pub stop: Arc<AtomicBool>,
    pub ffmpeg: ls_ingest::ChildSlot,
    /// 最新抽帧 JPEG（camera_snapshot 用；evidence stride 命中时更新）
    pub latest_jpeg: Mutex<Option<Snapshot>>,
    /// RA1.3：folder worker 立即扫描唤醒（POST /folders/{cid}/scan）
    pub wake: Arc<AtomicBool>,
    /// LS-050：Motion Gate 统计（跳过帧数 / 总帧数），供 /api/system 读取
    pub motion_skipped: Arc<AtomicU64>,
    pub motion_total: Arc<AtomicU64>,
    pub(crate) performance: Mutex<PerformanceWindow>,
    ppe_loaded: OnceLock<bool>,
    pub(crate) applied_policy_revision: AtomicU64,
}

#[derive(Clone)]
pub struct Snapshot {
    pub jpeg: axum::body::Bytes,
    pub captured_at_ms: u128,
    pub sequence: u64,
}

/// 全局 worker 注册表（AppState.workers）。
#[derive(Default)]
pub struct Workers {
    map: Mutex<HashMap<i64, Arc<WorkerHandle>>>,
    pub(crate) policy_revision: AtomicU64,
}

impl Workers {
    pub(crate) fn notify_policy_changed(&self) -> u64 {
        let revision = self.policy_revision.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(map) = self.map.lock() {
            for worker in map.values() {
                worker.wake.store(true, Ordering::Release);
            }
        }
        revision
    }

    pub(crate) fn ppe_runtime_counts(&self) -> (usize, usize) {
        self.map.lock().map_or((0, 0), |map| {
            map.values()
                .filter_map(|worker| worker.ppe_loaded.get())
                .fold((0, 0), |(loaded, failed), ready| {
                    (loaded + usize::from(*ready), failed + usize::from(!ready))
                })
        })
    }

    pub fn get(&self, cid: i64) -> Option<Arc<WorkerHandle>> {
        self.map.lock().ok()?.get(&cid).cloned()
    }

    /// LS-050：所有运行中相机的 Motion Gate 汇总统计（总帧/跳过帧）。
    pub fn motion_stats(&self) -> (u64, u64) {
        let Ok(map) = self.map.lock() else {
            return (0, 0);
        };
        map.values().fold((0u64, 0u64), |(t, s), h| {
            (
                t + h.motion_total.load(Ordering::Relaxed),
                s + h.motion_skipped.load(Ordering::Relaxed),
            )
        })
    }

    pub(crate) fn performance_summary(&self) -> Vec<serde_json::Value> {
        let Ok(map) = self.map.lock() else {
            return Vec::new();
        };
        let mut entries: Vec<_> = map
            .iter()
            .map(|(&id, handle)| (id, Arc::clone(handle)))
            .collect();
        drop(map);
        entries.sort_unstable_by_key(|(id, _)| *id);
        entries
            .into_iter()
            .filter_map(|(id, handle)| {
                let samples = handle.performance.lock().ok()?.clone();
                if samples.frames() == 0 {
                    return None;
                }
                let mut summary = samples.summary();
                summary["camera_id"] = serde_json::json!(id);
                Some(summary)
            })
            .collect()
    }

    fn insert(&self, cid: i64, handle: Arc<WorkerHandle>) -> bool {
        let Ok(mut map) = self.map.lock() else {
            return false;
        };
        let std::collections::hash_map::Entry::Vacant(entry) = map.entry(cid) else {
            return false;
        };
        entry.insert(handle);
        true
    }

    fn with_current<T>(
        &self,
        cid: i64,
        handle: &Arc<WorkerHandle>,
        action: impl FnOnce() -> T,
    ) -> Option<T> {
        let map = self.map.lock().ok()?;
        Arc::ptr_eq(map.get(&cid)?, handle).then(action)
    }

    fn remove_and_stop(
        &self,
        store: Option<&ls_store::Store>,
        cid: i64,
        expected: Option<&Arc<WorkerHandle>>,
        status: &str,
    ) -> Option<Arc<WorkerHandle>> {
        let mut map = self.map.lock().ok()?;
        let handle = map.get(&cid)?;
        if expected.is_some_and(|expected| !Arc::ptr_eq(handle, expected)) {
            return None;
        }
        handle.stop.store(true, Ordering::SeqCst);
        if let Some(store) = store {
            let _ = store.set_camera_status(cid, status);
        }
        let h = map.remove(&cid)?;
        drop(map);
        ls_ingest::Ingest::kill_slot(&h.ffmpeg);
        Some(h)
    }
}

/// 单路 worker 配置（RA1.2：从 DB 组装，不再读进程 env——make_detector 已参数化）。
pub struct WorkerConfig {
    pub camera_id: i64,
    pub spec: ls_analyzer::pipeline::CameraSpec,
    policy: CameraPolicy,
    /// cameras.analyzer 列原值（python/rust/vlm）——RA5 模型模块化：
    /// "vlm" = 该路强制走外部联网模型判定
    pub analyzer: String,
    pub detector: ls_analyzer::detector::DetectorConfig,
    /// RA2-S1：PPE 级联配置（None = 未配置模型/下载未完成 → 仅人员检测）
    pub ppe: Option<ls_analyzer::ppe::PpeConfig>,
    pub evidence_fps: f64,
    pub max_restarts: u32,
    pub analysis_source: String,
    pub analysis_shared: bool,
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

#[derive(Clone)]
struct CameraPolicy {
    zones: Vec<ls_analyzer::pipeline::ZoneSpec>,
    rules: Vec<ls_analyzer::pipeline::RuleSpec>,
    calendar: HashMap<String, String>,
    spatial: crate::spatial::SpatialPolicy,
    rule_grace: HashMap<i64, ls_analyzer::episode::GraceWindow>,
}

fn read_camera_policy(s: &ls_store::Store, cid: i64) -> Result<CameraPolicy, String> {
    let tx = s
        .conn()
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let zones = s
        .zones_spatial(Some(cid))
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|zone| {
            parse_polygon(&zone.polygon).map(|points| ls_analyzer::pipeline::ZoneSpec {
                id: zone.id,
                camera_id: cid,
                name: zone.name,
                points,
                spatial: zone.spatial,
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
    let spatial = crate::spatial::SpatialPolicy::read(s, cid).map_err(|error| error.to_string())?;
    let rule_grace = s
        .camera_rule_grace(cid)
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, seconds, ratio)| {
            (
                id,
                ls_analyzer::episode::GraceWindow {
                    seconds,
                    warning_percent: (ratio.clamp(0.01, 0.99) * 100.0).round() as u8,
                },
            )
        })
        .collect();
    tx.commit().map_err(|error| error.to_string())?;
    Ok(CameraPolicy {
        zones,
        rules,
        calendar,
        spatial,
        rule_grace,
    })
}

fn changed_camera_policy(
    st: &Shared,
    store: &ls_store::Store,
    cid: i64,
    handle: &WorkerHandle,
) -> Option<(u64, CameraPolicy)> {
    let revision = st.workers.policy_revision.load(Ordering::Acquire);
    if handle.applied_policy_revision.load(Ordering::Relaxed) == revision {
        return None;
    }
    read_camera_policy(store, cid)
        .ok()
        .map(|policy| (revision, policy))
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
        let policy = read_camera_policy(s, cid)?;
        // 模型路径：进程 env LS_MODEL_PATH（未设 = NullDetector，仅人员检测为空）。
        // RA2 接模型管理（assets.lock/网页端许可确认）后改从 settings 组装。
        let mut detector = ls_analyzer::detector::DetectorConfig::default();
        if let Some(confidence) = s
            .setting_get("detector.conf")
            .ok()
            .flatten()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| (0.05..=0.95).contains(value))
        {
            detector.conf = confidence;
        }
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
        // 0.3.0 修复：安装版快捷方式不经 LabSentinel.bat（无 env 注入）——发行包
        // 自带 models/yolo11n.onnx 时按安装根兜底，避免新装机器静默退化为 NullDetector。
        let bundled_model = {
            let p = crate::deploy::default_install_root().join("models/yolo11n.onnx");
            p.is_file().then(|| p.to_string_lossy().into_owned())
        };
        detector.model_path = model_active
            .or(model_env)
            .or(bundled_model)
            .map(PathBuf::from);
        detector.coco_context = detector
            .model_path
            .as_deref()
            .and_then(ls_analyzer::detector::anchor_model_file)
            .zip(ls_analyzer::ppe::anchored_child(
                &crate::deploy::default_install_root().join("models"),
                "yolo11n.onnx",
            ))
            .is_some_and(|(active, bundled)| active == bundled);
        // UX（2026-11-21）：推理设备偏好（app_settings runtime.ep；默认 dml=GPU，
        // DML 初始化失败 ls-infer 自动回落 CPU——任意厂商 GPU 鲁棒）
        detector.use_gpu = crate::detect::runtime_ep_pref(s) == "dml";
        // RA2-S1 / LS-041：PPE 级联优先级与 ppe_status 对齐：
        // LS_PPE_MODEL 环境变量 → app_settings["ppe_model"]（网页上传/下载）→ 随包 models/yolo11n_ppe.onnx。
        let selection = resolve_ppe_model(s);
        let ppe = selection
            .path
            .filter(|_| selection.enabled)
            .map(|model_path| {
                let dir = crate::deploy::default_install_root().join("models");
                let stock = bundled_ppe_model(&model_path);
                let (violation_conf, wearing_conf) = ppe_thresholds(s, stock);
                ls_analyzer::ppe::PpeConfig {
                    model_path,
                    close_mask: close_mask_enabled(s).then(close_mask_assets).flatten(),
                    output_format: s.setting_get("ppe.output_format").ok().flatten().and_then(
                        |value| ls_infer::yolo::OutputFormat::parse(&value).ok().flatten(),
                    ),
                    use_gpu: detector.use_gpu,
                    violation_conf,
                    wearing_conf,
                    coat_confirmation: stock.then(|| {
                        ls_analyzer::ppe::anchored_child(
                            &dir,
                            ls_analyzer::ppe::COAT_CONFIRMATION_FILE,
                        )
                        .filter(|path| path.is_file())
                        .unwrap_or_default()
                    }),
                    ..Default::default()
                }
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
            policy,
            detector,
            ppe,
            evidence_fps: efps,
            max_restarts: DEFAULT_MAX_RESTARTS,
            analysis_source: c.analysis_source,
            analysis_shared: c.analysis_shared,
        }))
    }
}

pub(crate) struct PpeModelSelection {
    pub path: Option<PathBuf>,
    pub source: &'static str,
    pub bundled: bool,
    pub enabled: bool,
    pub warning: Option<String>,
}

pub(crate) fn resolve_ppe_model(store: &ls_store::Store) -> PpeModelSelection {
    let bundled = crate::deploy::default_install_root().join("models/yolo11n_ppe.onnx");
    let custom = store
        .setting_get("ppe_model")
        .ok()
        .flatten()
        .filter(|path| !path.trim().is_empty());
    let env = std::env::var("LS_PPE_MODEL")
        .ok()
        .filter(|path| !path.trim().is_empty());
    let mut selection = PpeModelSelection {
        path: None,
        source: "none",
        bundled: bundled.is_file(),
        enabled: store
            .setting_get("ppe.enabled")
            .ok()
            .flatten()
            .is_none_or(|value| !matches!(value.trim(), "0" | "false" | "off" | "no")),
        warning: None,
    };
    let candidates = [
        (
            "env",
            "环境变量指定的 PPE 模型",
            env.as_deref().map(|path| PathBuf::from(path.trim())),
        ),
        (
            "custom",
            "自定义 PPE 模型",
            custom.as_deref().map(|path| PathBuf::from(path.trim())),
        ),
        (
            "bundled",
            "随包 PPE 模型",
            selection.bundled.then_some(bundled),
        ),
    ];
    let compatibility = ls_analyzer::ppe::PpeConfig::default();
    for (source, label, candidate) in candidates {
        let Some(candidate) = candidate else { continue };
        let valid = ls_analyzer::detector::anchor_model_file(&candidate).filter(|path| {
            compatibility.supports_labels(&ls_analyzer::ppe::load_class_names(path))
        });
        if let Some(path) = valid {
            selection.path = Some(path);
            selection.source = source;
            if source == "env" && custom.is_some() {
                selection.warning =
                    Some("当前由环境变量 LS_PPE_MODEL 覆盖，网页上传的自定义模型暂未生效".into());
            }
            break;
        }
        selection
            .warning
            .get_or_insert_with(|| format!("{label}不可用或缺少匹配的类名表，已尝试备用模型"));
    }
    selection
}

pub(crate) fn bundled_ppe_model(path: &Path) -> bool {
    let dir = crate::deploy::default_install_root().join("models");
    ls_analyzer::ppe::anchored_child(&dir, "yolo11n_ppe.onnx")
        .is_some_and(|bundled| path.canonicalize().is_ok_and(|active| active == bundled))
}

pub(crate) fn ppe_thresholds(store: &ls_store::Store, bundled: bool) -> (f32, f32) {
    let threshold = |key, env, default| {
        let parse = |v: String| v.trim().parse::<f32>().ok().filter(|v| v.is_finite());
        store
            .setting_get(key)
            .ok()
            .flatten()
            .and_then(parse)
            .or_else(|| std::env::var(env).ok().and_then(parse))
            .map(|v| v.clamp(0.05, 0.95))
            .unwrap_or(default)
    };
    let (violation, wearing) = if bundled { (0.50, 0.40) } else { (0.30, 0.20) };
    (
        threshold("ppe.violation_conf", "LS_PPE_CONF", violation),
        threshold("ppe.wearing_conf", "LS_PPE_WEARING_CONF", wearing),
    )
}

pub(crate) fn close_mask_enabled(store: &ls_store::Store) -> bool {
    store
        .setting_get("ppe.close_mask_enabled")
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
}

pub(crate) fn close_mask_assets() -> Option<ls_analyzer::ppe::CloseMaskAssets> {
    let dir = crate::deploy::default_install_root().join("models");
    ls_analyzer::ppe::CloseMaskAssets::from_dir(&dir)
}

/// 启动 worker（handler 入口）：组装配置 → 注册句柄 → 起分析线程。
/// folder 源走扫描登记线程（RA1.3），rtsp/file/usb 走 ffmpeg 帧循环。
pub fn start_worker(st: &Shared, cfg: WorkerConfig) -> Result<(), String> {
    let cid = cfg.camera_id;
    let handle = Arc::new(WorkerHandle {
        stop: Arc::new(AtomicBool::new(false)),
        ffmpeg: Arc::new(Mutex::new(None)),
        latest_jpeg: Mutex::new(None),
        wake: Arc::new(AtomicBool::new(false)),
        motion_skipped: Arc::new(AtomicU64::new(0)),
        motion_total: Arc::new(AtomicU64::new(0)),
        performance: Mutex::new(PerformanceWindow::default()),
        ppe_loaded: OnceLock::new(),
        applied_policy_revision: AtomicU64::new(u64::MAX),
    });
    if !st.workers.insert(cid, Arc::clone(&handle)) {
        return Err("已在运行".into());
    }
    if cfg.spec.type_ == "rtsp" {
        st.go2rtc.refresh();
    }
    let st2 = Arc::clone(st);
    let worker_handle = Arc::clone(&handle);
    let is_folder = cfg.spec.type_ == "folder";
    let spawned = if is_folder {
        std::thread::Builder::new()
            .name(format!("ls-folder-{cid}"))
            .spawn(move || folder_loop(st2, cfg, worker_handle))
    } else {
        std::thread::Builder::new()
            .name(format!("ls-worker-{cid}"))
            .spawn(move || worker_loop(st2, cfg, worker_handle))
    };
    match spawned {
        Ok(_) => Ok(()),
        Err(e) => {
            let store = ls_store::Store::open(&st.db_path).ok();
            st.workers
                .remove_and_stop(store.as_ref(), cid, Some(&handle), "error");
            Err(format!("起 worker 线程失败: {e}"))
        }
    }
}

/// 停止 worker（幂等）：置停标志 + kill ffmpeg slot + 状态写 stopped。
/// 返回 False = 该路本来就没在运行。
pub fn stop_worker(st: &Shared, cid: i64) -> bool {
    let store = ls_store::Store::open(&st.db_path).ok();
    st.workers
        .remove_and_stop(store.as_ref(), cid, None, "stopped")
        .is_some()
}

/// UX2：软重启单路（运行中才重启；配置组载失败保持停止并写 error）。
/// 用于 analyzer/全局检测模式变更后的即时生效（对齐 Python 重启语义）。
pub fn restart_camera_if_running(st: &Shared, store: &ls_store::Store, cid: i64) -> bool {
    let was_running = store
        .camera(cid)
        .ok()
        .flatten()
        .map(|c| matches!(c.status.as_str(), "starting" | "running" | "reconnecting"))
        .unwrap_or(false);
    if !was_running {
        return false;
    }
    stop_worker(st, cid);
    match WorkerConfig::from_db(store, cid) {
        Ok(Some(cfg)) => match start_worker(st, cfg) {
            Ok(()) => true,
            Err(e) => {
                crate::logger::error(&format!("[ls-worker] cam-{cid} 软重启失败: {e}"));
                false
            }
        },
        _ => {
            crate::logger::error(&format!("[ls-worker] cam-{cid} 软重启配置载入失败"));
            if let Ok(map) = st.workers.map.lock() {
                if !map.contains_key(&cid) {
                    let _ = store.set_camera_status(cid, "error");
                }
            }
            false
        }
    }
}

/// UX4（2026-11-20）：优雅关停第一步——停掉全部运行中 worker（ffmpeg 子进程
/// 一并强杀解除阻塞读）。返回停掉的路数。
pub fn stop_all_workers(st: &Shared) -> usize {
    let ids: Vec<i64> = {
        let Ok(m) = st.workers.map.lock() else {
            return 0;
        };
        m.keys().copied().collect()
    };
    let store = ls_store::Store::open(&st.db_path).ok();
    ids.into_iter()
        .filter(|cid| {
            st.workers
                .remove_and_stop(store.as_ref(), *cid, None, "stopped")
                .is_some()
        })
        .count()
}

/// UX（2026-11-21）：推理设备（runtime.ep）变更后重启**全部**运行中相机
/// （EP 影响所有模型加载；返回重启路数）。
pub fn restart_all_running(st: &Shared, store: &ls_store::Store) -> usize {
    let Ok(cams) = store.cameras() else {
        return 0;
    };
    cams.iter()
        .filter(|c| c.status == "running")
        .map(|c| c.id)
        .filter(|cid| restart_camera_if_running(st, store, *cid))
        .count()
}

/// UX2：全局检测模式变更后，重启所有「跟随全局」（analyzer='default'/空）的
/// 运行中相机；显式覆盖（rust/vlm/cascade）不受全局切换影响。返回重启路数。
pub fn restart_following_global(st: &Shared, store: &ls_store::Store) -> usize {
    let Ok(cams) = store.cameras() else {
        return 0;
    };
    let running: Vec<i64> = cams
        .iter()
        .filter(|c| c.status == "running" && matches!(c.analyzer.as_str(), "default" | ""))
        .map(|c| c.id)
        .collect();
    running
        .iter()
        .filter(|cid| restart_camera_if_running(st, store, **cid))
        .count()
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

#[derive(Default)]
struct CameraReconnect {
    attempts: u32,
    failed_since: Option<Instant>,
    good_since: Option<Instant>,
}

impl CameraReconnect {
    fn reset_after_good_frames(&mut self, now: Instant) -> bool {
        let first = *self.good_since.get_or_insert(now);
        if now.saturating_duration_since(first) < Duration::from_secs(RESTART_ALIVE_RESET_S) {
            return false;
        }
        self.attempts = 0;
        self.failed_since = None;
        true
    }

    fn failed(&mut self, now: Instant, max: u32) -> (bool, bool) {
        self.good_since = None;
        self.attempts = self.attempts.saturating_add(1);
        let first = *self.failed_since.get_or_insert(now);
        let retry = self.attempts <= max;
        (
            retry,
            !retry || now.saturating_duration_since(first) >= OUTAGE_DEBOUNCE,
        )
    }
}

fn worker_detector(
    st: &Shared,
    store: &ls_store::Store,
    cfg: &WorkerConfig,
    handle: &Arc<WorkerHandle>,
    api_only: bool,
) -> Option<Box<dyn ls_analyzer::pipeline::Detector>> {
    if st.testing && cfg.detector.model_path.is_none() {
        return Some(Box::new(ls_analyzer::pipeline::NullDetector));
    }
    match ls_analyzer::detector::load_detector(&cfg.detector) {
        Ok(detector) => Some(detector),
        Err(_) if api_only => Some(Box::new(ls_analyzer::pipeline::NullDetector)),
        Err(error) => {
            crate::logger::error(&format!(
                "[ls-worker] cam-{} 人员模型加载失败: {error}",
                cfg.camera_id
            ));
            if cfg.spec.type_ == "file" && !cfg.spec.file_loop {
                let _ = st.workers.with_current(cfg.camera_id, handle, || {
                    store.add_check_event(
                        cfg.camera_id,
                        "video_check",
                        "视频检查失败。人员模型无法加载，请检查所选模型与诊断日志。",
                        &crate::util::iso_now(),
                        None,
                    )
                });
            }
            st.workers
                .remove_and_stop(Some(store), cfg.camera_id, Some(handle), "model_error");
            None
        }
    }
}

/// 数据面主循环运行在专用线程，事件经 EventSink 落库。
fn load_worker_ppe(
    cfg: &WorkerConfig,
    handle: &WorkerHandle,
) -> Option<Box<dyn ls_analyzer::pipeline::PpeRunner>> {
    let model = cfg.ppe.as_ref()?;
    let runner = ls_analyzer::ppe::make_ppe_runner(model);
    let _ = handle.ppe_loaded.set(runner.is_some());
    runner
}

fn worker_loop(st: Shared, cfg: WorkerConfig, handle: Arc<WorkerHandle>) {
    use ls_infer::ort_session::{set_thread_gpu_priority, GpuPriority};
    set_thread_gpu_priority(if cfg.spec.type_ == "file" {
        GpuPriority::Normal
    } else {
        GpuPriority::Live
    });
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        st.workers
            .remove_and_stop(None, cfg.camera_id, Some(&handle), "error");
        return;
    };
    let cid = cfg.camera_id;
    let set_status = |status| {
        st.workers
            .with_current(cid, &handle, || store.set_camera_status(cid, status))
    };
    if set_status("starting").is_none() {
        return;
    }
    let enter = env_f64("LS_ZONE_ENTER_FRAMES", 2.0) as u32;
    let exit = env_f64("LS_ZONE_EXIT_FRAMES", 3.0) as u32;
    let mut policy = cfg.policy.clone();
    let mut pipe = ls_analyzer::pipeline::CameraPipeline::new(
        cfg.spec.clone(),
        policy.zones.clone(),
        policy.rules.clone(),
        policy.calendar.clone(),
        enter,
        exit,
    );
    // LS-050：Motion Gate 运动预筛（空场景跳过 YOLO/PPE）；运行时从 app_settings 读取
    let mut motion_gate = ls_analyzer::motion::MotionGate::new(ls_analyzer::motion::MotionConfig {
        enabled: store
            .setting_get("perf.motion_gate_enabled")
            .ok()
            .flatten()
            .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false),
        threshold: store
            .setting_get("perf.motion_threshold")
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(ls_analyzer::motion::DEFAULT_MOTION_THRESHOLD),
        heartbeat_frames: ls_analyzer::motion::DEFAULT_HEARTBEAT_FRAMES,
    });
    policy.spatial.apply_motion_masks(&mut motion_gate);
    let mut stationary = crate::spatial::StationaryTracks::default();
    let mut stationary_only = false;
    let mut motion_cfg_stamp = std::time::Instant::now() - std::time::Duration::from_secs(3600);
    // LS-051：实时检测元数据广播节流（≤5Hz），慢客户端由 WS Lagged 丢弃
    let mut last_det_broadcast = std::time::Instant::now() - std::time::Duration::from_secs(3600);
    let mut last_face_sample: Option<std::time::Instant> = None;
    let mut next_face = 0usize;
    // RA2-S1：PPE 级联注入管线（未配置模型 → None → 仅人员检测，ppe 恒 Unknown）
    let ppe_runner = load_worker_ppe(&cfg, &handle);
    pipe.set_ppe(ppe_runner);
    let evidence = Arc::new(ls_analyzer::evidence::Evidence::default());
    evidence.new_ring(cfg.spec.analysis_fps as f64, cfg.evidence_fps);
    let ingest_config = ls_ingest::IngestConfig {
        source: cfg.spec.source.clone(),
        analysis_fps: cfg.spec.analysis_fps,
        width: 1280,
        height: 720,
        file_loop: cfg.spec.file_loop,
        rtsp_transport_tcp: true,
    };
    let live_capture = matches!(cfg.spec.type_.as_str(), "rtsp" | "usb");
    let sink = EventSink::new(Arc::clone(&st));
    // UX2（2026-11-20）：检测模式解析——per-camera analyzer 覆盖全局策略
    // （default=跟随全局 / rust=仅本地 / vlm=仅 API / cascade=协作）。
    // 读取顺序 DB → env → 默认（DetectCfg::load）；api 模式沿用 RA5 抽样判定
    // 限流（默认 6 次/分钟/路）；cascade 模式走异步复核队列（见下方触发门）。
    let dcfg = crate::detect::DetectCfg::load(&store, Path::new(&st.data_dir));
    let mode = crate::detect::effective_mode(&cfg.analyzer, &dcfg.mode);
    let sink = sink.use_cascade(mode == "cascade");
    let continuous_ppe = mode != "api";
    pipe.set_continuous_ppe(continuous_ppe);
    let mut episodes = episodes::EpisodeRuntime::default();
    let stream_started = std::time::Instant::now();
    let mut last_episode_check = None;
    let cascade_suppress = mode == "cascade" && dcfg.on_negative == "suppress";
    let mut vlm_cfg = dcfg.to_vlm_cfg();
    match mode.as_str() {
        // UX2：per-camera 覆盖（analyzer=vlm）在全局 mode=local 时也必须生效——
        // enabled 按 effective mode + 合规门算，不看全局 mode
        "api" => vlm_cfg.enabled = dcfg.compliance_ok(),
        _ => vlm_cfg.enabled = false, // cascade 走触发门 + 复核队列；local 零 API
    }
    let Some(mut detector) = worker_detector(&st, &store, &cfg, &handle, vlm_cfg.enabled) else {
        return;
    };
    let vlm_interval = std::time::Duration::from_secs(
        (60f64 / env_f64("LS_VLM_MAX_PER_MIN", 6.0).max(0.1)).round() as u64,
    );
    // LS-043：用 Option<Instant> 替代 `Instant::now() - vlm_interval`，防 Windows 刚开机减法下溢 panic
    let mut last_vlm: Option<std::time::Instant> = None;
    let mut reviewed_tracks = HashSet::new();
    let mut reconnect = CameraReconnect::default();
    let mut last_compliance_sample = None::<Instant>;
    let notify_connection = |online: bool| {
        let (behavior, level, detail) = if online {
            ("camera_recovered", 1, "已重新收到稳定画面")
        } else {
            (
                "camera_offline",
                2,
                "无法继续读取主码流或采集设备，请检查设备和网络",
            )
        };
        let ts = ls_core::time::local_now_iso();
        let new_event = ls_store::NewEvent {
            camera_id: cid,
            type_: behavior,
            level,
            track_id: 0,
            detail,
            snapshot: None,
            ts: &ts,
        };
        if let Some(Ok(Some(eid))) = st.workers.with_current(cid, &handle, || {
            store.record_camera_connection_event(online, &new_event)
        }) {
            if let Ok(Some(event)) = store.event_by_id(eid) {
                let _ = st.events_tx.send(event_json(&event).to_string());
            }
            let candidate = ls_analyzer::pipeline::Candidate {
                camera_id: cid,
                zone_id: None,
                behavior: behavior.into(),
                level,
                track_id: 0,
                detail: detail.into(),
                ts_iso: ts,
                evidence_key: String::new(),
            };
            sink.deliver_deferred_pub(eid, &candidate, false);
        }
    };
    let after_failure = |state: &mut CameraReconnect, reason: &str| {
        let (retry, outage) = state.failed(Instant::now(), cfg.max_restarts);
        crate::logger::warn(&format!(
            "[ls-worker] cam-{cid} 重连第 {} 次: {reason}",
            state.attempts
        ));
        if live_capture && outage {
            notify_connection(false);
        }
        if retry {
            let _ = set_status("reconnecting");
        }
        retry
    };
    let mut live_index = 0;
    let mut use_main_stream = false;
    let mut exit_status = "stopped";
    let video_file = cfg.spec.type_ == "file";
    let mut checked_frames = 0u64;
    let mut person_frames = 0u64;
    let mut failed_frames = 0u64;
    let mut consecutive_failures = 0u32;
    let mut alert_events = 0u64;
    'outer: loop {
        if handle.stop.load(Ordering::SeqCst) {
            break;
        }
        let mut input = ingest_config.clone();
        let mut input_kind = "main";
        if !use_main_stream && cfg.spec.type_ == "rtsp" {
            if !cfg.analysis_source.is_empty() {
                input.source.clone_from(&cfg.analysis_source);
                input_kind = "substream";
            } else if cfg.analysis_shared {
                if let Some(source) = st
                    .go2rtc
                    .local_analysis_source(cfg.camera_id, &input.source)
                {
                    input.source = source;
                    input_kind = "relay";
                }
            }
        }
        let mut reader = match ls_ingest::capture::FrameReader::start(
            input,
            Arc::clone(&handle.ffmpeg),
            live_capture,
        ) {
            Ok(reader) => reader,
            Err(e) => {
                if let Ok(mut metrics) = handle.performance.lock() {
                    metrics.reconnect();
                }
                if input_kind != "main" {
                    use_main_stream = true;
                    continue;
                }
                if !after_failure(&mut reconnect, &format!("启动采集失败: {e}")) {
                    exit_status = "error";
                    break;
                }
                if !sleep_interruptible(&handle.stop, restart_backoff(reconnect.attempts)) {
                    break;
                }
                continue;
            }
        };
        let stream_index_base = live_index;
        let mut running_marked = false;
        let mut stable = false;
        let mut stream_ended = false;
        if let Ok(mut metrics) = handle.performance.lock() {
            metrics.input_kind = input_kind;
        }
        loop {
            if handle.stop.load(Ordering::SeqCst) {
                break 'outer;
            }
            let read_started = std::time::Instant::now();
            match reader.read() {
                Ok(Some(captured)) => {
                    let received = std::time::Instant::now();
                    if handle.stop.load(Ordering::SeqCst) {
                        break 'outer;
                    }
                    if let Some((revision, updated)) =
                        changed_camera_policy(&st, &store, cid, &handle)
                    {
                        pipe.replace_policy(&updated.zones, &updated.rules, &updated.calendar);
                        updated.spatial.apply_motion_masks(&mut motion_gate);
                        stationary = Default::default();
                        stationary_only = false;
                        policy = updated;
                        handle
                            .applied_policy_revision
                            .store(revision, Ordering::Release);
                    }
                    if !running_marked {
                        let status = if consecutive_failures >= 3 {
                            "model_error"
                        } else {
                            "running"
                        };
                        if set_status(status).is_none() {
                            break 'outer;
                        }
                        running_marked = true;
                    }
                    if !stable && reconnect.reset_after_good_frames(captured.arrived) {
                        stable = true;
                        if live_capture {
                            notify_connection(true);
                        }
                    }
                    let mut frame = captured.frame;
                    if live_capture {
                        frame.index = frame.index.saturating_add(stream_index_base);
                        live_index = frame.index;
                    }
                    // 先喂证据环（事件帧须已在环内；同时自动补活动录像 post 帧）
                    let jpeg_start = std::time::Instant::now();
                    let mut ev_snapshot: Option<Vec<u8>> = evidence.feed(frame.index, || {
                        ls_analyzer::evidence::encode_jpeg_bgr(
                            &frame.data,
                            frame.width,
                            frame.height,
                        )
                        .unwrap_or_default()
                    });
                    if let Some(j) = ev_snapshot.as_ref().filter(|jpeg| !jpeg.is_empty()) {
                        if let Ok(mut slot) = handle.latest_jpeg.lock() {
                            *slot = Some(Snapshot {
                                jpeg: j.clone().into(),
                                captured_at_ms: captured
                                    .timestamp
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis(),
                                sequence: PREVIEW_SEQ.fetch_add(1, Ordering::Relaxed),
                            });
                        }
                    }
                    let jpeg_elapsed = jpeg_start.elapsed();
                    // LS-050 Motion Gate：每秒刷新配置（网页开关/阈值即时生效）；
                    // 无活跃 track + 低帧差 + 心跳未到 → 跳过 YOLO/PPE 推理。
                    if motion_cfg_stamp.elapsed() >= std::time::Duration::from_secs(1) {
                        motion_cfg_stamp = std::time::Instant::now();
                        motion_gate.set_config(ls_analyzer::motion::MotionConfig {
                            enabled: store
                                .setting_get("perf.motion_gate_enabled")
                                .ok()
                                .flatten()
                                .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
                                .unwrap_or(false),
                            threshold: store
                                .setting_get("perf.motion_threshold")
                                .ok()
                                .flatten()
                                .and_then(|v| v.trim().parse::<f32>().ok())
                                .filter(|v| v.is_finite() && *v >= 0.0)
                                .unwrap_or(ls_analyzer::motion::DEFAULT_MOTION_THRESHOLD),
                            heartbeat_frames: ls_analyzer::motion::DEFAULT_HEARTBEAT_FRAMES,
                        });
                    }
                    let mut motion_skip = live_capture
                        && consecutive_failures == 0
                        && motion_gate.should_skip(
                            &frame.data,
                            frame.width,
                            frame.height,
                            pipe.active_track_count() > 0 && !stationary_only,
                        );
                    let ts = ls_core::time::format_system_time_local_iso(captured.timestamp);
                    let audit_frame = crate::spatial::AuditFrame {
                        data_dir: &st.data_dir,
                        jpeg: ev_snapshot.as_deref(),
                        pixels: Some((&frame.data, frame.width, frame.height)),
                    };
                    if motion_skip && (stationary_only || motion_gate.masked_skip()) {
                        motion_skip = crate::spatial::audit(
                            &store,
                            cid,
                            if stationary_only {
                                "stationary"
                            } else {
                                "motion_mask"
                            },
                            0,
                            "person_inference",
                            &ts,
                            Some(&audit_frame),
                        );
                    }
                    handle.motion_total.fetch_add(1, Ordering::Relaxed);
                    let detect_start = std::time::Instant::now();
                    let dets = if motion_skip {
                        handle.motion_skipped.fetch_add(1, Ordering::Relaxed);
                        Vec::new() // 跳过昂贵推理；管线空帧路径仍走（保留 exit 语义/心跳）
                    } else if vlm_cfg.enabled {
                        detector.detect(&frame)
                    } else {
                        match detector.try_detect(&frame) {
                            Ok(dets) => {
                                if live_capture && consecutive_failures >= 3 {
                                    let _ = set_status("running");
                                }
                                consecutive_failures = 0;
                                dets
                            }
                            Err(error) => {
                                failed_frames += u64::from(video_file);
                                consecutive_failures = consecutive_failures.saturating_add(1);
                                if consecutive_failures == 1 {
                                    crate::logger::error(&format!(
                                        "[ls-worker] cam-{cid} 人员推理失败: {error}"
                                    ));
                                }
                                if live_capture && consecutive_failures == 3 {
                                    let _ = set_status("model_error");
                                }
                                continue;
                            }
                        }
                    };
                    let detection_elapsed = detect_start.elapsed();
                    if video_file {
                        checked_frames += 1;
                        person_frames += u64::from(!dets.is_empty());
                    }
                    let rules_start = std::time::Instant::now();
                    let episode_clock = if video_file {
                        std::time::Duration::from_secs_f64(
                            (checked_frames + failed_frames) as f64
                                / f64::from(cfg.spec.analysis_fps.max(1.0)),
                        )
                    } else {
                        captured.arrived.saturating_duration_since(stream_started)
                    };
                    let review_interval = if episodes.recovery_check_due() {
                        std::time::Duration::from_secs(1)
                    } else {
                        ls_analyzer::episode::REVIEW_INTERVAL
                    };
                    let recheck = continuous_ppe
                        && !motion_skip
                        && !dets.is_empty()
                        && last_episode_check.is_none_or(|previous| {
                            episode_clock.saturating_sub(previous) >= review_interval
                        })
                        && policy
                            .rules
                            .iter()
                            .any(|rule| episodes::episode_behavior(&rule.behavior));
                    let sample_ppe = live_capture
                        && mode != "api"
                        && !motion_skip
                        && last_compliance_sample.is_none_or(|last| {
                            captured.arrived.saturating_duration_since(last)
                                >= COMPLIANCE_SAMPLE_INTERVAL
                        });
                    let mut cands = if motion_skip && stationary_only {
                        // Keep existing tracks until an observed frame resumes analysis.
                        Vec::new()
                    } else {
                        pipe.process_with_recheck(
                            &dets,
                            &ts,
                            Some((&frame.data, frame.width, frame.height)),
                            sample_ppe,
                            recheck,
                        )
                    };
                    if continuous_ppe
                        && !motion_skip
                        && (recheck || !pipe.ppe_readings().is_empty())
                    {
                        last_episode_check = Some(episode_clock);
                    }
                    if live_capture && !motion_skip {
                        stationary_only = stationary.update(&policy.spatial, pipe.last_tracks());
                    }
                    let tracked_box = |id| {
                        pipe.last_tracks()
                            .iter()
                            .find(|track| track.id == id && !track.occluded)
                            .map(|track| track.bbox)
                    };
                    cands.retain(|cand| {
                        tracked_box(cand.track_id).is_none_or(|bbox| {
                            !policy.spatial.suppress_with_evidence(
                                &store,
                                cid,
                                &cand.behavior,
                                bbox,
                                &ts,
                                Some(&audit_frame),
                            )
                        })
                    });
                    if !motion_skip
                        && !dets.is_empty()
                        && last_face_sample.is_none_or(|t| {
                            captured.arrived.saturating_duration_since(t)
                                >= std::time::Duration::from_secs(4)
                        })
                    {
                        last_face_sample = Some(captured.arrived);
                        if crate::local_face::match_alert_enabled(&store, cfg.camera_id) {
                            let tracks: Vec<_> = pipe
                                .last_tracks()
                                .iter()
                                .filter(|track| !track.occluded)
                                .collect();
                            if !tracks.is_empty() {
                                let track = tracks[next_face % tracks.len()];
                                next_face = next_face.wrapping_add(1);
                                let jpeg = ev_snapshot.clone().unwrap_or_else(|| {
                                    ls_analyzer::evidence::encode_jpeg_bgr(
                                        &frame.data,
                                        frame.width,
                                        frame.height,
                                    )
                                    .unwrap_or_default()
                                });
                                crate::local_face::identify_event(
                                    &st,
                                    &store,
                                    crate::local_face::MatchTarget::Alert(track.id),
                                    cfg.camera_id,
                                    track.bbox,
                                    &jpeg,
                                );
                            }
                        }
                    }
                    let rules_elapsed = rules_start.elapsed();
                    let analyzed = std::time::Instant::now();
                    let analysis_elapsed = analyzed - received;
                    let inference = ls_infer::yolo::take_inference_timings();
                    if sample_ppe && inference.calls > 0 {
                        let sample = pipe.last_ppe_sample();
                        if let Some(day) = ts.get(..10) {
                            let _ = store.add_compliance_sample(
                                cid,
                                day,
                                sample.compliant,
                                sample.violations,
                                sample.unknown,
                            );
                        }
                        last_compliance_sample = Some(captured.arrived);
                    }
                    // LS-051：≤5Hz 推送实时检测框元数据（tracks + zones）到 WS 订阅者
                    let broadcast_due =
                        last_det_broadcast.elapsed() >= std::time::Duration::from_millis(200);
                    if let Some(detections) = broadcast_due
                        .then(|| st.detections.subscribed(cfg.camera_id))
                        .flatten()
                    {
                        last_det_broadcast = std::time::Instant::now();
                        let tracks: Vec<serde_json::Value> = pipe
                            .last_tracks()
                            .iter()
                            .map(|track| {
                                let state = match track.ppe {
                                    ls_analyzer::pipeline::PpeStatus::NoPpe => "violation",
                                    ls_analyzer::pipeline::PpeStatus::Wearing => "compliant",
                                    ls_analyzer::pipeline::PpeStatus::Unknown => "pending",
                                };
                                serde_json::json!({
                                    "id": track.id,
                                    "box": track.bbox,
                                    "label": "person",
                                    "ppe_state": state,
                                    "conf": track.confidence,
                                    "occluded": track.occluded,
                                })
                            })
                            .collect();
                        let zones: Vec<serde_json::Value> = policy
                            .zones
                            .iter()
                            .map(|z| serde_json::json!({"id": z.id, "points": z.points}))
                            .collect();
                        let payload = serde_json::json!({
                            "camera_id": cfg.camera_id,
                            "ts": ts,
                            "img_w": frame.width,
                            "img_h": frame.height,
                            "tracks": tracks,
                            "zones": zones,
                            "episode_countdowns": episodes.countdowns(&policy),
                            "objects": if motion_skip { &[] } else { detector.objects() },
                        });
                        let _ = detections.send(payload.to_string());
                    }
                    // UX2 cascade：suppress 策略下本地事件先不派发（复核肯定补投递）
                    let dispatch_now = !cascade_suppress;
                    let mut ev_round: Vec<(i64, ls_analyzer::pipeline::Candidate)> = Vec::new();
                    if continuous_ppe && !motion_skip {
                        let episode_frame = episodes::EpisodeFrame {
                            camera_id: cid,
                            camera_name: &cfg.spec.name,
                            time: episode_clock,
                            timestamp: &ts,
                            frame: &frame,
                            jpeg: ev_snapshot.as_deref(),
                            dispatch: dispatch_now,
                        };
                        match episodes.process(&store, &sink, &pipe, &policy, &episode_frame) {
                            Ok(events) => {
                                for (id, candidate) in events {
                                    spawn_finalize(&st, &evidence, id);
                                    alert_events += u64::from(video_file);
                                    ev_round.push((id, candidate));
                                }
                            }
                            Err(error) => crate::logger::error(&format!(
                                "[ls-worker] cam-{cid} 事件段落库失败: {error}"
                            )),
                        }
                    }
                    let rules_hit = !cands.is_empty() || !ev_round.is_empty();
                    if !cands.is_empty() {
                        // 事件帧快照：优先复用本帧抽帧结果，未命中 stride 则现编
                        let snap = match ev_snapshot.as_ref() {
                            Some(j) if !j.is_empty() => j.clone(),
                            _ => ls_analyzer::evidence::encode_jpeg_bgr(
                                &frame.data,
                                frame.width,
                                frame.height,
                            )
                            .unwrap_or_default(),
                        };
                        for cand in cands {
                            let bounds = tracked_box(cand.track_id);
                            let overlay = detection_overlay(
                                frame.width,
                                frame.height,
                                bounds,
                                cand.zone_id
                                    .and_then(|id| policy.zones.iter().find(|zone| zone.id == id)),
                                pipe.last_tracks()
                                    .iter()
                                    .find(|track| track.id == cand.track_id)
                                    .map(|track| {
                                        serde_json::json!({
                                            "person_confidence": track.confidence,
                                            "occluded": track.occluded,
                                        })
                                    }),
                            );
                            match sink.record_located(
                                &cand,
                                Some(&snap),
                                dispatch_now,
                                bounds,
                                overlay.as_deref(),
                                None,
                            ) {
                                Ok(Some(eid)) => {
                                    if video_file {
                                        alert_events += 1;
                                    }
                                    spawn_finalize(&st, &evidence, eid);
                                    if let Some(track) = pipe
                                        .last_tracks()
                                        .iter()
                                        .find(|track| track.id == cand.track_id && !track.occluded)
                                    {
                                        crate::local_face::identify_event(
                                            &st,
                                            &store,
                                            crate::local_face::MatchTarget::Event(eid),
                                            cfg.camera_id,
                                            track.bbox,
                                            &snap,
                                        );
                                    }
                                    ev_round.push((eid, cand));
                                }
                                Ok(None) => {} // 去重命中
                                Err(e) => crate::logger::error(&format!(
                                    "[ls-worker] cam-{} 事件落地失败: {e}",
                                    cfg.camera_id
                                )),
                            }
                        }
                        ev_snapshot = Some(snap);
                    }
                    if mode == "cascade" {
                        keep_geometry_local(&store, &sink, &mut ev_round, cascade_suppress);
                        let tracks = pipe.last_tracks();
                        reviewed_tracks.retain(|id| pipe.can_resume_track(*id));
                        let gray: Vec<_> = tracks
                            .iter()
                            .filter(|track| {
                                !motion_skip
                                    && !track.occluded
                                    && !reviewed_tracks.contains(&track.id)
                                    && crate::detect::cascade_should_review(
                                        &dcfg,
                                        1,
                                        track.confidence,
                                        false,
                                    )
                            })
                            .map(|track| track.id)
                            .collect();
                        let rule_review = !ev_round.is_empty() && dcfg.trigger != "persons";
                        let gray_review = !gray.is_empty()
                            && (!ev_round.is_empty()
                                || last_vlm.is_none_or(|t| t.elapsed() >= vlm_interval));
                        if rule_review || gray_review {
                            last_vlm = Some(Instant::now());
                            reviewed_tracks.extend(gray);
                            reviewed_tracks
                                .extend(ev_round.iter().map(|(_, event)| event.track_id));
                            let jpeg = ev_snapshot.filter(|jpeg| !jpeg.is_empty()).or_else(|| {
                                ls_analyzer::evidence::encode_jpeg_bgr(
                                    &frame.data,
                                    frame.width,
                                    frame.height,
                                )
                                .ok()
                            });
                            submit_cascade_review(
                                &store,
                                &sink,
                                CascadeReview {
                                    camera_id: cid,
                                    cfg: dcfg.clone(),
                                    jpeg: jpeg.unwrap_or_default(),
                                    detected_people: dets.len(),
                                    ev_round,
                                    suppress: cascade_suppress,
                                    evidence: Some(Arc::clone(&evidence)),
                                    evidence_key_prefix: format!("vlmc:{}", frame.index),
                                },
                            );
                        } else if !ev_round.is_empty() {
                            finish_cascade_events(
                                &store,
                                &sink,
                                &ev_round,
                                cascade_suppress,
                                &serde_json::json!({"status":"unreviewed","reason":"trigger_not_met"}),
                            );
                        }
                    } else if vlm_cfg.enabled
                        && last_vlm.is_none_or(|t| t.elapsed() >= vlm_interval)
                    {
                        last_vlm = Some(std::time::Instant::now());
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
                                                "AI 精判（视频）：{}，画面中约 {persons} 人 — {summary}",
                                                ls_dispatch::type_text(behavior)
                                            ),
                                            ts_iso: ts.clone(),
                                            evidence_key: format!("vlm:{}:{behavior}", frame.index),
                                        };
                                        if let Ok(Some(veid)) = sink.record(&cand, Some(&jpeg)) {
                                            if video_file {
                                                alert_events += 1;
                                            }
                                            spawn_finalize(&st, &evidence, veid);
                                        }
                                    }
                                }
                                Err(e) => {
                                    crate::logger::error(&format!(
                                        "[ls-worker] cam-{} VLM 判定失败（降级跳过）: {e}",
                                        cfg.camera_id
                                    ));
                                }
                            }
                        }
                    }
                    if let Ok(mut metrics) = handle.performance.lock() {
                        metrics.record(FrameTiming {
                            read_wait: received.saturating_duration_since(read_started),
                            queue_wait: received.saturating_duration_since(captured.arrived),
                            analysis_age: analyzed.saturating_duration_since(captured.arrived),
                            overwritten: captured.overwritten,
                            jpeg: jpeg_elapsed,
                            detection: detection_elapsed,
                            rules: rules_elapsed,
                            ort: inference.run,
                            directml_wait: inference.directml_wait,
                            analysis: analysis_elapsed,
                            persistence: rules_hit.then(|| received.elapsed()),
                            inferred: inference.calls > 0,
                            directml_calls: inference.directml_calls,
                        });
                    }
                }
                Ok(None) => {
                    stream_ended = true;
                    break;
                }
                Err(e) => {
                    crate::logger::error(&format!(
                        "[ls-worker] cam-{} 读帧失败: {e}",
                        cfg.camera_id
                    ));
                    break;
                }
            }
        }
        let file_completed = cfg.spec.type_ == "file"
            && !cfg.spec.file_loop
            && running_marked
            && stream_ended
            && reader.file_exit_ok()
            && checked_frames > 0
            && failed_frames == 0;
        drop(reader);
        if handle.stop.load(Ordering::SeqCst) {
            break;
        }
        if cfg.spec.type_ == "file" && !cfg.spec.file_loop {
            exit_status = if file_completed { "completed" } else { "error" };
            break;
        }
        if let Ok(mut metrics) = handle.performance.lock() {
            metrics.reconnect();
        }
        if input_kind != "main" {
            use_main_stream = true;
            reconnect.good_since = None;
            continue;
        }
        use_main_stream = false;
        if !after_failure(&mut reconnect, "画面流中断") {
            exit_status = "error";
            break;
        }
        if !sleep_interruptible(&handle.stop, restart_backoff(reconnect.attempts)) {
            break;
        }
    }
    evidence.flush();
    episodes.interrupt(&store, &sink, cid, &crate::util::iso_now());
    if video_file && (checked_frames > 0 || failed_frames > 0 || exit_status == "error") {
        let outcome = match exit_status {
            "completed" => "检查完成",
            "error" => "检查异常",
            _ => "检查已停止",
        };
        let enabled_rules = policy.rules.iter().filter(|rule| rule.enabled).count();
        let detail = format!(
            "视频{outcome}。已处理 {checked_frames} 帧，{person_frames} 帧检出人员，{failed_frames} 帧人员推理失败，启用规则 {enabled_rules} 条，已记录规则和模型事件 {alert_events} 条。"
        );
        let finished = crate::util::iso_now();
        let latest = handle.latest_jpeg.lock().ok().and_then(|slot| slot.clone());
        let snapshot = latest.and_then(|frame| {
            sink.save_snapshot(cid, &finished, &frame.jpeg)
                .map_err(|error| {
                    crate::logger::warn(&format!("[ls-worker] cam-{cid} {error}"));
                })
                .ok()
        });
        if let Err(error) = store.add_check_event_with_digest(
            cid,
            "video_check",
            &detail,
            &finished,
            snapshot.as_ref().map(|image| image.path.as_str()),
            snapshot.as_ref().map(|image| image.sha256.as_str()),
        ) {
            if let Some(image) = snapshot {
                let _ = std::fs::remove_file(image.path);
            }
            crate::logger::error(&format!(
                "[ls-worker] cam-{cid} 无法保存视频检查记录: {error}"
            ));
        }
    }
    st.workers
        .remove_and_stop(Some(&store), cid, Some(&handle), exit_status);
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
            crate::logger::warn("[ls-worker] 并发录像超限，跳过视频回填");
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
                Err(e) => {
                    crate::logger::error(&format!("[ls-worker] event#{eid} 视频回填失败: {e}"))
                }
            }
            // 从活动列表移除（防长跑泄漏）
            if let Ok(mut recs) = evidence.recs.lock() {
                recs.retain(|r| !Arc::ptr_eq(r, &rec));
            }
        });
    if let Err(e) = spawned {
        crate::logger::error(&format!("[ls-worker] event#{eid} 收尾线程创建失败: {e}"));
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
    ls_infer::ort_session::set_thread_gpu_priority(ls_infer::ort_session::GpuPriority::Background);
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        st.workers
            .remove_and_stop(None, cfg.camera_id, Some(&handle), "error");
        return;
    };
    if st
        .workers
        .with_current(cfg.camera_id, &handle, || {
            store.set_camera_status(cfg.camera_id, "starting")
        })
        .is_none()
    {
        return;
    }
    let root = PathBuf::from(cfg.spec.source.trim());
    let interval = env_f64("LS_FOLDER_POLL_S", 3.0).max(0.5);
    let scan_cfg = ls_ingest::folder::FolderScanConfig::default();
    let internal = Some(PathBuf::from(&st.data_dir)); // 截图/视频数据目录不入库
                                                      // UX2：检测模式解析（default=跟随全局 / rust=仅本地 / vlm=仅 API / cascade=协作）
    let dcfg = crate::detect::DetectCfg::load(&store, Path::new(&st.data_dir));
    let mode = crate::detect::effective_mode(&cfg.analyzer, &dcfg.mode);
    let cascade_suppress = mode == "cascade" && dcfg.on_negative == "suppress";
    let mut vlm_cfg = dcfg.to_vlm_cfg();
    // UX2：per-camera 覆盖（analyzer=vlm）在全局 mode=local 时也必须生效
    vlm_cfg.enabled = mode == "api" && dcfg.compliance_ok();
    let review_interval = std::time::Duration::from_secs(
        (60f64 / env_f64("LS_VLM_MAX_PER_MIN", 6.0).max(0.1)).round() as u64,
    );
    let mut last_review: Option<std::time::Instant> = None; // 首图即可复核（无下溢 panic）
    let mut marked: Option<String> = None;
    let mut set_status = |v: &str| {
        if marked.as_deref() != Some(v) {
            let _ = st.workers.with_current(cfg.camera_id, &handle, || {
                store.set_camera_status(cfg.camera_id, v)
            });
            marked = Some(v.into());
        }
    };
    // RA2 反思修正：识别器/管件在循环外构造一次——此前在 loop 体内，
    // 每 tick（3s）都重新加载 YOLO 模型（数百 ms 级开销 × 每 3s）
    let Some(mut detector) = worker_detector(&st, &store, &cfg, &handle, vlm_cfg.enabled) else {
        return;
    };
    let mut ppe_runner = load_worker_ppe(&cfg, &handle);
    let mut policy = cfg.policy.clone();
    let sink = EventSink::new(Arc::clone(&st)).use_cascade(mode == "cascade");
    loop {
        if handle.stop.load(Ordering::SeqCst) {
            break;
        }
        handle.wake.store(false, Ordering::SeqCst); // 消费上一轮唤醒
        if let Some((revision, updated)) =
            changed_camera_policy(&st, &store, cfg.camera_id, &handle)
        {
            policy = updated;
            handle
                .applied_policy_revision
                .store(revision, Ordering::Release);
        }
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
        // LS-043 修复饥饿：不再对全量 cands 截断 `.take(max_per_tick)`（此前目录 >20 张图时
        // 前 20 张已处理图占满名额，第 21 张及更早图片永久不会被扫描），改为仅实际新识别满额后 break。
        for c in cands.iter() {
            if handle.stop.load(Ordering::SeqCst) || scanned >= max_per_tick {
                break;
            }
            if let Some((revision, updated)) =
                changed_camera_policy(&st, &store, cfg.camera_id, &handle)
            {
                policy = updated;
                handle
                    .applied_policy_revision
                    .store(revision, Ordering::Release);
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
                    crate::logger::error(&format!(
                        "[ls-folder] cam-{} 登记失败 {}: {e}",
                        cfg.camera_id,
                        c.path.display()
                    ));
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
                                        "AI 精判：图片检出 {}（{} 人，未穿服 {}）— {}",
                                        ls_dispatch::type_text(behavior),
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
            let mut dets = match detector.try_detect(&frame) {
                Ok(dets) => dets,
                Err(error) => {
                    crate::logger::error(&format!(
                        "[ls-folder] cam-{} 人员推理失败: {error}",
                        cfg.camera_id
                    ));
                    let _ = store.update_folder_image_analysis(
                        iid,
                        w as i64,
                        h as i64,
                        0,
                        0,
                        0,
                        0,
                        "[]",
                        "[]",
                        false,
                        false,
                        None,
                        Some("人员推理失败，请检查所选模型与诊断日志"),
                        &processed_ts,
                    );
                    scanned += 1;
                    continue;
                }
            };
            let mut unmasked = std::collections::HashSet::new();
            // PPE 级联：逐人框判定（LS-043：空图 0 人跳过 PPE 推理节省算力）
            if !dets.is_empty() {
                if let Some(runner) = ppe_runner.as_mut() {
                    let boxes: Vec<[f32; 4]> = dets.iter().map(|d| d.box_).collect();
                    let observations = runner.observations(&bgr, w, h, &boxes);
                    for (index, (d, observation)) in dets.iter_mut().zip(observations).enumerate() {
                        d.ppe = observation.coat;
                        if observation.mask == ls_analyzer::pipeline::PpeStatus::NoPpe {
                            unmasked.insert(index);
                        }
                    }
                }
            }
            let cid = cfg.camera_id;
            let audit_frame = crate::spatial::AuditFrame {
                data_dir: &st.data_dir,
                jpeg: None,
                pixels: Some((&bgr, w, h)),
            };
            let uncoated: HashSet<_> = dets
                .iter()
                .enumerate()
                .filter_map(|(index, det)| {
                    (det.ppe == ls_analyzer::pipeline::PpeStatus::NoPpe
                        && !policy.spatial.suppress_with_evidence(
                            &store,
                            cid,
                            "enter_no_ppe",
                            det.box_,
                            &processed_ts,
                            Some(&audit_frame),
                        ))
                    .then_some(index)
                })
                .collect();
            unmasked.retain(|&index| {
                !policy.spatial.suppress_with_evidence(
                    &store,
                    cid,
                    "no_mask",
                    dets[index].box_,
                    &processed_ts,
                    Some(&audit_frame),
                )
            });
            // 照片按与视频相同的区域锚点判定；单帧没有历史在场状态。
            let mut zone_hits: Vec<serde_json::Value> = Vec::new();
            let mut in_zone_ids: std::collections::HashSet<usize> = Default::default();
            let ppe_no = dets
                .iter()
                .filter(|d| d.ppe == ls_analyzer::pipeline::PpeStatus::NoPpe)
                .count() as i64;
            let ppe_wearing = dets
                .iter()
                .filter(|d| d.ppe == ls_analyzer::pipeline::PpeStatus::Wearing)
                .count() as i64;
            let mut violations: Vec<String> = Vec::new();
            let mut occupancy = std::collections::HashMap::new();
            let mut zone_members = std::collections::HashMap::<_, HashSet<_>>::new();
            for z in &policy.zones {
                let mut zone_person = 0i64;
                let mut zone_no = 0i64;
                let mut zone_wear = 0i64;
                for (di, d) in dets.iter().enumerate() {
                    if z.spatial.observe(&z.points, d.box_) == Some(true) {
                        zone_person += 1;
                        in_zone_ids.insert(di);
                        zone_members.entry(z.id).or_default().insert(di);
                        match d.ppe {
                            ls_analyzer::pipeline::PpeStatus::NoPpe => {
                                zone_no += 1;
                            }
                            ls_analyzer::pipeline::PpeStatus::Wearing => {
                                zone_wear += 1;
                            }
                            ls_analyzer::pipeline::PpeStatus::Unknown => {}
                        }
                    }
                }
                if zone_person > 0 {
                    occupancy.insert(z.id, zone_person);
                    zone_hits.push(serde_json::json!({
                        "zone_id": z.id, "name": z.name,
                        "persons": zone_person, "no": zone_no, "wearing": zone_wear,
                    }));
                }
            }
            let persons = dets.len() as i64;
            let in_zone = in_zone_ids.len() as i64;
            if !uncoated.is_empty() {
                violations.push("enter_no_ppe".into());
            }
            if !unmasked.is_empty() {
                violations.push("no_mask".into());
            }
            let matches: Vec<_> = policy
                .rules
                .iter()
                .filter_map(|rule| {
                    if !ls_analyzer::pipeline::rule_active_at(rule, &processed_ts, &policy.calendar)
                    {
                        return None;
                    }
                    let zone_id = (rule.zone_id != 0).then_some(rule.zone_id);
                    let count =
                        zone_id.map_or(persons, |id| occupancy.get(&id).copied().unwrap_or(0));
                    let hits = match rule.behavior.as_str() {
                        "enter_no_ppe" => &uncoated,
                        "no_mask" => &unmasked,
                        "occupancy_over" => {
                            return (rule.threshold >= 0 && count > rule.threshold)
                                .then_some((rule, zone_id, count, None))
                        }
                        _ => return None,
                    };
                    let members = zone_id.and_then(|id| zone_members.get(&id));
                    let mut targets = hits.iter().filter(|index| {
                        zone_id.is_none()
                            && (rule.behavior != "enter_no_ppe" || in_zone_ids.contains(index))
                            || members.is_some_and(|people| people.contains(index))
                    });
                    let first = *targets.next()?;
                    let target = targets.next().is_none().then_some(dets[first].box_);
                    Some((rule, zone_id, count, target))
                })
                .collect();
            for (rule, _, _, _) in &matches {
                violations.push(rule.behavior.clone());
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
            // UX2 cascade：suppress 策略下先不派发，VLM 复核后放行/抑制
            let dispatch_now = !cascade_suppress;
            let mut ev_round: Vec<(i64, ls_analyzer::pipeline::Candidate)> = Vec::new();
            for (rule, zone_id, count, target) in matches {
                let zone = policy.zones.iter().find(|zone| Some(zone.id) == zone_id);
                let behavior = &rule.behavior;
                let cand = ls_analyzer::pipeline::Candidate {
                    camera_id: cfg.camera_id,
                    zone_id,
                    behavior: behavior.clone(),
                    level: if rule.level > 0 { rule.level } else { 2 },
                    track_id: 0,
                    // UX（2026-11-21 用户实测反馈）：detail 人话化——
                    // 文件名截短（原始名含长哈希后缀），行为名中文化
                    detail: format!(
                        "图片 {} 检测到 {} 人{}（{}）",
                        name.as_deref()
                            .unwrap_or("")
                            .split("_-_")
                            .next()
                            .unwrap_or(""),
                        count,
                        zone.map(|z| format!("，位于「{}」", z.name))
                            .unwrap_or_default(),
                        ls_dispatch::type_text(behavior)
                    ),
                    ts_iso: processed_ts.clone(),
                    evidence_key: format!("folder:{iid}:{}", rule.id),
                };
                let visual = target
                    .and_then(|bounds| dets.iter().find(|det| det.box_ == bounds))
                    .map(
                        |det| serde_json::json!({"person_confidence": det.score, "coat": det.ppe}),
                    );
                let overlay = detection_overlay(w, h, target, zone, visual);
                if let Ok(Some(eid)) = sink.record_located(
                    &cand,
                    snapshot_jpeg.as_deref(),
                    dispatch_now,
                    target,
                    overlay.as_deref(),
                    None,
                ) {
                    let _ = store.set_folder_image_event(iid, eid);
                    ev_round.push((eid, cand));
                }
            }
            if mode == "cascade" {
                keep_geometry_local(&store, &sink, &mut ev_round, cascade_suppress);
                let max_conf = dets.iter().map(|d| d.score).fold(0.0f32, f32::max);
                let has_event = !ev_round.is_empty();
                if crate::detect::cascade_should_review(&dcfg, persons, max_conf, has_event)
                    && (has_event || last_review.is_none_or(|t| t.elapsed() >= review_interval))
                {
                    last_review = Some(Instant::now());
                    let jpeg = snapshot_jpeg
                        .or_else(|| ls_analyzer::evidence::encode_jpeg_bgr(&bgr, w, h).ok());
                    submit_cascade_review(
                        &store,
                        &sink,
                        CascadeReview {
                            camera_id: cfg.camera_id,
                            cfg: dcfg.clone(),
                            jpeg: jpeg.unwrap_or_default(),
                            detected_people: dets.len(),
                            ev_round,
                            suppress: cascade_suppress,
                            evidence: None,
                            evidence_key_prefix: format!("vlmc-f:{iid}"),
                        },
                    );
                } else if has_event {
                    finish_cascade_events(
                        &store,
                        &sink,
                        &ev_round,
                        cascade_suppress,
                        &serde_json::json!({"status":"unreviewed","reason":"trigger_not_met"}),
                    );
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
    st.workers
        .remove_and_stop(Some(&store), cfg.camera_id, Some(&handle), "stopped");
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
        let jpeg = ls_analyzer::evidence::encode_jpeg_bgr(&[110; 8 * 8 * 3], 8, 8).unwrap();
        let eid = sink
            .record(&cand(1, "cam:1:100"), Some(&jpeg))
            .unwrap()
            .expect("首条不应去重");
        // DB 行 + 截图文件
        let s = ls_store::Store::open(&st.db_path).unwrap();
        let row = s.event_by_id(eid).unwrap().expect("事件行存在");
        assert_eq!(row.type_, "person_enter");
        assert_eq!(row.track_id, 7);
        let snap = row.snapshot.expect("截图路径已入库");
        assert!(std::path::Path::new(&snap).is_file(), "截图文件真实存在");
        let webp = std::fs::read(&snap).unwrap();
        assert_eq!(
            image::guess_format(&webp).unwrap(),
            image::ImageFormat::WebP
        );
        use sha2::{Digest, Sha256};
        assert_eq!(
            row.snapshot_sha256.as_deref(),
            Some(format!("{:x}", Sha256::digest(&webp)).as_str())
        );
        let jpeg = ls_jobs::media::snapshot_jpeg(std::path::Path::new(&snap)).unwrap();
        assert_eq!(
            image::guess_format(&jpeg).unwrap(),
            image::ImageFormat::Jpeg
        );
        // WS 广播行口径与 /api/events 一致（无绝对路径泄漏）
        let payload = rx.try_recv().unwrap();
        assert!(payload.contains("\"has_snapshot\":true"), "{payload}");
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
        assert_eq!(cfg.policy.zones.len(), 1);
        assert_eq!(
            cfg.policy.zones[0].points,
            vec![(0, 0), (100, 0), (100, 100)]
        );
        assert_eq!(cfg.policy.rules.len(), 1);
        assert_eq!(
            cfg.policy.rules[0].weekdays, "1,2,3,4,5",
            "RA1.1 days_of_week 进 RuleSpec"
        );
        assert_eq!(cfg.policy.rules[0].zone_id, zid);
        assert_eq!(cfg.policy.rules[0].time_start, "08:00");
        assert_eq!(cfg.policy.calendar.len(), 0);
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
                .add_camera_full(
                    "cam-bad",
                    &tmp.path().join("missing.mp4").to_string_lossy(),
                    "file",
                    "",
                )
                .unwrap();
            WorkerConfig::from_db(&s, cid).unwrap().unwrap()
        };
        cfg.spec.file_loop = true; // 缺失的循环文件源触发重连逻辑；普通文件 EOF 正常结束。
        cfg.max_restarts = 0;
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
            s.migrate().unwrap();
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
                    "SELECT COUNT(*) FROM folder_images WHERE camera_id=1 AND error IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            if n >= 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "5s 内应处理 2 张损坏图片"
            );
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
            track_id: 0,
            snapshot_path: None,
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
