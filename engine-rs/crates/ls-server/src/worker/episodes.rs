use super::{detection_overlay, CameraPolicy, EventSink};
use ls_analyzer::{
    episode::{
        AlertTier, CloseReason, Episode, EpisodeChange, EpisodeKey, EpisodeTracker, Observation,
    },
    pipeline::{rule_active_at, CameraPipeline, Candidate, PpeReading, PpeStatus, RuleSpec},
};
use ls_ingest::Frame;
use ls_store::{EpisodeEvidence, NewEpisode, Store};
use std::{collections::HashMap, time::Duration};

#[derive(Default)]
pub(super) struct EpisodeRuntime {
    detector: EpisodeTracker,
    persisted: HashMap<EpisodeKey, Persisted>,
    closing: HashMap<EpisodeKey, (CloseReason, String)>,
}

struct Persisted {
    episode_id: i64,
    event_id: Option<i64>,
    sent_tier: AlertTier,
    behavior: String,
    peak: Option<FrameEvidence>,
}

struct FrameEvidence {
    jpeg: Vec<u8>,
    bbox: [f32; 4],
    overlay: Option<String>,
}

pub(super) struct EpisodeFrame<'a> {
    pub camera_id: i64,
    pub camera_name: &'a str,
    pub time: Duration,
    pub timestamp: &'a str,
    pub frame: &'a Frame,
    pub jpeg: Option<&'a [u8]>,
    pub dispatch: bool,
}

pub(super) fn episode_behavior(behavior: &str) -> bool {
    matches!(behavior, "enter_no_ppe" | "no_mask")
}

impl EpisodeRuntime {
    pub fn countdowns(&self, policy: &CameraPolicy) -> Vec<ls_analyzer::episode::EpisodeCountdown> {
        self.detector.countdowns(&policy.rule_grace)
    }

    pub fn recovery_check_due(&self) -> bool {
        self.detector.needs_recovery_check()
    }

