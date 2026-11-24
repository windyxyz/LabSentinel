use std::{collections::HashMap, time::Duration};

use ls_analyzer::pipeline::{rule_active_at, CameraPipeline, Candidate};
use ls_infer::pose::{Pose, PoseModel, POSENET_FILE};

use super::{episodes::EpisodeFrame, CameraPolicy};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(400);
const MAX_GAP: Duration = Duration::from_millis(1250);

#[derive(Clone, Copy)]
struct Body {
    upright: bool,
    horizontal: bool,
    hip: [f32; 2],
    height: f32,
}

impl Body {
    fn from_pose(pose: &Pose) -> Option<Self> {
        let pair = |a: usize, b: usize| {
            let [ax, ay, ac] = pose.keypoints[a];
            let [bx, by, bc] = pose.keypoints[b];
            (ac.min(bc) >= 0.40).then_some([(ax + bx) / 2.0, (ay + by) / 2.0])
        };
        let shoulder = pair(5, 6)?;
        let hip = pair(11, 12)?;
        let dx = (shoulder[0] - hip[0]).abs();
        let dy = (shoulder[1] - hip[1]).abs();
        let [x1, y1, x2, y2] = pose.bounds;
        let (width, height) = (x2 - x1, y2 - y1);
        if height < 24.0 || width < 12.0 || dx.hypot(dy) < 12.0 {
            return None;
        }
        Some(Self {
            upright: dy > dx * 1.5 && height > width * 1.2,
            horizontal: dx > dy * 1.5 && width > height * 1.15,
            hip,
            height,
        })
    }
}

#[derive(Default)]
struct Transition {
    last_upright: Option<(Duration, Body)>,
    down: Option<(Duration, Body)>,
    last: Option<Duration>,
    last_alert: Option<Duration>,
    reported: bool,
}

impl Transition {
    fn observe(&mut self, now: Duration, body: Body, hold: Duration) -> bool {
        if self.last.is_some_and(|t| now < t || now - t > MAX_GAP) {
            *self = Self {
                last_alert: self.last_alert,
                ..Default::default()
            };
        }
        self.last = Some(now);
        if body.upright {
            self.last_upright = Some((now, body));
            self.down = None;
            self.reported = false;
        } else if body.horizontal && !self.reported {
            if let Some((since, previous)) = self.down {
                let displacement =
                    (body.hip[0] - previous.hip[0]).hypot(body.hip[1] - previous.hip[1]);
                if displacement > previous.height * 0.28 {
                    self.down = Some((now, body));
                } else if now - since >= hold {
                    self.reported = true;
                    return true;
                }
            } else if self.last_upright.is_some_and(|(t, standing)| {
                now - t <= Duration::from_secs(2)
                    && body.hip[1] - standing.hip[1] > standing.height * 0.18
                    && body.height < standing.height * 0.82
            }) {
                self.down = Some((now, body));
            }
        } else if !body.horizontal {
            self.down = None;
        }
        false
    }
}

#[derive(Default)]
pub(super) struct Falls {
    model: Option<PoseModel>,
    next_sample: Duration,
    transitions: HashMap<(i64, i64), Transition>,
}

impl Falls {
    pub fn observe(
        &mut self,
        policy: &CameraPolicy,
        pipe: &CameraPipeline,
        sample: &EpisodeFrame<'_>,
        gpu: bool,
    ) -> Vec<Candidate> {
        let active: Vec<_> = policy
            .rules
            .iter()
            .filter(|rule| {
                rule.behavior == "person_fall"
                    && rule_active_at(rule, sample.timestamp, &policy.calendar)
            })
            .collect();
        self.transitions.retain(|&(rule, track), _| {
            active.iter().any(|item| item.id == rule) && pipe.can_resume_track(track)
        });
        if active.is_empty() {
            self.model = None;
            return Vec::new();
        }
        if sample.time < self.next_sample || pipe.last_tracks().is_empty() {
            return Vec::new();
        }
        self.next_sample = sample.time + SAMPLE_INTERVAL;
        if self.model.is_none() {
            let path = crate::deploy::default_install_root()
                .join("models")
                .join(POSENET_FILE);
            match PoseModel::load(&path, gpu) {
                Ok(model) => self.model = Some(model),
                Err(error) => {
                    self.next_sample = sample.time + Duration::from_secs(30);
                    crate::logger::error(&format!("[pose] cam-{} {error}", sample.camera_id));
                    return Vec::new();
                }
            }
        }
        let frame = sample.frame;
        let poses =
            match self
                .model
                .as_mut()
                .unwrap()
                .detect(&frame.data, frame.width, frame.height)
            {
                Ok(poses) => poses,
                Err(error) => {
                    self.next_sample = sample.time + Duration::from_secs(5);
                    crate::logger::error(&format!("[pose] cam-{} {error}", sample.camera_id));
                    return Vec::new();
                }
            };
        let mut events = Vec::new();
        for pose in &poses {
            let mut matches = pipe
                .last_tracks()
                .iter()
                .filter(|t| !t.occluded && pose.overlap(&t.bbox) >= 0.35);
            let Some(track) = matches.next() else {
                continue;
            };
            if matches.next().is_some()
                || poses
                    .iter()
                    .filter(|p| p.overlap(&track.bbox) >= 0.35)
                    .count()
                    > 1
            {
                continue;
            }
            let Some(body) = Body::from_pose(pose) else {
                continue;
            };
            for rule in &active {
                if rule.zone_id != 0 && !pipe.confirmed_in_zone(track.id, rule.zone_id) {
                    self.transitions.remove(&(rule.id, track.id));
                    continue;
                }
                let hold = Duration::from_secs(if rule.threshold > 0 {
                    rule.threshold.clamp(2, 60) as u64
                } else {
                    3
                });
                let state = self.transitions.entry((rule.id, track.id)).or_default();
                let cooldown = Duration::from_secs(rule.cooldown_s.max(0) as u64);
                if state.observe(sample.time, body, hold)
                    && state
                        .last_alert
                        .is_none_or(|last| sample.time.saturating_sub(last) >= cooldown)
                {
                    state.last_alert = Some(sample.time);
                    events.push(Candidate {
                        camera_id: sample.camera_id,
                        zone_id: (rule.zone_id != 0).then_some(rule.zone_id),
                        behavior: "person_fall".into(),
                        level: rule.level,
                        track_id: track.id,
                        detail: format!("检测到快速倒地后持续 {} 秒。请确认现场。", hold.as_secs()),
                        ts_iso: sample.timestamp.into(),
                        evidence_key: format!("fall:{}:{}", rule.id, frame.index),
                    });
                }
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fall_requires_recent_upright_drop_and_continuous_horizontal_hold() {
        let upright = Body {
            upright: true,
            horizontal: false,
            hip: [0.0, 30.0],
            height: 100.0,
        };
        let prone = Body {
            upright: false,
            horizontal: true,
            hip: [10.0, 70.0],
            height: 35.0,
        };
        let mut state = Transition::default();
        let hold = Duration::from_secs(3);
        let sec = Duration::from_secs;
        assert!(!state.observe(sec(0), prone, hold));
        assert!(!state.observe(sec(1), upright, hold));
        for time in 2..5 {
            assert!(!state.observe(sec(time), prone, hold));
        }
        assert!(state.observe(sec(5), prone, hold));
        assert!(!state.observe(sec(6), prone, hold));
        assert!(!state.observe(sec(7), upright, hold));
        assert!(!state.observe(sec(10), prone, hold));
        for time in 11..16 {
            assert!(!state.observe(sec(time), prone, hold));
        }
    }
}
