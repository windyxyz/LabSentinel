use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    state::Shared,
    util::{detail_err, with_store},
    worker::EventSink,
};

const HOLD: Duration = Duration::from_millis(300);
const MAX_GAP: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct ThermalRuntime(Mutex<HashMap<i64, ThermalReading>>);

#[derive(Default)]
struct ThermalReading {
    above_since: Option<Instant>,
    last: Option<Instant>,
    alarm: bool,
    rule: i64,
    threshold: i64,
}

impl ThermalReading {
    fn observe(&mut self, now: Instant, value: f32) -> bool {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) > MAX_GAP)
        {
            self.above_since = None;
        }
        self.last = Some(now);
        if value < self.threshold as f32 {
            self.above_since = None;
            if value <= self.threshold as f32 - 3.0 {
                self.alarm = false;
            }
            return false;
        }
        let since = *self.above_since.get_or_insert(now);
        !self.alarm && now.saturating_duration_since(since) >= HOLD
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ThermalSample {
    camera_id: i64,
    celsius: f32,
}

pub(crate) async fn thermal_sample(
    State(st): State<Shared>,
    Json(sample): Json<ThermalSample>,
) -> Response {
    if !(-40.0..=1_000.0).contains(&sample.celsius) {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "物体温度超出有效范围")
            .into_response();
    }
    with_store(&st, |store| {
        if store.camera(sample.camera_id)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "未找到关联摄像头").into_response());
        }
        let mut readings = st.thermal.0.lock().unwrap_or_else(|e| e.into_inner());
        let ts = ls_core::time::local_now_iso();
        let policy = match crate::worker::read_camera_policy(store, sample.camera_id) {
            Ok(policy) => policy,
            Err(_) => {
                return Ok(
                    detail_err(StatusCode::INTERNAL_SERVER_ERROR, "读取温度规则失败")
                        .into_response(),
                )
            }
        };
        let rule = policy
            .rules
            .iter()
            .filter(|rule| {
                rule.behavior == "thermal_overheat"
                    && ls_analyzer::pipeline::rule_active_at(rule, &ts, &policy.calendar)
            })
            .max_by_key(|rule| (rule.level, std::cmp::Reverse(rule.id)));
        let Some(rule) = rule else {
            readings.remove(&sample.camera_id);
            return Ok(Json(json!({"accepted": false, "reason": "no_active_rule"})).into_response());
        };
        let threshold = if rule.threshold == 0 {
            55
        } else {
            rule.threshold.clamp(1, 1_000)
        };
        let reading = readings.entry(sample.camera_id).or_default();
        if reading.rule != rule.id || reading.threshold != threshold {
            *reading = ThermalReading {
                rule: rule.id,
                threshold,
                ..Default::default()
            };
        }
        let mut event_id = None;
        if reading.observe(Instant::now(), sample.celsius) {
            let candidate = ls_analyzer::pipeline::Candidate {
                camera_id: sample.camera_id,
                zone_id: None,
                behavior: "thermal_overheat".into(),
                level: rule.level,
                track_id: 0,
                ts_iso: ts.clone(),
                detail: format!(
                    "传感器温度 {:.1} °C，达到 {} °C 阈值",
                    sample.celsius, threshold
                ),
                evidence_key: format!("thermal:{}:{ts}", rule.id),
            };
            match EventSink::new(st.clone()).record(&candidate, None) {
                Ok(id) => {
                    event_id = id;
                    reading.alarm = true;
                }
                Err(_) => {
                    return Ok(detail_err(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "温度告警写入失败，请重试",
                    )
                    .into_response())
                }
            }
        }
        Ok(
            Json(json!({"accepted": true, "event_id": event_id, "alarm": reading.alarm}))
                .into_response(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thermal_alarm_requires_a_hold_and_rearms_after_cooldown() {
        let mut reading = ThermalReading {
            threshold: 55,
            ..Default::default()
        };
        let start = Instant::now();
        assert!(!reading.observe(start, 55.0));
        assert!(reading.observe(start + HOLD, 58.0));
        reading.alarm = true;
        assert!(!reading.observe(start + HOLD * 2, 54.0));
        assert!(reading.alarm);
        assert!(!reading.observe(start + HOLD * 3, 52.0));
        assert!(!reading.alarm);
        assert!(!reading.observe(start + MAX_GAP * 2, 57.0));
    }
}
