use std::{fs::File, path::Path};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{CameraPolicy, EventSink, WorkerConfig};

pub(super) struct Inputs(Value);

fn model(path: &Path) -> Option<Value> {
    let mut file = File::open(path).ok()?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest).ok()?;
    Some(
        json!({"name": path.file_name()?.to_string_lossy(), "sha256": format!("{:x}", digest.finalize())}),
    )
}

impl Inputs {
    pub fn new(
        cfg: &WorkerConfig,
        version: &str,
        mode: &str,
        prompt: &str,
        ppe_loaded: bool,
    ) -> Self {
        let ppe = cfg.ppe.as_ref().filter(|_| ppe_loaded);
        Self(json!({
            "version": version, "source_type": cfg.spec.type_, "analysis_mode": mode,
            "requested_backend": if cfg.cv_backend == "dml" { "directml" } else { &cfg.cv_backend },
            "vulkan_adapter": if let ls_infer::yolo::VulkanAdapter::Index(index) = cfg.vulkan_adapter {
                Some(index)
            } else {
                None
            },
            "vulkan_gpu_id": cfg.vulkan_adapter.id(),
            "directml_adapter": ls_infer::ort_session::directml_adapter(),
            "person_model": cfg.detector.model_path.as_deref().and_then(model),
            "person_confidence": cfg.detector.conf, "person_iou": cfg.detector.iou,
            "ppe_model": ppe.and_then(|ppe| model(&ppe.model_path)),
            "ppe_confirmation": ppe.and_then(|ppe| ppe.coat_confirmation.as_deref()).and_then(model),
            "ppe_violation_confidence": ppe.map(|ppe| ppe.violation_conf),
            "ppe_wearing_confidence": ppe.map(|ppe| ppe.wearing_conf),
            "review_prompt_sha256": (mode != "local").then(|| crate::util::sha256_hex(prompt.as_bytes())),
        }))
    }

    pub fn update(&self, sink: &EventSink, policy: &CameraPolicy) {
        let mut data = self.0.clone();
        data["rules"] = json!(policy.rules);
        data["zones"] = json!(policy.zones);
        data["lab_rules"] = json!(policy.lab);
        let settings = data.to_string();
        let hash = crate::util::sha256_hex(settings.as_bytes());
        if let Ok(mut saved) = sink.provenance.lock() {
            *saved = Some((hash, settings));
        }
    }
}
