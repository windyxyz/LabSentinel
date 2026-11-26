use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use ls_analyzer::evidence::{
    ActiveRec, Evidence, MAX_CONCURRENT_RECORDINGS, VIDEO_POST_S, VIDEO_PRE_S,
};
use ls_ingest::recording::{ClipCoverage, PendingClip, Recorder, RecordingError};
use ls_store::{EventRecording, Store};

use super::{EventSink, Shared};

#[derive(Default)]
pub(super) struct MotionRecording {
    previous: Option<[u8; 64 * 36]>,
    moving: bool,
    last_trigger: Option<Instant>,
}

impl MotionRecording {
    pub(super) fn observe(
        &mut self,
        frame: &ls_ingest::Frame,
        arrived: Instant,
        threshold: f32,
    ) -> bool {
        use ls_analyzer::motion::{downsample_gray, mean_abs_diff};
        let gray = downsample_gray(&frame.data, frame.width, frame.height);
        let moving = self
            .previous
            .replace(gray)
            .is_some_and(|prev| mean_abs_diff(&prev, &gray) >= threshold.max(2.0));
        let started = moving && !self.moving;
        self.moving = moving;
        if started
            && self.last_trigger.is_none_or(|last| {
                arrived.saturating_duration_since(last) >= Duration::from_secs(30)
            })
        {
            self.last_trigger = Some(arrived);
            true
        } else {
            false
        }
    }
}

pub(super) fn spawn_finalize(st: &Shared, evidence: &Arc<Evidence>, event: i64) {
    spawn_finalize_at(st, evidence, event, None, SystemTime::now());
}

pub(super) fn spawn_finalize_at(
    st: &Shared,
    evidence: &Arc<Evidence>,
    event: i64,
    recorder: Option<&Recorder>,
    at: SystemTime,
) {
    let Ok(store) = Store::open(&st.db_path) else {
        return;
    };
    match store.start_event_recording(event) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            crate::logger::warn(&format!("event#{event} 无法登记录像: {error}"));
            return;
        }
    }
    let raw = recorder.map(|source| {
        source.trigger(
            at,
            Duration::from_secs_f64(VIDEO_PRE_S),
            Duration::from_secs_f64(VIDEO_POST_S),
        )
    });
    let decoded = reserve_fallback(evidence);
    if decoded.is_none() && !matches!(raw, Some(Ok(_))) {
        let _ = store.event_recording_failed(event, "busy");
        return;
    }
    let state = Arc::clone(st);
    let pending_ref = decoded.as_ref().map(|(rec, _)| Arc::clone(rec));
    let buffers = Arc::clone(evidence);
    let spawn = std::thread::Builder::new()
        .name(format!("ls-finalize-{event}"))
        .spawn(move || {
            let sink = EventSink::new(state);
            let result = finish(&sink, event, raw, decoded.as_ref());
            if let Some((rec, _)) = &decoded {
                release(&buffers, rec);
            }
            if let Err(reason) = result {
                if let Ok(store) = Store::open(&sink.st.db_path) {
                    let _ = store.event_recording_failed(event, reason);
                }
            }
        });
    if let Err(error) = spawn {
        if let Some(rec) = pending_ref {
            release(evidence, &rec);
        }
        crate::logger::warn(&format!("event#{event} 无法启动录像收尾: {error}"));
        let _ = store.event_recording_failed(event, "worker_start_failed");
    }
}

fn reserve_fallback(evidence: &Evidence) -> Option<(Arc<ActiveRec>, f64)> {
    let buffered = evidence
        .ring
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let ring = buffered.as_ref()?;
    let mut active = evidence
        .recs
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if active.len() >= MAX_CONCURRENT_RECORDINGS {
        return None;
    }
    let frames = ring
        .snapshot()
        .into_iter()
        .map(|frame| frame.data)
        .collect();
    let rec = Arc::new(ActiveRec::new(frames));
    active.push(Arc::clone(&rec));
    Some((rec, ring.out_fps()))
}

fn release(evidence: &Evidence, rec: &Arc<ActiveRec>) {
    let mut active = evidence
        .recs
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    active.retain(|item| !Arc::ptr_eq(item, rec));
}

