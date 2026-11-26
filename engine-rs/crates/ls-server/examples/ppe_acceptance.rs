//! Offline PPE predictions using the released assets and folder worker's image geometry.
//! Reads JSONL with an image path and normalized XYXY human boxes per row.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use ls_analyzer::detector::{load_detector_cv, DetectorConfig};
use ls_analyzer::pipeline::{Detector, PpeRunner};
use ls_analyzer::ppe::{make_ppe_runner_cv, PpeConfig, COAT_CONFIRMATION_FILE};
use ls_infer::yolo::{VulkanAdapter, VulkanCv};
use ls_ingest::Frame;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct Sample {
    image: PathBuf,
    boxes: Vec<[f32; 4]>,
}

fn predict(
    detector: &mut dyn Detector,
    ppe: &mut dyn PpeRunner,
    frame: &Frame,
    reference_boxes: &[[f32; 4]],
) -> Result<Value, String> {
    let detections = detector.try_detect(frame)?;
    let boxes: Vec<_> = detections.iter().map(|det| det.box_).collect();
    let observations = ppe.observations(&frame.data, frame.width, frame.height, &boxes);
    if observations.len() != detections.len() {
        return Err("PPE result count differs from person detections".into());
    }
    let with_reference_boxes =
        ppe.observations(&frame.data, frame.width, frame.height, reference_boxes);
    if with_reference_boxes.len() != reference_boxes.len() {
        return Err("PPE result count differs from reference boxes".into());
    }
    let detail = |observation: ls_analyzer::pipeline::PpeObservation| json!({"coat": observation.coat, "diagnostic": observation.coat_diagnostic});
    Ok(json!({
        "detected": detections
            .iter()
            .zip(observations)
            .map(|(person, observation)| {
                json!({
                    "bbox": person.box_, "person_confidence": person.score,
                    "ppe": detail(observation),
                })
            })
            .collect::<Vec<_>>(),
        "reference": with_reference_boxes.into_iter().map(detail).collect::<Vec<_>>(),
    }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [install, manifest, output, backend] = args.as_slice() else {
        return Err("usage: ppe_acceptance INSTALL IMAGE_MANIFEST OUTPUT cpu|dml|vulkan".into());
    };
    let output = PathBuf::from(output);
    let install = std::path::Path::new(install).canonicalize()?;
    let models = install.join("models");
    let vulkan = match backend.to_str() {
        Some("cpu" | "dml") => None,
        Some("vulkan") => Some(VulkanCv {
            install_root: install,
            cache: output.with_extension("vulkan-cache"),
            adapter: VulkanAdapter::Auto,
        }),
        _ => return Err("backend must be cpu, dml or vulkan".into()),
    };
    let use_gpu = backend != "cpu";
    let mut detector = load_detector_cv(
        &DetectorConfig {
            model_path: Some(models.join("yolo11n.onnx")),
            use_gpu,
            ..Default::default()
        },
        vulkan.as_ref(),
    )?;
    let mut ppe = make_ppe_runner_cv(
        &PpeConfig {
            model_path: models.join("yolo11n_ppe.onnx"),
            coat_confirmation: Some(models.join(COAT_CONFIRMATION_FILE)),
            // Bundled model defaults in ls-server worker::ppe_thresholds.
            violation_conf: 0.50,
            wearing_conf: 0.40,
            use_gpu,
            ..Default::default()
        },
        vulkan.as_ref(),
    )
    .ok_or("bundled PPE model failed to load")?;

    let mut report = BufWriter::new(File::options().write(true).create_new(true).open(output)?);
    for line in fs::read_to_string(manifest)?
        .lines()
        .filter(|line| !line.is_empty())
    {
        let sample: Sample = serde_json::from_str(line)?;
        let (data, width, height) = ls_ingest::decode_image(&sample.image, 1280, 720)
            .map_err(|error| format!("{}: {error}", sample.image.display()))?;
        let boxes: Vec<_> = sample
            .boxes
            .iter()
            .map(|[x1, y1, x2, y2]| {
                [
                    x1 * width as f32,
                    y1 * height as f32,
                    x2 * width as f32,
                    y2 * height as f32,
                ]
            })
            .collect();
        let mut frame = Frame {
            width,
            height,
            data,
            index: 0,
        };
        let original = predict(detector.as_mut(), ppe.as_mut(), &frame, &boxes)?;
        for channel in &mut frame.data {
            *channel /= 3;
        }
        let dimmed = predict(detector.as_mut(), ppe.as_mut(), &frame, &boxes)?;
        let device = |value: Option<ls_infer::yolo::InferenceDevice>| value.map(|d| d.label());
        serde_json::to_writer(
            &mut report,
            &json!({
                "image": sample.image, "width": width, "height": height,
                "person_device": device(detector.inference_device()), "ppe_device": device(ppe.inference_device()),
                "original": original, "dimmed": dimmed,
            }),
        )?;
        writeln!(report)?;
    }
    report.flush()?;
    Ok(())
}