    pub fn process(
        &mut self,
        store: &Store,
        sink: &EventSink,
        pipe: &CameraPipeline,
        policy: &CameraPolicy,
        frame: &EpisodeFrame<'_>,
    ) -> Result<Vec<(i64, Candidate, bool)>, String> {
        let applicable: Vec<_> = policy
            .rules
            .iter()
            .filter(|rule| {
                episode_behavior(&rule.behavior)
                    && rule_active_at(rule, frame.timestamp, &policy.calendar)
            })
            .collect();
        let mut closed = self
            .detector
            .remove_rules(|key| applicable.iter().any(|r| r.id == key.rule_id));
        closed.extend(self.detector.expire(frame.time));
        let mut output = Vec::new();
        for (track, zone) in pipe.observed_exits() {
            for rule in &applicable {
                let key = EpisodeKey {
                    rule_id: rule.id,
                    track_id: *track,
                    zone_id: Some(*zone),
                };
                if let Some(change) = self.detector.observe(
                    key,
                    Observation::Outside,
                    frame.time,
                    frame.timestamp,
                    Default::default(),
                ) {
                    closed.push((key, change));
                }
            }
        }
        self.finish_changes(
            store,
            sink,
            closed.drain(..),
            frame.camera_id,
            frame.timestamp,
        )?;
        for reading in pipe.ppe_readings() {
            for behavior in ["enter_no_ppe", "no_mask"] {
                let Some((rule, zone_id)) =
                    applicable_rule(&applicable, pipe, policy, reading, behavior)
                else {
                    continue;
                };
                let key = EpisodeKey {
                    rule_id: rule.id,
                    track_id: reading.track_id,
                    zone_id,
                };
                let (status, confidence) = if behavior == "no_mask" {
                    (
                        reading.observation.mask,
                        reading.observation.mask_violation_confidence,
                    )
                } else {
                    (
                        reading.observation.coat,
                        reading.observation.coat_violation_confidence,
                    )
                };
                let observation = match status {
                    PpeStatus::Wearing => Observation::Compliant,
                    PpeStatus::NoPpe
                        if !policy.spatial.suppress(
                            store,
                            frame.camera_id,
                            behavior,
                            reading.bbox,
                            frame.timestamp,
                        ) =>
                    {
                        Observation::Violation { confidence }
                    }
                    _ => Observation::Unknown,
                };
                let window = policy.rule_grace.get(&rule.id).copied().unwrap_or_default();
                let Some(change) =
                    self.detector
                        .observe(key, observation, frame.time, frame.timestamp, window)
                else {
                    continue;
                };
                match change {
                    EpisodeChange::Hit {
                        episode,
                        better_evidence,
                    } => {
                        if let Some(event) = self.hit(
                            store,
                            sink,
                            policy,
                            frame,
                            rule,
                            key,
                            *reading,
                            &episode,
                            better_evidence,
                        )? {
                            output.push(event);
                        }
                    }
                    change @ EpisodeChange::Closed { .. } => closed.push((key, change)),
                }
            }
        }
        self.finish_changes(store, sink, closed, frame.camera_id, frame.timestamp)?;
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn hit(
        &mut self,
        store: &Store,
        sink: &EventSink,
        policy: &CameraPolicy,
        frame: &EpisodeFrame<'_>,
        rule: &RuleSpec,
        key: EpisodeKey,
        reading: PpeReading,
        episode: &Episode,
        better_evidence: bool,
    ) -> Result<Option<(i64, Candidate, bool)>, String> {
        let evidence = EpisodeEvidence {
            first_ts: &episode.first_ts,
            last_ts: &episode.last_ts,
            duration_s: episode.duration.as_secs_f64(),
            hits: episode.hit_count.min(i64::MAX as u64) as i64,
            peak_conf: episode.peak_confidence,
            tier: episode.tier.as_str(),
        };
        let persisted = match self.persisted.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                store
                    .update_episode(entry.get().episode_id, &evidence)
                    .map_err(|e| e.to_string())?;
                entry.into_mut()
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                let episode_id = store
                    .add_episode(&NewEpisode {
                        camera_id: frame.camera_id,
                        rule_id: key.rule_id,
                        zone_id: key.zone_id,
                        track_id: key.track_id,
                        behavior: &rule.behavior,
                        evidence,
                    })
                    .map_err(|e| e.to_string())?;
                entry.insert(Persisted {
                    episode_id,
                    event_id: None,
                    sent_tier: AlertTier::Grace,
                    behavior: rule.behavior.clone(),
                    peak: None,
                })
            }
        };
        if better_evidence || (persisted.peak.is_none() && persisted.event_id.is_none()) {
            let jpeg = frame
                .jpeg
                .filter(|bytes| !bytes.is_empty())
                .map(<[u8]>::to_vec)
                .or_else(|| {
                    ls_analyzer::evidence::encode_jpeg_bgr(
                        &frame.frame.data,
                        frame.frame.width,
                        frame.frame.height,
                    )
                    .ok()
                });
            if let Some(jpeg) = jpeg {
                persisted.peak = Some(FrameEvidence {
                    jpeg,
                    bbox: reading.bbox,
                    overlay: detection_overlay(
                        frame.frame.width,
                        frame.frame.height,
                        Some(reading.bbox),
                        key.zone_id
                            .and_then(|id| policy.zones.iter().find(|zone| zone.id == id)),
                    ),
                });
            }
        }
        if let Some((event_id, peak)) = persisted.event_id.zip(persisted.peak.as_ref()) {
            let snapshot = sink.save_snapshot(frame.camera_id, frame.timestamp, &peak.jpeg)?;
            if let Err(error) = store.refresh_episode_snapshot(
                event_id,
                &snapshot.path,
                &snapshot.sha256,
                peak.overlay.as_deref(),
                peak.bbox,
            ) {
                let _ = std::fs::remove_file(snapshot.path);
                return Err(error.to_string());
            }
            persisted.peak = None;
        }
        if episode.tier == AlertTier::Grace || persisted.sent_tier == episode.tier {
            return Ok(None);
        }
        let level = if episode.tier == AlertTier::Warning {
            rule.level.clamp(1, 2)
        } else {
            rule.level.clamp(1, 3)
        };
        let condition = ls_dispatch::type_text(&rule.behavior);
        let detail = if episode.tier == AlertTier::Warning {
            format!("即将超出宽限。{condition}")
        } else {
            condition
        };
        let candidate = Candidate {
            camera_id: frame.camera_id,
            zone_id: key.zone_id,
            behavior: rule.behavior.clone(),
            level,
            track_id: key.track_id,
            detail,
            ts_iso: frame.timestamp.into(),
            evidence_key: format!("episode:{}", persisted.episode_id),
        };
        if let Some(event_id) = persisted.event_id {
            store
                .promote_episode_event(event_id, level, &candidate.detail)
                .map_err(|e| e.to_string())?;
            if let Some(event) = store.event_by_id(event_id).map_err(|e| e.to_string())? {
                let mut payload = super::event_json(&event);
                payload["op"] = serde_json::json!("episode_update");
                let _ = sink.st.events_tx.send(payload.to_string());
                if frame.dispatch {
                    sink.deliver(
                        event_id,
                        &candidate,
                        frame.camera_name,
                        event.snapshot.is_some(),
                        event.snapshot.as_deref(),
                        true,
                    );
                }
            }
            persisted.sent_tier = episode.tier;
            return Ok(Some((event_id, candidate, false)));
        }
        let peak = persisted.peak.as_ref();
        let id = sink.record_located(
            &candidate,
            peak.map(|frame| frame.jpeg.as_slice()),
            frame.dispatch,
            peak.map(|frame| frame.bbox),
            peak.and_then(|frame| frame.overlay.as_deref()),
            Some(persisted.episode_id),
        )?;
        if let Some(id) = id {
            persisted.event_id = Some(id);
            persisted.sent_tier = episode.tier;
            persisted.peak = None;
            return Ok(Some((id, candidate, true)));
        }
        Ok(None)
    }

    fn finish_changes(
        &mut self,
        store: &Store,
        sink: &EventSink,
        changes: impl IntoIterator<Item = (EpisodeKey, EpisodeChange)>,
        camera: i64,
        ts: &str,
    ) -> Result<(), String> {
        for (key, change) in changes {
            if let EpisodeChange::Closed { reason, .. } = change {
                self.closing
                    .entry(key)
                    .or_insert_with(|| (reason, ts.to_owned()));
            }
        }
        for key in self.closing.keys().copied().collect::<Vec<_>>() {
            let (reason, closed_at) = &self.closing[&key];
            if let Some(persisted) = self.persisted.get(&key) {
                close_persisted(store, sink, camera, key, persisted, closed_at, *reason)?;
            }
            self.persisted.remove(&key);
            self.closing.remove(&key);
        }
        Ok(())
    }

    pub fn interrupt(&mut self, store: &Store, sink: &EventSink, camera: i64, ts: &str) {
        let changes = self.detector.remove_rules(|_| false);
        if let Err(error) = self.finish_changes(store, sink, changes, camera, ts) {
            crate::logger::warn(&format!(
                "[ls-worker] cam-{camera} 关闭中断事件段失败: {error}"
            ));
        }
    }
}

