use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ls_analyzer::{evidence::Evidence, pipeline::Candidate};
use ls_store::Store;
use serde_json::{json, Value};

use super::{spawn_finalize, EventSink};

const MAX_CONCURRENT_REVIEWS: usize = 4;
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

struct Slot;

impl Slot {
    fn acquire() -> Option<Self> {
        IN_FLIGHT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_CONCURRENT_REVIEWS).then_some(active + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::Release);
    }
}

pub(super) struct CascadeReview {
    pub camera_id: i64,
    pub cfg: crate::detect::DetectCfg,
    pub jpeg: axum::body::Bytes,
    pub bounds: Option<[f32; 4]>,
    pub track_scope: Option<(u64, i64)>,
    pub detected_people: usize,
    pub ev_round: Vec<(i64, Candidate)>,
    pub suppress: bool,
    pub evidence: Option<Arc<Evidence>>,
    pub evidence_key_prefix: String,
}

type TrackReviews = HashMap<(u64, i64), (Option<i64>, i64)>;

#[derive(Default)]
pub(super) struct ReviewWindows(Mutex<TrackReviews>);

impl ReviewWindows {
    fn reserve(
        &self,
        store: &Store,
        cfg: &crate::detect::DetectCfg,
        track: Option<(u64, i64)>,
        person: Option<i64>,
        now: i64,
    ) -> Result<(), &'static str> {
        let mut tracks = self.0.lock().map_err(|_| "review_window_unavailable")?;
        let window = crate::detect::person_review_window(store) as i64;
        let active = |sent_at: i64| window > 0 && sent_at > now.saturating_sub(window);
        tracks.retain(|_, (_, sent_at)| {
            *sent_at > now.saturating_sub(crate::detect::MAX_PERSON_REVIEW_WINDOW_S as i64)
        });

        let db_error = |_| "review_window_unavailable";
        let person_sent = person
            .map(|id| store.face_review_sent_at(id).map_err(db_error))
            .transpose()?
            .flatten()
            .filter(|&sent_at| active(sent_at));
        let track_sent =
            track
                .and_then(|key| tracks.get(&key).copied())
                .filter(|(known, sent_at)| {
                    active(*sent_at) && (person.is_none() || known.is_none() || *known == person)
                });
        if let Some(sent_at) = person_sent.max(track_sent.map(|(_, sent)| sent)) {
            if let Some(id) = person.filter(|_| person_sent != Some(sent_at)) {
                store
                    .record_face_review(id, sent_at, now)
                    .map_err(db_error)?;
            }
            if let Some(key) = track {
                tracks.insert(
                    key,
                    (
                        person.or_else(|| track_sent.and_then(|(known, _)| known)),
                        sent_at,
                    ),
                );
            }
            return Err("person_review_window");
        }

        if crate::detect::usage_try_consume(store, cfg, true).0 {
            return Err("budget_exhausted");
        }
        if let Some(id) = person {
            store.record_face_review(id, now, now).map_err(db_error)?;
        }
        if let Some(key) = track {
            tracks.insert(key, (person, now));
        }
        Ok(())
    }
}

pub(super) fn group_events(events: Vec<(i64, Candidate)>) -> BTreeMap<i64, Vec<(i64, Candidate)>> {
    let mut groups: BTreeMap<i64, Vec<_>> = BTreeMap::new();
    for event in events {
        groups.entry(event.1.track_id).or_default().push(event);
    }
    groups
}

pub(super) fn keep_geometry_local(
    store: &Store,
    sink: &EventSink,
    events: &mut Vec<(i64, Candidate)>,
    suppress: bool,
) {
    events.retain(|event| {
        // These decisions already have trusted local timing, counts, and zone coordinates.
        if !matches!(
            event.1.behavior.as_str(),
            "enter" | "exit" | "occupancy_over" | "solo_after_hours"
        ) {
            return true;
        }
        finish_cascade_events(
            store,
            sink,
            std::slice::from_ref(event),
            suppress,
            &json!({"status":"unreviewed","reason":"local_geometry_rule"}),
        );
        false
    });
}

