use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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
    pub jpeg: Vec<u8>,
    pub detected_people: usize,
    pub ev_round: Vec<(i64, Candidate)>,
    pub suppress: bool,
    pub evidence: Option<Arc<Evidence>>,
    pub evidence_key_prefix: String,
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
    let (blocked, _) = crate::detect::usage_try_consume(store, &review.cfg, true);
    if blocked {
        finish_cascade_events(
            store,
            sink,
            &review.ev_round,
            review.suppress,
            &json!({"status":"unreviewed", "reason":"budget_exhausted"}),
        );
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
    let mut result =
        match crate::vlm::analyze_frames(&cfg, &prompt, std::slice::from_ref(&review.jpeg)) {
            Ok(result) => result,
            Err(error) => {
                crate::logger::error(&format!(
                    "[ls-cascade] cam-{} 复核失败: {error}",
                    review.camera_id
                ));
                json!({"status": "error", "reason": error})
            }
        };
    result["single_local_target"] = json!(review.detected_people == 1);
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
        if let Ok(Some(eid)) = sink.record(&candidate, Some(&review.jpeg)) {
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