fn close_persisted(
    store: &Store,
    sink: &EventSink,
    camera: i64,
    key: EpisodeKey,
    persisted: &Persisted,
    ts: &str,
    reason: CloseReason,
) -> Result<(), String> {
    let detail = if reason == CloseReason::Recovered {
        "已确认恢复合规"
    } else {
        "已确认离开规则区域"
    };
    let recovered = reason != CloseReason::Interrupted;
    let recovery = (recovered && persisted.event_id.is_some()).then_some(ls_store::NewEvent {
        camera_id: camera,
        type_: "alert_resolved",
        level: 0,
        track_id: key.track_id,
        detail,
        snapshot: None,
        ts,
    });
    let id = store
        .close_episode(persisted.episode_id, ts, reason.as_str(), recovery.as_ref())
        .map_err(|e| e.to_string())?;
    if let Some(id) = id {
        if let Some(event) = store.event_by_id(id).map_err(|e| e.to_string())? {
            let _ = sink
                .st
                .events_tx
                .send(super::event_json(&event).to_string());
        }
        let candidate = Candidate {
            camera_id: camera,
            zone_id: key.zone_id,
            behavior: "alert_resolved".into(),
            level: 0,
            track_id: key.track_id,
            detail: detail.into(),
            ts_iso: ts.into(),
            evidence_key: format!("episode:{}:resolved", persisted.episode_id),
        };
        let camera_name = store
            .camera(camera)
            .map_err(|e| e.to_string())?
            .map(|camera| camera.name)
            .unwrap_or_default();
        sink.deliver_deferred(id, &candidate, &camera_name, false, None);
    } else if recovered && persisted.event_id.is_none() {
        crate::spatial::audit(
            store,
            camera,
            "grace_recovered",
            persisted.episode_id,
            &persisted.behavior,
            ts,
        );
    }
    Ok(())
}