pub(super) fn submit_cascade_review(store: &Store, sink: &EventSink, review: CascadeReview) {
    let skipped = |review: &CascadeReview, reason: &str| {
        finish_cascade_events(
            store,
            sink,
            &review.ev_round,
            review.suppress,
            &json!({"status": "unreviewed", "reason": reason}),
        );
    };
    if review.jpeg.is_empty() {
        skipped(&review, "no_snapshot");
        return;
    }
    if !review.cfg.compliance_ok() {
        skipped(&review, "remote_not_allowed");
        return;
    }
    let Some(slot) = Slot::acquire() else {
        skipped(&review, "review_busy");
        return;
    };
    let review = Arc::new(review);
    let task = Arc::clone(&review);
    let st = Arc::clone(&sink.st);
    let spawned = std::thread::Builder::new()
        .name(format!("ls-cascade-{}", review.camera_id))
        .spawn(move || {
            let _slot = slot;
            let sink = EventSink::new(st).use_cascade(true);
            match Store::open(&sink.st.db_path) {
                Ok(store) => run(&store, &sink, &task),
                Err(error) => {
                    crate::logger::error(&format!("[ls-cascade] 数据库打开失败: {error}"))
                }
            }
        });
    if let Err(error) = spawned {
        crate::logger::error(&format!("[ls-cascade] 无法启动复核: {error}"));
        skipped(&review, "worker_start_failed");
    }
}

fn event_verdict(scene: &Value, behavior: &str, single_target: bool) -> &'static str {
    let label = match behavior {
        "enter_no_ppe" => "no_ppe",
        "no_mask" => "no_mask",
        _ => return "uncertain",
    };
    if !single_target || scene["persons"].as_i64() != Some(1) {
        return "uncertain";
    }
    let has = |field: &str| {
        scene[field]
            .as_array()
            .is_some_and(|labels| labels.iter().any(|item| item.as_str() == Some(label)))
    };
    let violation =
        has("violations") || (label == "no_ppe" && scene["no_ppe"].as_i64().unwrap_or(0) > 0);
    match (violation, has("compliant")) {
        (true, false) => "likely_true",
        (false, true) => "likely_false",
        _ => "uncertain",
    }
}

pub(super) fn finish_cascade_events(
    store: &Store,
    sink: &EventSink,
    events: &[(i64, Candidate)],
    suppress: bool,
    scene: &Value,
) {
    let now = crate::util::iso_now();
    let single_target = scene["single_local_target"] == true
        && events.first().is_some_and(|(_, first)| {
            events
                .iter()
                .all(|(_, event)| event.track_id == first.track_id)
        });
    for (eid, candidate) in events {
        let mut result = scene.clone();
        if scene.get("status").is_none() {
            result["verdict"] = json!(event_verdict(scene, &candidate.behavior, single_target));
        }
        let suppressed = suppress && result["verdict"] == "likely_false";
        match store.finish_event_vlm(*eid, &result.to_string(), &now, suppressed) {
            Ok(Some((level, detail))) if suppress && !suppressed => {
                let mut candidate = candidate.clone();
                candidate.level = level;
                candidate.detail = detail;
                sink.deliver_deferred_pub(*eid, &candidate, false);
            }
            Ok(_) => {}
            Err(error) => {
                crate::logger::error(&format!("[ls-cascade] 事件 #{eid} 复核保存失败: {error}"));
                if suppress {
                    sink.deliver_deferred_pub(*eid, candidate, false);
                }
            }
        }
    }
}