fn finish(
    sink: &EventSink,
    event: i64,
    raw: Option<Result<PendingClip, RecordingError>>,
    decoded: Option<&(Arc<ActiveRec>, f64)>,
) -> Result<(), &'static str> {
    let path = sink.recording_path(event).map_err(|error| {
        crate::logger::warn(&format!("event#{event} 无法创建录像目录: {error}"));
        "write_failed"
    })?;
    let fallback_reason = match raw {
        Some(result) => match result.and_then(|pending| pending.wait(Duration::from_secs(8))) {
            Ok(clip) => match clip.write_mp4(&path) {
                Ok(coverage) => {
                    return sink
                        .persist_original(event, &path, coverage)
                        .map_err(|error| {
                            crate::logger::warn(&format!("event#{event} 无法登记录像: {error}"));
                            "database_failed"
                        })
                }
                Err(error) => {
                    crate::logger::warn(&format!("event#{event} 无法保存原编码录像: {error}"));
                    Some("write_failed")
                }
            },
            Err(error) => Some(error.as_str()),
        },
        None => None,
    };
    let (rec, fps) = decoded.ok_or(fallback_reason.unwrap_or("unavailable"))?;
    let deadline = Instant::now()
        + Duration::from_secs_f64((VIDEO_POST_S + 1.5 - rec.elapsed().as_secs_f64()).max(0.0));
    while !rec.done.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let frames = std::mem::take(&mut *rec.frames.lock().unwrap_or_else(|error| error.into_inner()));
    sink.finalize(event, &frames, *fps, fallback_reason)
        .map_err(|error| {
            crate::logger::warn(&format!("event#{event} 无法保存回退录像: {error}"));
            fallback_reason.unwrap_or("encode_failed")
        })
}

impl EventSink {
    fn recording_path(&self, event: i64) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.paths.videos)?;
        Ok(self.paths.videos.join(format!("ev_{event}.mp4")))
    }

    pub(super) fn finalize(
        &self,
        event: i64,
        frames: &[Vec<u8>],
        fps: f64,
        reason: Option<&str>,
    ) -> Result<(), String> {
        let path = self.recording_path(event).map_err(|e| e.to_string())?;
        let temporary = tempfile::Builder::new()
            .prefix("ev-encoding-")
            .suffix(".mp4")
            .tempfile_in(&self.paths.videos)
            .map_err(|e| e.to_string())?
            .into_temp_path();
        ls_analyzer::evidence::encode_mp4(frames, fps, temporary.as_ref())?;
        temporary.persist(&path).map_err(|e| e.to_string())?;
        self.persist_recording(
            event,
            &path,
            &EventRecording {
                state: "reencoded".into(),
                codec: None,
                start_ts: None,
                end_ts: None,
                reason: reason.map(str::to_owned),
            },
            &[],
        )
    }

    fn persist_original(
        &self,
        event: i64,
        path: &Path,
        coverage: ClipCoverage,
    ) -> Result<(), String> {
        use ls_core::time::format_system_time_local_iso as iso;
        let keyframes = coverage
            .keyframes
            .into_iter()
            .map(|keyframe| (iso(keyframe.timestamp), keyframe.offset as i64))
            .collect::<Vec<_>>();
        self.persist_recording(
            event,
            path,
            &EventRecording {
                state: "original".into(),
                codec: Some(coverage.codec.as_str().into()),
                start_ts: Some(iso(coverage.start)),
                end_ts: Some(iso(coverage.end)),
                reason: coverage.truncation.map(|reason| reason.as_str().into()),
            },
            &keyframes,
        )
    }

    fn persist_recording(
        &self,
        event: i64,
        path: &Path,
        info: &EventRecording,
        keyframes: &[(String, i64)],
    ) -> Result<(), String> {
        let result = Store::open(&self.st.db_path).and_then(|store| {
            let relative = path.strip_prefix(&self.st.data_dir).unwrap_or(path);
            let saved =
                store.save_event_recording(event, &relative.to_string_lossy(), info, keyframes)?;
            if saved {
                if let Ok(Some(row)) = store.event_by_id(event) {
                    let mut message = super::event_json(&row);
                    message["op"] = serde_json::json!("recording_update");
                    let _ = self.st.events_tx.send(message.to_string());
                }
            }
            Ok(saved)
        });
        match result {
            Ok(true) => Ok(()),
            other => {
                let _ = std::fs::remove_file(path);
                Err(match other {
                    Err(error) => error.to_string(),
                    _ => "事件已清理".into(),
                })
            }
        }
    }
}
