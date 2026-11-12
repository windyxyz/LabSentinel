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

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

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
}