fn run(store: &Store, sink: &EventSink, review: &CascadeReview) {
    let report_skip = |reason: &str| {
        finish_cascade_events(
            store,
            sink,
            &review.ev_round,
            review.suppress,
            &json!({"status":"unreviewed", "reason":reason}),
        );
    };
    if review.bounds.is_none() && review.detected_people != 1 {
        report_skip("target_not_isolated");
        return;
    }
    let Ok(photo) = crate::local_face::Photo::decode_person(&review.jpeg, review.bounds) else {
        report_skip("no_snapshot");
        return;
    };
    let person = crate::local_face::review_subject(
        &sink.st,
        store,
        review.camera_id,
        &review.jpeg,
        review.bounds,
    );
    let now = crate::local_face::unix_now();
    if let Some(person) = &person {
        for (event, _) in &review.ev_round {
            let _ = store.save_face_match(*event, person.id, person.similarity, now);
        }
    }
    let Ok(jpeg) = photo.jpeg() else {
        report_skip("no_snapshot");
        return;
    };
    if let Err(reason) = sink.st.workers.review_windows.reserve(
        store,
        &review.cfg,
        review.track_scope,
        person.map(|person| person.id),
        now,
    ) {
        report_skip(reason);
        return;
    }
    let mut cfg = review.cfg.to_vlm_cfg();
    cfg.enabled = true;
    let prompt = format!(
        "{}\n本轮只有一张图，按以下 JSON 取值复核。violations 可包含 no_ppe（未穿白大褂）和 no_mask（未正确佩戴口罩）。\
         compliant 数组仅在可清楚判断画面里唯一一人的穿戴时填写，no_ppe 表示实际已穿白大褂，no_mask 表示实际已正确戴口罩。\
         遮挡、背对、太小、无法区分或多人时，compliant 留空。",
        review.cfg.effective_prompt(),
    );
    let mut result = match crate::vlm::analyze_frames(&cfg, &prompt, std::slice::from_ref(&jpeg)) {
        Ok(result) => result,
        Err(error) => {
            crate::logger::error(&format!(
                "[ls-cascade] cam-{} 复核失败: {error}",
                review.camera_id
            ));
            json!({"status": "error", "reason": error})
        }
    };
    result["single_local_target"] = json!(true);
    finish_cascade_events(store, sink, &review.ev_round, review.suppress, &result);

    // A gray-area probe can discover a rule the local model missed. A review of
    // an existing event only updates that event's evidence and decision.
    if !review.ev_round.is_empty() {
        return;
    }
    for behavior in result["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let behavior = if behavior == "no_ppe" {
            "enter_no_ppe"
        } else {
            behavior
        };
        let candidate = Candidate {
            camera_id: review.camera_id,
            zone_id: None,
            behavior: behavior.into(),
            level: 2,
            track_id: 0,
            detail: format!(
                "AI 精判：{}。{}",
                ls_dispatch::type_text(behavior),
                result["summary"].as_str().unwrap_or_default()
            ),
            ts_iso: crate::util::iso_now(),
            evidence_key: format!("{}:{behavior}", review.evidence_key_prefix),
        };
        if let Ok(Some(eid)) = sink.record(&candidate, Some(&jpeg)) {
            if let Some(evidence) = &review.evidence {
                spawn_finalize(&sink.st, evidence, eid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn person_window_follows_confirmed_identity_without_merging_people() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("review-windows.db");
        let store = Store::open(db.to_str().unwrap()).unwrap();
        store.ensure_schema().unwrap();
        let enroll = |subject: &str| {
            store
                .save_face_person(
                    &ls_store::FacePerson {
                        id: 0,
                        profile: "local".into(),
                        subject: subject.into(),
                        remote_subject: None,
                        consent_ts: "2026-11-24T10:00:00".into(),
                        expires_at: 10_000,
                        removing: false,
                        samples: 0,
                    },
                    &[],
                )
                .unwrap()
        };
        let alice = enroll("fixture-a");
        let bob = enroll("fixture-b");
        let cfg = crate::detect::DetectCfg::default();
        let windows = ReviewWindows::default();
        assert!(windows
            .reserve(&store, &cfg, Some((1, 5)), Some(alice), 1_000)
            .is_ok());
        let reopened = Store::open(db.to_str().unwrap()).unwrap();
        let after_restart = ReviewWindows::default();
        assert_eq!(
            after_restart.reserve(&reopened, &cfg, Some((7, 3)), Some(alice), 1_040),
            Err("person_review_window")
        );
        assert_eq!(
            after_restart.reserve(&reopened, &cfg, None, Some(alice), 1_050),
            Err("person_review_window")
        );
        assert!(after_restart
            .reserve(&reopened, &cfg, Some((8, 3)), Some(bob), 1_050)
            .is_ok());
        assert!(after_restart
            .reserve(&reopened, &cfg, Some((7, 4)), None, 1_060)
            .is_ok());
        assert!(after_restart
            .reserve(&reopened, &cfg, Some((8, 4)), None, 1_061)
            .is_ok());
        assert_eq!(
            after_restart.reserve(&reopened, &cfg, Some((7, 4)), None, 1_080),
            Err("person_review_window")
        );
        assert_eq!(crate::detect::usage_today(&store, &cfg).count, 4);
        assert!(after_restart
            .reserve(&reopened, &cfg, Some((7, 5)), Some(alice), 1_300)
            .is_ok());
        store.delete_face_person(bob).unwrap();
        assert_eq!(store.face_review_sent_at(bob).unwrap(), None);
    }

    #[test]
    fn cascade_requires_explicit_single_person_evidence_before_suppression() {
        let unknown = json!({"persons":0,"violations":[]});
        assert_eq!(event_verdict(&unknown, "no_mask", true), "uncertain");
        let compliant = json!({"persons":1,"compliant":["no_ppe"],"violations":[]});
        assert_eq!(event_verdict(&compliant, "no_mask", true), "uncertain");
        assert_eq!(
            event_verdict(&compliant, "enter_no_ppe", false),
            "uncertain"
        );
        assert_eq!(
            event_verdict(&compliant, "enter_no_ppe", true),
            "likely_false"
        );
        let conflicting = json!({"persons":1,"compliant":["no_ppe"],"violations":["no_ppe"]});
        assert_eq!(
            event_verdict(&conflicting, "enter_no_ppe", true),
            "uncertain"
        );
    }
}
