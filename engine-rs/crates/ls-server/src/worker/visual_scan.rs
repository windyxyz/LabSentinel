use std::time::Duration;

use ls_analyzer::{evidence::Evidence, pipeline::Candidate};

use super::{spawn_finalize, EventSink};

const POST: Duration = Duration::from_secs(3);

pub(super) struct Sample<'a> {
    pub at: Duration,
    pub index: u64,
    pub timestamp: &'a str,
    pub jpeg: &'a [u8],
    pub people: usize,
    pub score: f32,
}

#[derive(Default)]
pub(super) struct Window {
    previous: Option<(Duration, Vec<u8>)>,
    pending: Option<Sequence>,
    sent: Option<(Duration, [u8; 32])>,
}

pub(super) struct Sequence {
    before: Option<Vec<u8>>,
    peak: Vec<u8>,
    after: Option<Vec<u8>>,
    started: Duration,
    peak_at: Duration,
    index: u64,
    timestamp: String,
    people: usize,
    score: f32,
}

impl Window {
    pub fn observe(&mut self, sample: Sample<'_>, interval: Duration) -> Option<Sequence> {
        if sample.jpeg.is_empty() {
            return None;
        }
        if let Some(sequence) = &mut self.pending {
            sequence.people = sequence.people.max(sample.people);
            if sample.score > sequence.score && sample.at.saturating_sub(sequence.started) < POST {
                sequence.peak.clear();
                sequence.peak.extend_from_slice(sample.jpeg);
                sequence.peak_at = sample.at;
                sequence.index = sample.index;
                sequence.timestamp = sample.timestamp.into();
                sequence.score = sample.score;
            }
            if sample.at.saturating_sub(sequence.peak_at) >= POST {
                let frame = sample.jpeg.to_vec();
                self.previous = Some((sample.at, frame.clone()));
                return self.finish_with(sample.at, Some(frame));
            }
        } else if sample.people > 0
            && self
                .sent
                .is_none_or(|(time, _)| sample.at.saturating_sub(time) >= interval)
        {
            let before = self
                .previous
                .as_ref()
                .filter(|(time, _)| sample.at.saturating_sub(*time) <= POST)
                .map(|(_, frame)| frame.clone());
            self.pending = Some(Sequence {
                before,
                peak: sample.jpeg.to_vec(),
                after: None,
                started: sample.at,
                peak_at: sample.at,
                index: sample.index,
                timestamp: sample.timestamp.into(),
                people: sample.people,
                score: sample.score,
            });
        }
        self.previous = Some((sample.at, sample.jpeg.to_vec()));
        None
    }

    pub fn finish(&mut self) -> Option<Sequence> {
        let (time, frame) = self.previous.take()?;
        self.finish_with(time, Some(frame))
    }

    pub fn discard(&mut self) {
        self.previous = None;
        self.pending = None;
    }

    fn finish_with(&mut self, time: Duration, frame: Option<Vec<u8>>) -> Option<Sequence> {
        let mut sequence = self.pending.take()?;
        let signature = crate::vlm::fingerprint(&sequence.peak);
        if self.sent.is_some_and(|(_, before)| before == signature) {
            self.sent = Some((time, signature));
            return None;
        }
        self.sent = Some((time, signature));
        sequence.after = frame;
        Some(sequence)
    }
}

#[derive(Default)]
pub(super) struct Counts {
    pub attempted: u64,
    pub succeeded: u64,
    pub budget_skipped: u64,
}

pub(super) struct Scanner<'a> {
    pub store: &'a ls_store::Store,
    pub sink: &'a EventSink,
    pub evidence: &'a std::sync::Arc<Evidence>,
    pub detect: &'a crate::detect::DetectCfg,
    pub vision: &'a crate::vlm::VlmCfg,
    pub prompt: &'a str,
    pub camera: i64,
}

impl Scanner<'_> {
    pub fn review(&self, sequence: Sequence, counts: &mut Counts) -> u64 {
        if !self.detect.still_authorized(
            self.store,
            std::path::Path::new(&self.sink.st.data_dir),
            self.camera,
            "api",
        ) {
            return 0;
        }
        if crate::detect::usage_try_consume(self.store, self.detect, true).0 {
            counts.budget_skipped += 1;
            return 0;
        }
        let samples: Vec<_> = sequence
            .before
            .into_iter()
            .chain([sequence.peak.clone()])
            .chain(sequence.after)
            .collect();
        let samples = crate::vlm::sample_frames(&samples, 3);
        counts.attempted += 1;
        let mut result = match crate::vlm::analyze_frames(self.vision, self.prompt, &samples) {
            Ok(result) => result,
            Err(error) => {
                crate::logger::error(&format!(
                    "[ls-worker] cam-{} 视觉判断失败：{error}",
                    self.camera
                ));
                return 0;
            }
        };
        counts.succeeded += 1;
        crate::vlm::limit_to_local_people(&mut result, sequence.people);
        let people = result["persons"].as_u64().unwrap_or_default();
        let summary = result["summary"].as_str().unwrap_or_default();
        let prompt_hash = crate::util::sha256_hex(self.prompt.as_bytes());
        let mut events = 0;
        for label in result["violations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
        {
            let behavior = if label == "no_ppe" {
                "enter_no_ppe"
            } else {
                label
            };
            let candidate = Candidate {
                camera_id: self.camera,
                zone_id: None,
                behavior: behavior.into(),
                level: 2,
                track_id: 0,
                detail: format!(
                    "AI 视频复核。{}，画面中约 {people} 人。{summary}",
                    ls_dispatch::type_text(behavior)
                ),
                ts_iso: sequence.timestamp.clone(),
                evidence_key: format!("vlm:{}:{behavior}", sequence.index),
            };
            if let Ok(Some(id)) = self.sink.record(&candidate, Some(&sequence.peak)) {
                events += 1;
                let _ = self
                    .store
                    .set_event_vision_source(id, &self.vision.model, &prompt_hash);
                spawn_finalize(&self.sink.st, self.evidence, id);
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_pre_peak_and_three_seconds_after_with_a_short_video_fallback() {
        let mut window = Window::default();
        let mut frame = |time, jpeg, people, score| {
            window.observe(
                Sample {
                    at: Duration::from_secs(time),
                    index: time,
                    timestamp: "recording",
                    jpeg,
                    people,
                    score,
                },
                Duration::from_secs(5),
            )
        };
        assert!(frame(0, &[1], 0, 0.0).is_none());
        assert!(frame(1, &[2], 1, 0.7).is_none());
        assert!(frame(2, &[3], 1, 0.9).is_none());
        assert!(frame(4, &[5], 1, 0.8).is_none());
        let sequence = frame(5, &[6], 0, 0.0).unwrap();
        assert_eq!(sequence.before.as_deref(), Some([1].as_slice()));
        assert_eq!(sequence.peak, [3]);
        assert_eq!(sequence.after.as_deref(), Some([6].as_slice()));
        assert_eq!(sequence.index, 2);

        assert!(frame(12, &[7], 1, 0.8).is_none());
        let short = window.finish().unwrap();
        assert_eq!(short.peak, [7]);
        assert_eq!(
            crate::vlm::sample_frames(&[short.peak, short.after.unwrap()], 3).len(),
            1
        );
    }
}
