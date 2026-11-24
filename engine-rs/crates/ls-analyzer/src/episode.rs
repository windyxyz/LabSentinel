//! Temporal state for repeated observations of the same rule, track and region.

use std::{collections::HashMap, time::Duration};

pub const REVIEW_INTERVAL: Duration = Duration::from_secs(2);
const MAX_OBSERVATION_GAP: Duration = Duration::from_secs(6);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EpisodeKey {
    pub rule_id: i64,
    pub track_id: i64,
    pub zone_id: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraceWindow {
    pub seconds: u32,
    pub warning_percent: u8,
}

impl Default for GraceWindow {
    fn default() -> Self {
        Self {
            seconds: 0,
            warning_percent: 70,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertTier {
    Grace,
    Warning,
    Alert,
}

impl AlertTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grace => "grace",
            Self::Warning => "warning",
            Self::Alert => "alert",
        }
    }
}

impl GraceWindow {
    fn tier(self, elapsed: Duration) -> AlertTier {
        let full = Duration::from_secs(self.seconds.into());
        if elapsed >= full {
            AlertTier::Alert
        } else if elapsed >= full * u32::from(self.warning_percent) / 100 {
            AlertTier::Warning
        } else {
            AlertTier::Grace
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Observation {
    Violation { confidence: f32 },
    Compliant,
    ConfirmedCompliant,
    Outside,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Episode {
    pub first_ts: String,
    pub last_ts: String,
    pub duration: Duration,
    pub hit_count: u64,
    pub peak_confidence: f32,
    pub tier: AlertTier,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseReason {
    Recovered,
    LeftZone,
    Interrupted,
}

impl CloseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recovered => "recovered",
            Self::LeftZone => "left_zone",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug)]
pub enum EpisodeChange {
    Hit {
        episode: Episode,
        better_evidence: bool,
    },
    Closed {
        episode: Episode,
        reason: CloseReason,
    },
}

struct ActiveEpisode {
    public: Episode,
    first: Duration,
    last_confirmed: Duration,
    confirmed_violation: Option<Duration>,
    confirmed_for: Duration,
    compliant: bool,
}

#[derive(Default)]
pub struct EpisodeTracker {
    active: HashMap<EpisodeKey, ActiveEpisode>,
}

#[derive(serde::Serialize)]
pub struct EpisodeCountdown {
    pub track_id: i64,
    pub rule_id: i64,
    pub remaining_s: u32,
    pub tier: AlertTier,
}

impl EpisodeTracker {
    pub fn countdowns(&self, windows: &HashMap<i64, GraceWindow>) -> Vec<EpisodeCountdown> {
        self.active
            .iter()
            .filter_map(|(key, entry)| {
                let window = windows.get(&key.rule_id)?;
                let remaining = f64::from(window.seconds) - entry.confirmed_for.as_secs_f64();
                (entry.public.tier != AlertTier::Alert && remaining > 0.0).then_some(
                    EpisodeCountdown {
                        track_id: key.track_id,
                        rule_id: key.rule_id,
                        remaining_s: remaining.ceil() as u32,
                        tier: entry.public.tier,
                    },
                )
            })
            .collect()
    }

    pub fn observe(
        &mut self,
        key: EpisodeKey,
        observation: Observation,
        clock: Duration,
        timestamp: &str,
        window: GraceWindow,
    ) -> Option<EpisodeChange> {
        match observation {
            Observation::Violation { confidence } => {
                let confidence = if confidence.is_finite() {
                    confidence.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let entry = self.active.entry(key).or_insert_with(|| ActiveEpisode {
                    public: Episode {
                        first_ts: timestamp.to_owned(),
                        last_ts: timestamp.to_owned(),
                        duration: Duration::ZERO,
                        hit_count: 0,
                        peak_confidence: confidence,
                        tier: window.tier(Duration::ZERO),
                    },
                    first: clock,
                    last_confirmed: clock,
                    confirmed_violation: None,
                    confirmed_for: Duration::ZERO,
                    compliant: false,
                });
                if let Some(previous) = entry.confirmed_violation {
                    let gap = clock.saturating_sub(previous);
                    if gap <= MAX_OBSERVATION_GAP {
                        entry.confirmed_for += gap;
                    }
                }
                entry.confirmed_violation = Some(clock);
                entry.last_confirmed = clock;
                entry.compliant = false;
                let episode = &mut entry.public;
                let better_evidence =
                    episode.hit_count == 0 || confidence > episode.peak_confidence;
                episode.hit_count += 1;
                episode.last_ts = timestamp.to_owned();
                episode.peak_confidence = confidence.max(episode.peak_confidence);
                episode.duration = clock.saturating_sub(entry.first);
                episode.tier = episode.tier.max(window.tier(entry.confirmed_for));
                Some(EpisodeChange::Hit {
                    episode: episode.clone(),
                    better_evidence,
                })
            }
            Observation::Outside => self.close(key, CloseReason::LeftZone),
            Observation::ConfirmedCompliant => self.close(key, CloseReason::Recovered),
            Observation::Compliant => {
                let entry = self.active.get_mut(&key)?;
                entry.last_confirmed = clock;
                entry.confirmed_violation = None;
                if entry.compliant {
                    return self.close(key, CloseReason::Recovered);
                }
                entry.compliant = true;
                None
            }
            Observation::Unknown => {
                let entry = self.active.get_mut(&key)?;
                entry.confirmed_violation = None;
                entry.compliant = false;
                None
            }
        }
    }

    pub fn needs_recovery_check(&self) -> bool {
        self.active.values().any(|entry| entry.compliant)
    }

    pub fn expire(&mut self, clock: Duration) -> Vec<(EpisodeKey, EpisodeChange)> {
        self.close_where(|_, entry| {
            clock.saturating_sub(entry.last_confirmed) > MAX_OBSERVATION_GAP
        })
    }

    pub fn remove_rules(
        &mut self,
        keep: impl Fn(EpisodeKey) -> bool,
    ) -> Vec<(EpisodeKey, EpisodeChange)> {
        self.close_where(|key, _| !keep(key))
    }

    fn close_where(
        &mut self,
        should_close: impl Fn(EpisodeKey, &ActiveEpisode) -> bool,
    ) -> Vec<(EpisodeKey, EpisodeChange)> {
        self.active
            .extract_if(|key, entry| should_close(*key, entry))
            .map(|(key, entry)| {
                (
                    key,
                    EpisodeChange::Closed {
                        episode: entry.public,
                        reason: CloseReason::Interrupted,
                    },
                )
            })
            .collect()
    }

    fn close(&mut self, key: EpisodeKey, reason: CloseReason) -> Option<EpisodeChange> {
        self.active.remove(&key).map(|entry| EpisodeChange::Closed {
            episode: entry.public,
            reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: EpisodeKey = EpisodeKey {
        rule_id: 4,
        track_id: 2,
        zone_id: Some(5),
    };

    #[test]
    fn episode_warns_then_upgrades_and_requires_confirmed_recovery() {
        let window = GraceWindow {
            seconds: 10,
            warning_percent: 70,
        };
        let mut tracker = EpisodeTracker::default();
        let mut observe = |sec, state| {
            tracker.observe(
                KEY,
                state,
                Duration::from_secs(sec),
                &sec.to_string(),
                window,
            )
        };
        for sec in [0, 2, 4, 6, 8, 10] {
            let confidence = if sec == 4 { 0.9 } else { 0.7 };
            let Some(EpisodeChange::Hit {
                episode,
                better_evidence,
            }) = observe(sec, Observation::Violation { confidence })
            else {
                panic!("expected a hit")
            };
            let expected = if sec < 8 {
                AlertTier::Grace
            } else if sec < 10 {
                AlertTier::Warning
            } else {
                AlertTier::Alert
            };
            assert_eq!(episode.tier, expected);
            assert_eq!(episode.hit_count, sec / 2 + 1);
            assert_eq!(better_evidence, [0, 4].contains(&sec));
        }
        assert!(observe(12, Observation::Compliant).is_none());
        assert!(observe(13, Observation::Unknown).is_none());
        assert!(observe(14, Observation::Compliant).is_none());
        assert!(matches!(
            observe(15, Observation::Compliant),
            Some(EpisodeChange::Closed {
                reason: CloseReason::Recovered,
                ..
            })
        ));
    }
}