fn applicable_rule<'a>(
    rules: &[&'a RuleSpec],
    pipe: &CameraPipeline,
    policy: &CameraPolicy,
    reading: &PpeReading,
    behavior: &str,
) -> Option<(&'a RuleSpec, Option<i64>)> {
    let matches = |rule: &&RuleSpec| rule.behavior == behavior;
    if let Some(rule) = rules
        .iter()
        .copied()
        .filter(matches)
        .find(|rule| rule.zone_id > 0 && pipe.confirmed_in_zone(reading.track_id, rule.zone_id))
    {
        return Some((rule, Some(rule.zone_id)));
    }
    let rule = rules
        .iter()
        .copied()
        .filter(matches)
        .find(|rule| rule.zone_id == 0)?;
    if behavior == "no_mask" {
        return Some((rule, None));
    }
    let zone = policy
        .zones
        .iter()
        .find(|zone| pipe.confirmed_in_zone(reading.track_id, zone.id))?;
    Some((rule, Some(zone.id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::helpers::{seed_schema, test_state_custom};
    use ls_analyzer::pipeline::{CameraSpec, PersonDetection, PpeObservation, PpeRunner};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct FixturePpe(Arc<AtomicUsize>);

    impl PpeRunner for FixturePpe {
        fn statuses(&mut self, _: &[u8], _: u32, _: u32, boxes: &[[f32; 4]]) -> Vec<PpeStatus> {
            vec![PpeStatus::Unknown; boxes.len()]
        }

        fn observations(
            &mut self,
            _: &[u8],
            _: u32,
            _: u32,
            boxes: &[[f32; 4]],
        ) -> Vec<PpeObservation> {
            let stage = self.0.load(Ordering::Relaxed);
            vec![
                PpeObservation {
                    mask: if stage >= 2 {
                        PpeStatus::Wearing
                    } else {
                        PpeStatus::NoPpe
                    },
                    mask_violation_confidence: if stage == 1 { 0.95 } else { 0.74 },
                    ..Default::default()
                };
                boxes.len()
            ]
        }
    }

    #[test]
    fn ten_minute_episode_uses_one_event_and_records_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("episodes.db");
        seed_schema(&db);
        let root = temp.path().to_str().unwrap();
        let store = Store::open(db.to_str().unwrap()).unwrap();
        let cid = store
            .conn_add_camera("Camera", "sample.mp4", "file")
            .unwrap();
        let rule = store
            .add_rule(cid, None, "no_mask", 3, 60, None, None, 0, true)
            .unwrap();
        store.set_rule_grace(rule, 10, 0.7).unwrap();
        let policy = super::super::read_camera_policy(&store, cid).unwrap();
        let state = test_state_custom(db.to_str().unwrap(), "", false, root, true, "");
        let sink = EventSink::new(state);
        let mut pipe = CameraPipeline::new(
            CameraSpec {
                id: cid,
                name: "Camera".into(),
                source: "sample.mp4".into(),
                type_: "file".into(),
                analysis_fps: 5.0,
                file_loop: false,
            },
            policy.zones.clone(),
            policy.rules.clone(),
            policy.calendar.clone(),
            2,
            3,
        );
        pipe.set_continuous_ppe(true);
        let stage = Arc::new(AtomicUsize::new(0));
        pipe.set_ppe(Some(Box::new(FixturePpe(Arc::clone(&stage)))));
        let frame = Frame {
            index: 1,
            width: 64,
            height: 64,
            data: vec![108; 64 * 64 * 3],
        };
        let det = PersonDetection {
            box_: [6.0, 6.0, 45.0, 58.0],
            score: 0.99,
            ppe: PpeStatus::Unknown,
        };
        let mut runtime = EpisodeRuntime::default();
        let mut sample = |seconds| {
            let timestamp = format!("2026-11-24T10:{:02}:{:02}", seconds / 60, seconds % 60);
            let events = pipe.process_with_recheck(
                &[det],
                &timestamp,
                Some((&frame.data, frame.width, frame.height)),
                false,
                true,
            );
            assert!(events.is_empty(), "legacy pipeline duplicated the PPE rule");
            runtime
                .process(
                    &store,
                    &sink,
                    &pipe,
                    &policy,
                    &EpisodeFrame {
                        camera_id: cid,
                        camera_name: "Camera",
                        time: Duration::from_secs(seconds),
                        timestamp: &timestamp,
                        frame: &frame,
                        jpeg: None,
                        dispatch: false,
                    },
                )
                .unwrap()
        };
        for sec in [0, 2, 4, 6] {
            assert!(sample(sec).is_empty());
        }
        let warning = sample(8);
        assert_eq!(warning.len(), 1);
        assert_eq!(warning[0].1.level, 2);
        let id = warning[0].0;
        let initial = store.event_by_id(id).unwrap().unwrap();
        assert_eq!(initial.alert_tier, "warning");
        let upgraded = sample(10);
        assert_eq!(upgraded[0].0, id);
        assert_eq!(upgraded[0].1.level, 3);
        stage.store(1, Ordering::Relaxed);
        for sec in (12..=600).step_by(2) {
            assert!(sample(sec).is_empty());
        }
        let current = store.event_by_id(id).unwrap().unwrap();
        assert_eq!(current.hit_count, 301);
        assert_eq!(current.duration_s, 600.0);
        assert!((current.peak_conf.unwrap() - 0.95).abs() < 0.0001);
        assert_ne!(current.snapshot, initial.snapshot);
        assert_eq!(store.count_events().unwrap(), 1);
        stage.store(2, Ordering::Relaxed);
        sample(602);
        assert!(store
            .event_by_id(id)
            .unwrap()
            .unwrap()
            .resolved_ts
            .is_none());
        sample(603);
        let closed = store.event_by_id(id).unwrap().unwrap();
        assert_eq!(closed.resolved_ts.as_deref(), Some("2026-11-24T10:10:03"));
        let history = store.events(10, 0, Some(cid), None).unwrap();
        assert_eq!(history[0].type_, "alert_resolved");
        assert_eq!(history[0].episode_id, closed.episode_id);
        assert_eq!(history.len(), 2);
    }
}
