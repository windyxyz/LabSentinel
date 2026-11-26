use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{json, Value};

const SAMPLE_CAPACITY: usize = 1_200;

#[derive(Default)]
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
    pub vulkan: Option<Duration>,
    pub vulkan_first: Option<Duration>,
    pub vulkan_wait: Option<Duration>,
    pub gpu_queue: Option<Duration>,
    pub person_ppe: Option<Duration>,
    pub ppe: ls_analyzer::pipeline::PpeTimings,
    pub analysis: Duration,
    pub persistence: Option<Duration>,
    pub inferred: bool,
    pub directml_calls: u64,
    pub vulkan_calls: u64,
}

#[derive(Clone, Default)]
pub(crate) struct PerformanceWindow {
    samples: VecDeque<[u64; 19]>,
    frames: u64,
    overwritten: u64,
    inferred: u64,
    reconnects: u64,
    directml_calls: u64,
    vulkan_calls: u64,
    pub input_kind: &'static str,
    pub capture_backend: &'static str,
}

impl PerformanceWindow {
    pub fn record(&mut self, sample: FrameTiming) {
        self.frames += 1;
        self.overwritten += sample.overwritten;
        self.inferred += u64::from(sample.inferred);
        self.directml_calls += sample.directml_calls;
        self.vulkan_calls += sample.vulkan_calls;
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
            sample.vulkan.map_or(u64::MAX, micros),
            sample.person_ppe.map_or(u64::MAX, micros),
            sample.ppe.full_frame.map_or(u64::MAX, micros),
            sample.ppe.crops.map_or(u64::MAX, micros),
            sample.ppe.coat_confirmation.map_or(u64::MAX, micros),
            sample.ppe.close_mask.map_or(u64::MAX, micros),
            sample.vulkan_first.map_or(u64::MAX, micros),
            sample.vulkan_wait.map_or(u64::MAX, micros),
            sample.gpu_queue.map_or(u64::MAX, micros),
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
            "vulkan_run_ms",
            "person_ppe_ms",
            "ppe_full_frame_ms",
            "ppe_crop_ms",
            "ppe_coat_confirmation_ms",
            "ppe_close_mask_ms",
            "vulkan_first_run_ms",
            "vulkan_lock_wait_ms",
            "gpu_queue_ms",
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
                json!({"p50": percentile(50usize), "p95": percentile(95usize), "p99": percentile(99usize), "samples": samples.len()})
            });
        }
        json!({"frames": self.frames, "capture_overwritten_frames": self.overwritten,
            "inferred": self.inferred, "reconnects": self.reconnects, "directml_calls": self.directml_calls,
            "vulkan_calls": self.vulkan_calls, "input": self.input_kind,
            "capture_backend": self.capture_backend,
            "sample_count": self.samples.len(), "latency": latency})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_latency_uses_only_frames_that_ran_the_reported_stage() {
        let mut report = PerformanceWindow::default();
        for _ in 0..30 {
            report.record(FrameTiming {
                inferred: true,
                ..Default::default()
            });
        }
        let empty = report.summary();
        assert!(empty["latency"]["person_ppe_ms"].is_null());
        assert!(empty["latency"]["vulkan_run_ms"].is_null());
        assert!(empty["latency"]["gpu_queue_ms"].is_null());
        assert!(empty["latency"]["vulkan_first_run_ms"].is_null());
        report.record(FrameTiming {
            inferred: true,
            person_ppe: Some(Duration::from_millis(28)),
            ppe: ls_analyzer::pipeline::PpeTimings {
                full_frame: Some(Duration::from_millis(12)),
                coat_confirmation: Some(Duration::from_millis(6)),
                ..Default::default()
            },
            vulkan: Some(Duration::from_millis(21)),
            vulkan_first: Some(Duration::from_millis(16)),
            vulkan_wait: Some(Duration::from_millis(3)),
            gpu_queue: Some(Duration::from_millis(3)),
            vulkan_calls: 2,
            ..Default::default()
        });
        let summary = report.summary();
        assert_eq!(summary["inferred"], 31);
        assert_eq!(summary["vulkan_calls"], 2);
        assert_eq!(summary["latency"]["person_ppe_ms"]["p95"], 28.0);
        assert_eq!(summary["latency"]["vulkan_run_ms"]["p95"], 21.0);
        assert_eq!(summary["latency"]["vulkan_first_run_ms"]["p95"], 16.0);
        assert_eq!(summary["latency"]["vulkan_lock_wait_ms"]["p95"], 3.0);
        assert_eq!(summary["latency"]["gpu_queue_ms"]["p95"], 3.0);
        assert_eq!(summary["latency"]["person_ppe_ms"]["samples"], 1);
        assert_eq!(summary["latency"]["detection_ms"]["samples"], 31);
        assert_eq!(summary["latency"]["ppe_full_frame_ms"]["p95"], 12.0);
        assert_eq!(summary["latency"]["ppe_coat_confirmation_ms"]["samples"], 1);
        assert!(summary["latency"]["ppe_crop_ms"].is_null());
    }
}
