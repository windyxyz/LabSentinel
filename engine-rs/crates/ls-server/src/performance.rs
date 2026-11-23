use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{json, Value};

const SAMPLE_CAPACITY: usize = 1_200;

pub(crate) struct FrameTiming {
    pub read_wait: Duration,
    pub queue_wait: Duration,
    pub analysis_age: Duration,
    pub overwritten: u64,
    pub jpeg: Duration,
    pub detection: Duration,
    pub rules: Duration,
    pub ort: Duration,
    pub directml_wait: Duration,
    pub analysis: Duration,
    pub persistence: Option<Duration>,
    pub inferred: bool,
    pub directml_calls: u64,
}

#[derive(Clone, Default)]
pub(crate) struct PerformanceWindow {
    samples: VecDeque<[u64; 10]>,
    frames: u64,
    overwritten: u64,
    inferred: u64,
    reconnects: u64,
    directml_calls: u64,
    pub input_kind: &'static str,
}

impl PerformanceWindow {
    pub fn record(&mut self, sample: FrameTiming) {
        self.frames += 1;
        self.overwritten += sample.overwritten;
        self.inferred += u64::from(sample.inferred);
        self.directml_calls += sample.directml_calls;
        if self.samples.len() == SAMPLE_CAPACITY {
            self.samples.pop_front();
        }
        let micros = |duration: Duration| duration.as_micros() as u64;
        self.samples.push_back([
            micros(sample.read_wait),
            micros(sample.jpeg),
            micros(sample.detection),
            micros(sample.rules),
            micros(sample.ort),
            micros(sample.directml_wait),
            micros(sample.analysis),
            sample.persistence.map_or(u64::MAX, micros),
            micros(sample.queue_wait),
            micros(sample.analysis_age),
        ]);
    }

    pub fn reconnect(&mut self) {
        self.reconnects += 1;
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn reconnects(&self) -> u64 {
        self.reconnects
    }

    pub fn summary(&self) -> Value {
        let labels = [
            "read_wait_ms",
            "jpeg_ms",
            "detection_ms",
            "rules_with_ppe_ms",
            "ort_run_ms",
            "directml_lock_wait_ms",
            "analysis_after_read_ms",
            "event_work_after_read_ms",
            "capture_queue_ms",
            "analysis_frame_age_ms",
        ];
        let mut latency = serde_json::Map::new();
        for (index, name) in labels.into_iter().enumerate() {
            let mut samples: Vec<_> = self
                .samples
                .iter()
                .map(|sample| sample[index])
                .filter(|value| *value != u64::MAX)
                .collect();
            samples.sort_unstable();
            latency.insert(name.into(), if samples.is_empty() { Value::Null } else {
                let percentile = |percent: usize| {
                    let rank = (samples.len() * percent).div_ceil(100usize).saturating_sub(1);
                    samples[rank] as f64 / 1_000.0
                };
                json!({"p50": percentile(50usize), "p95": percentile(95usize), "p99": percentile(99usize)})
            });
        }
        json!({"frames": self.frames, "capture_overwritten_frames": self.overwritten,
            "inferred": self.inferred, "reconnects": self.reconnects, "directml_calls": self.directml_calls,
            "input": self.input_kind,
            "sample_count": self.samples.len(), "latency": latency})
    }
}
