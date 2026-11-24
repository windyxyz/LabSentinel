//! PPE 级联检测器（RA2-S1 / ADR-0011）：从 bin 参数化迁出——sidecar 走 env 组装
//! `PpeConfig`，ls-server/folder worker 从 settings 组装，共用同一实现。
//!
//! 单次整图推理解码实验服和口罩证据，各自判定，保留无法确认的状态。

use std::path::{Path, PathBuf};

use ls_infer::face::FaceMaskModel;
use ls_infer::post::Det;
use ls_infer::yolo::{OutputFormat, YoloModel};

use crate::pipeline::{PpeObservation, PpeRunner, PpeStatus};

pub const COAT_CONFIRMATION_FILE: &str = "labcoat-confirm.onnx";
const COAT_CONFIRMATION_CONF: f32 = 0.55;

#[derive(Debug, Clone)]
pub struct CloseMaskAssets {
    detector: PathBuf,
    classifier: PathBuf,
}

impl CloseMaskAssets {
    pub fn from_dir(dir: &Path) -> Option<Self> {
        Some(Self {
            detector: anchored_child(dir, "face_detection_yunet_2023mar.onnx")
                .filter(|p| p.is_file())?,
            classifier: anchored_child(dir, "mask_detector.onnx").filter(|p| p.is_file())?,
        })
    }
}

/// PPE 级联配置（模型路径须已过 `crate::detector::anchor_model_file` 锚点）。
#[derive(Debug, Clone)]
pub struct PpeConfig {
    pub model_path: PathBuf,
    pub output_format: Option<OutputFormat>,
    pub imgsz: u32,
    pub iou: f32,
    pub violation_conf: f32,
    pub wearing_conf: f32,
    /// 违规类名（在类表中查找索引；找不到 → None = 仅人员检测）
    pub violation_class_name: String,
    /// 穿戴类名列表（逗号拆分后逐个查索引）
    pub wearing_class_names: Vec<String>,
    /// UX（2026-11-21）：GPU 推理（与主检测同设备；DirectML，失败回落 CPU）
    pub use_gpu: bool,
    /// LS-055：灰度/低照度 ROI 启用 CLAHE 局部对比度增强
    pub clahe_enabled: bool,
    pub close_mask: Option<CloseMaskAssets>,
    /// A bundled white-coat model must corroborate positive wearing observations.
    pub coat_confirmation: Option<PathBuf>,
}

impl Default for PpeConfig {
    fn default() -> Self {
        // 默认值与 sidecar env 口径一致（LS_IMGSZ/LS_IOU_THRES/LS_PPE_CONF/
        // LS_PPE_WEARING_CONF/LS_PPE_VIOLATION_CLASS/LS_PPE_CLASS_NAMES）
        PpeConfig {
            model_path: PathBuf::new(),
            output_format: None,
            imgsz: 640,
            iou: 0.50,
            violation_conf: 0.30,
            wearing_conf: 0.20,
            violation_class_name: "no lab coat".into(),
            wearing_class_names: vec!["lab coat".into()],
            use_gpu: true,
            clahe_enabled: true,
            close_mask: None,
            coat_confirmation: None,
        }
    }
}

impl PpeConfig {
    pub fn supports_labels(&self, labels: &[String]) -> bool {
        class_index(labels, &self.violation_class_name).is_some()
            && self
                .wearing_class_names
                .iter()
                .any(|name| class_index(labels, name).is_some())
    }
}

fn class_index(labels: &[String], name: &str) -> Option<usize> {
    let name = name.trim().replace(['_', '-'], " ");
    labels
        .iter()
        .position(|label| label.replace(['_', '-'], " ").eq_ignore_ascii_case(&name))
}

/// 模型目录内锚点校验：目标必须是 dir 的直接子文件（拒绝 ../、绝对路径、分隔符）。
/// 返回规范化后的路径；越界/不存在返回 None。
pub fn anchored_child(dir: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || Path::new(name).is_absolute()
    {
        return None;
    }
    let dir = dir.canonicalize().ok()?;
    let target = dir.join(name).canonicalize().ok()?;
    if target.parent() != Some(dir.as_path()) {
        return None; // 符号链接逃逸/非直接子文件
    }
    Some(target)
}

/// 读模型目录的类别名（`<stem>.classes.txt` 或 model.json `classes` 指向的文件；
/// 一行一个）。安全锚点：只接受**模型目录内的直接文件名**（`anchored_child`），
/// 拒绝 `../`、绝对路径与符号链接逃逸——模型目录内的清单不得成为任意文件读取入口。
pub fn load_class_names(model_path: &Path) -> Vec<String> {
    let dir = model_path.parent().unwrap_or(Path::new("."));
    let stem = model_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut file = anchored_child(dir, &format!("{stem}.classes.txt"));
    if let Ok(text) = std::fs::read_to_string(dir.join("model.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(c) = v.get("classes").and_then(|c| c.as_str()) {
                // LS-043 修复：仅当 anchored_child 命中有效文件时覆盖，不把有效回退 file 置空
                if let Some(p) = anchored_child(dir, c) {
                    file = Some(p);
                }
            }
        }
    }
    let Some(file) = file else {
        return Vec::new();
    };
    std::fs::read_to_string(file)
        .map(|t| {
            t.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect()
        })
        .unwrap_or_default()
}

/// YOLO PPE 级联。
pub struct YoloPpeRunner {
    model: YoloModel,
    violation_class: usize,
    wearing_classes: Vec<usize>,
    mask_classes: Option<(usize, usize)>,
    violation_conf: f32,
    wearing_conf: f32,
    clahe_enabled: bool,
    close_mask: Option<FaceMaskModel>,
    coat_confirmation: Option<CoatConfirmation>,
}

struct CoatConfirmation {
    model: Option<YoloModel>,
}

fn apply_coat_confirmation(
    statuses: &mut [PpeStatus],
    people: &[[f32; 4]],
    detections: &[Vec<Det>],
) {
    let mut evidence = vec![[0.0_f32; 2]; people.len()];
    for (class, boxes) in detections.iter().enumerate() {
        for detection in boxes {
            if let Some(owner) = coat_owner(detection.box_, people) {
                let score = &mut evidence[owner][usize::from(class != 0)];
                *score = score.max(detection.score);
            }
        }
    }
    for (status, [positive, negative]) in statuses.iter_mut().zip(evidence) {
        if *status == PpeStatus::Wearing && positive <= negative {
            *status = PpeStatus::Unknown;
        }
    }
}

fn head_owners(person_boxes: &[[f32; 4]], x: f32, y: f32) -> impl Iterator<Item = usize> + '_ {
    person_boxes
        .iter()
        .enumerate()
        .filter_map(move |(index, b)| {
            (x >= b[0] && x <= b[2] && y >= b[1] && y <= b[1] + (b[3] - b[1]) * 0.42)
                .then_some(index)
        })
}

fn close_mask_status([covered, uncovered]: [f32; 2]) -> PpeStatus {
    if !covered.is_finite() || !uncovered.is_finite() {
        return PpeStatus::Unknown;
    }
    // A no-mask alert needs stronger evidence than a positive wearing observation.
    if (0.99..=1.0).contains(&uncovered) && (0.0..=0.01 + f32::EPSILON).contains(&covered) {
        return PpeStatus::NoPpe;
    }
    if (0.95..=1.0).contains(&covered) && (0.0..=0.05 + f32::EPSILON).contains(&uncovered) {
        return PpeStatus::Wearing;
    }
    PpeStatus::Unknown
}

fn supplement_masks(
    model: &mut FaceMaskModel,
    bgr: &[u8],
    w: u32,
    h: u32,
    person_boxes: &[[f32; 4]],
    masks: &mut [PpeStatus],
    confidence: &mut [f32],
) {
    let Ok(faces) = model.faces(bgr, w, h) else {
        return;
    };
    let mut associated = vec![Vec::new(); masks.len()];
    for bounds in faces {
        let [x1, y1, x2, y2] = bounds;
        let mut owners = head_owners(person_boxes, (x1 + x2) as f32 / 2.0, (y1 + y2) as f32 / 2.0);
        if let (Some(owner), None) = (owners.next(), owners.next()) {
            associated[owner].push(bounds);
        }
    }
    for ((status, confidence), faces) in masks.iter_mut().zip(confidence).zip(associated) {
        let (PpeStatus::Unknown, [bounds]) = (*status, faces.as_slice()) else {
            continue;
        };
        if let Ok(scores) = model.classify(bgr, w, *bounds) {
            *status = close_mask_status(scores);
            if *status == PpeStatus::NoPpe {
                *confidence = scores[1];
            }
        }
    }
}

fn area(bounds: [f32; 4]) -> f32 {
    (bounds[2] - bounds[0]).max(0.0) * (bounds[3] - bounds[1]).max(0.0)
}

fn intersection(a: [f32; 4], b: [f32; 4]) -> f32 {
    area([
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ])
}

pub(crate) fn overlapping_people(a: [f32; 4], b: [f32; 4]) -> bool {
    let overlap = intersection(a, b);
    overlap > 0.0 && overlap >= 0.35 * (area(a) + area(b) - overlap)
}

fn coat_owner(coat: [f32; 4], people: &[[f32; 4]]) -> Option<usize> {
    let coat_area = area(coat);
    if coat_area <= 0.0 || !coat_area.is_finite() {
        return None;
    }
    let mut best = (0, 0.0);
    let mut second = 0.0_f32;
    for (index, person) in people.iter().enumerate() {
        let mut body = *person;
        body[1] += 0.10 * (body[3] - body[1]);
        let coverage = intersection(coat, body) / coat_area;
        if coverage > best.1 {
            second = best.1;
            best = (index, coverage);
        } else {
            second = second.max(coverage);
        }
    }
    (best.1 >= 0.60 && best.1 - second >= 0.15).then_some(best.0)
}

/// 裁切 BGR 子图（带 15% padding），用于远景/灰度监控图人框 ROI 二次精检。
fn crop_bgr_roi(bgr: &[u8], w: u32, h: u32, pb: [f32; 4]) -> Option<(Vec<u8>, u32, u32)> {
    let bw = (pb[2] - pb[0]).max(0.0);
    let bh = (pb[3] - pb[1]).max(0.0);
    if bw < 32.0 || bh < 32.0 {
        return None;
    }
    let pad_x = bw * 0.15;
    let pad_y = bh * 0.15;
    let x1 = ((pb[0] - pad_x).floor().max(0.0) as u32).min(w.saturating_sub(1));
    let y1 = ((pb[1] - pad_y).floor().max(0.0) as u32).min(h.saturating_sub(1));
    let x2 = ((pb[2] + pad_x).ceil().max(0.0) as u32).min(w);
    let y2 = ((pb[3] + pad_y).ceil().max(0.0) as u32).min(h);
    let rw = x2.saturating_sub(x1);
    let rh = y2.saturating_sub(y1);
    if rw < 32 || rh < 32 || bgr.len() < (w as usize) * (h as usize) * 3 {
        return None;
    }
    let mut out = Vec::with_capacity((rw as usize) * (rh as usize) * 3);
    for y in y1..y2 {
        let start = ((y as usize) * (w as usize) + (x1 as usize)) * 3;
        let end = start + (rw as usize) * 3;
        out.extend_from_slice(&bgr[start..end]);
    }
    Some((out, rw, rh))
}

impl PpeRunner for YoloPpeRunner {
    fn statuses(
        &mut self,
        bgr: &[u8],
        w: u32,
        h: u32,
        person_boxes: &[[f32; 4]],
    ) -> Vec<PpeStatus> {
        self.observations(bgr, w, h, person_boxes)
            .into_iter()
            .map(|observation| observation.coat)
            .collect()
    }

    fn observations(
        &mut self,
        bgr: &[u8],
        w: u32,
        h: u32,
        person_boxes: &[[f32; 4]],
    ) -> Vec<PpeObservation> {
        if person_boxes.is_empty() {
            return Vec::new();
        }
        // LS-043：单次前向推理同时解码 violation + 全部 wearing 类（消除 2× 重复推理）
        let mut specs: Vec<(usize, f32, f32)> = Vec::with_capacity(1 + self.wearing_classes.len());
        specs.push((self.violation_class, self.violation_conf, 0.0));
        for &wc in &self.wearing_classes {
            specs.push((wc, self.wearing_conf, 0.0));
        }
        let coat_classes = specs.len();
        if let Some((no_mask, mask)) = self.mask_classes {
            specs.push((no_mask, self.violation_conf, 0.0));
            specs.push((mask, self.wearing_conf, 0.0));
        }
        let unknown = PpeObservation::default();
        let Ok(mut all_dets) = self.model.detect_classes(bgr, w, h, &specs) else {
            return vec![unknown; person_boxes.len()];
        };
        let mask_dets = all_dets.split_off(coat_classes);
        let no_boxes_raw = if !all_dets.is_empty() {
            all_dets.remove(0)
        } else {
            Vec::new()
        };
        let wear_boxes: Vec<[f32; 4]> = all_dets
            .into_iter()
            .flat_map(|v| v.into_iter().map(|d| d.box_))
            .collect();
        let frame_area = ((w as f32) * (h as f32)).max(1.0);
        let mut masks = vec![PpeStatus::Unknown; person_boxes.len()];
        let mut mask_confidence = vec![0.0f32; person_boxes.len()];
        if let [uncovered, covered] = mask_dets.as_slice() {
            let mut evidence = vec![(false, false); person_boxes.len()];
            for (index, dets) in [uncovered, covered].into_iter().enumerate() {
                for det in dets {
                    let [x1, y1, x2, y2] = det.box_;
                    let x = (x1 + x2) * 0.5;
                    let y = (y1 + y2) * 0.5;
                    let mut owners = head_owners(person_boxes, x, y);
                    if let (Some(owner), None) = (owners.next(), owners.next()) {
                        if index == 0 {
                            evidence[owner].0 = true;
                            mask_confidence[owner] = mask_confidence[owner].max(det.score);
                        } else {
                            evidence[owner].1 = true;
                        }
                    }
                }
            }
            for (mask, observed) in masks.iter_mut().zip(evidence) {
                *mask = match observed {
                    (true, false) => PpeStatus::NoPpe,
                    (false, true) => PpeStatus::Wearing,
                    _ => PpeStatus::Unknown,
                };
            }
        }
        if masks.contains(&PpeStatus::Unknown) {
            if let Some(model) = self.close_mask.as_mut() {
                supplement_masks(
                    model,
                    bgr,
                    w,
                    h,
                    person_boxes,
                    &mut masks,
                    &mut mask_confidence,
                );
            }
        }
        let mut coat_hits = vec![(false, false); person_boxes.len()];
        let mut coat_confidence = vec![0.0f32; person_boxes.len()];
        for detection in no_boxes_raw {
            if let Some(owner) = coat_owner(detection.box_, person_boxes) {
                coat_hits[owner].0 = true;
                coat_confidence[owner] = coat_confidence[owner].max(detection.score);
            }
        }
        for bounds in wear_boxes {
            if let Some(owner) = coat_owner(bounds, person_boxes) {
                coat_hits[owner].1 = true;
            }
        }
        let mut coats: Vec<_> = person_boxes
            .iter()
            .enumerate()
            .map(|(index, pb)| {
                match coat_hits[index] {
                    (true, false) => return PpeStatus::NoPpe,
                    (false, true) => return PpeStatus::Wearing,
                    (true, true) => return PpeStatus::Unknown,
                    (false, false) => (),
                }
                // LS-043：远景/灰度图整图无检出时，对较小人框裁切 ROI 二次精检（提升远景召回率）
                let box_area = ((pb[2] - pb[0]).max(0.0)) * ((pb[3] - pb[1]).max(0.0));
                let occluded = person_boxes
                    .iter()
                    .enumerate()
                    .any(|(other, bounds)| index != other && overlapping_people(*pb, *bounds));
                if !occluded && box_area / frame_area < 0.35 {
                    if let Some((roi, rw, rh)) = crop_bgr_roi(bgr, w, h, *pb) {
                        // LS-055：灰度/低照度 ROI 先做 CLAHE 增强（仅灰度图触发，避免彩色失真）
                        let roi = if self.clahe_enabled
                            && ls_infer::preprocess::looks_grayscale(&roi, rw, rh)
                        {
                            ls_infer::preprocess::clahe_bgr(
                                &roi,
                                rw,
                                rh,
                                ls_infer::preprocess::ClaheConfig::default(),
                            )
                        } else {
                            roi
                        };
                        if let Ok(roi_dets) =
                            self.model
                                .detect_classes(&roi, rw, rh, &specs[..coat_classes])
                        {
                            let has_no = roi_dets.first().is_some_and(|v| !v.is_empty());
                            let has_wear = roi_dets.iter().skip(1).any(|v| !v.is_empty());
                            return match (has_no, has_wear) {
                                (true, false) => {
                                    coat_confidence[index] =
                                        roi_dets[0].iter().map(|d| d.score).fold(0.0, f32::max);
                                    PpeStatus::NoPpe
                                }
                                (false, true) => PpeStatus::Wearing,
                                _ => PpeStatus::Unknown,
                            };
                        }
                    }
                }
                PpeStatus::Unknown
            })
            .collect();
        if let Some(confirmation) = self.coat_confirmation.as_mut() {
            if coats.contains(&PpeStatus::Wearing) {
                let detections = confirmation
                    .model
                    .as_mut()
                    .and_then(|model| {
                        model
                            .detect_classes(
                                bgr,
                                w,
                                h,
                                &[
                                    (0, COAT_CONFIRMATION_CONF, 0.0),
                                    (1, COAT_CONFIRMATION_CONF, 0.0),
                                    (2, COAT_CONFIRMATION_CONF, 0.0),
                                ],
                            )
                            .ok()
                    })
                    .unwrap_or_default();
                apply_coat_confirmation(&mut coats, person_boxes, &detections);
            }
        }
        coats
            .into_iter()
            .zip(masks)
            .zip(coat_confidence.into_iter().zip(mask_confidence))
            .map(
                |((coat, mask), (coat_confidence, mask_confidence))| PpeObservation {
                    coat,
                    mask,
                    coat_violation_confidence: if coat == PpeStatus::NoPpe {
                        coat_confidence
                    } else {
                        0.0
                    },
                    mask_violation_confidence: if mask == PpeStatus::NoPpe {
                        mask_confidence
                    } else {
                        0.0
                    },
                },
            )
            .collect()
    }
}

/// 构造 PPE 检测器：模型/类名/类别索引任一不满足 → None（仅人员检测）。
pub fn make_ppe_runner(cfg: &PpeConfig) -> Option<Box<dyn PpeRunner>> {
    let names = load_class_names(&cfg.model_path);
    if names.is_empty() {
        println!(
            "[ls-analyzer] PPE 类名文件缺失/为空（仅人员检测）: {}",
            cfg.model_path.display()
        );
        return None;
    }
    let find = |name: &str| class_index(&names, name);
    let Some(violation_class) = find(&cfg.violation_class_name) else {
        println!(
            "[ls-analyzer] PPE 违规类 {} 不在类表 {:?}",
            cfg.violation_class_name, names
        );
        return None;
    };
    let wearing_classes: Vec<usize> = cfg
        .wearing_class_names
        .iter()
        .filter_map(|n| find(n))
        .collect();
    if wearing_classes.is_empty() {
        println!(
            "[ls-analyzer] PPE 穿戴类 {:?} 不在类表 {:?}",
            cfg.wearing_class_names, names
        );
        return None;
    }
    match YoloModel::load(
        &cfg.model_path,
        cfg.imgsz,
        cfg.violation_conf,
        cfg.iou,
        0.0,
        None,
        cfg.use_gpu,
    ) {
        Ok(mut model) => {
            model.use_output_format(cfg.output_format);
            let coat_confirmation = cfg.coat_confirmation.as_ref().map(|path| {
                let model = YoloModel::load(
                    path,
                    640,
                    COAT_CONFIRMATION_CONF,
                    0.5,
                    0.0,
                    None,
                    cfg.use_gpu,
                )
                .map(|mut model| {
                    model.use_output_format(Some(OutputFormat::Yolo));
                    model
                })
                .map_err(|error| {
                    println!("[ls-analyzer] 白大褂复核模型不可用，穿戴状态保留未知: {error}");
                })
                .ok();
                CoatConfirmation { model }
            });
            let close_mask = cfg.close_mask.as_ref().and_then(|assets| {
                match FaceMaskModel::load(&assets.detector, &assets.classifier, cfg.use_gpu) {
                    Ok(model) => Some(model),
                    Err(error) => {
                        println!("[ls-analyzer] 近景口罩模型加载失败: {error}");
                        None
                    }
                }
            });
            println!(
                "[ls-analyzer] PPE 模型已加载: {}（违规类={violation_class} 穿戴类={wearing_classes:?}）",
                cfg.model_path.display()
            );
            Some(Box::new(YoloPpeRunner {
                model,
                violation_class,
                wearing_classes,
                mask_classes: find("no mask").zip(find("mask")),
                violation_conf: cfg.violation_conf,
                wearing_conf: cfg.wearing_conf,
                clahe_enabled: cfg.clahe_enabled,
                close_mask,
                coat_confirmation,
            }))
        }
        Err(e) => {
            println!("[ls-analyzer] PPE 模型加载失败（仅人员检测）: {e}");
            None
        }
    }
}

/// LS-050 动态 PPE 类别映射：自定义 `.classes.txt` 中的违规类
/// （`no_gloves`/`no_goggles`/`no_mask`/`no_lab_coat` 等）→ 事件 behavior 字符串。
/// 未识别的违规类统一回退 `enter_no_ppe`（与既有口径一致）；映射表与
/// `ls-dispatch::type_text` / 前端 `TYPE_TEXT` 的键保持 1:1。
pub fn violation_behavior(class_name: &str) -> &'static str {
    let n = class_name
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_");
    match n.as_str() {
        "no_gloves" | "gloves_no" | "without_gloves" => "no_gloves",
        "no_goggles" | "goggles_no" | "without_goggles" => "no_goggles",
        "no_mask" | "mask_no" | "without_mask" => "no_mask",
        // 实验服类违规沿用既有事件名，避免破坏生产告警链
        "no_lab_coat" | "no_labcoat" | "no_ppe" | "lab_coat_no" => "enter_no_ppe",
        _ => "enter_no_ppe",
    }
}

/// 从类表筛出所有「违规类」（名字以 no_ 开头或含 without），返回
/// `(类名, 事件 behavior)` 列表，供规则引擎按类触发。穿戴类（lab_coat 等）不在其列。
pub fn violation_classes(classes: &[String]) -> Vec<(String, String)> {
    classes
        .iter()
        .filter(|n| {
            let l = n.trim().to_ascii_lowercase();
            l.starts_with("no_") || l.contains("without") || l.ends_with("_no")
        })
        .map(|n| (n.clone(), violation_behavior(n).to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_coat_boxes_do_not_confirm_wearing_or_suppress_an_independent_violation() {
        let people = [
            [0.0, 0.0, 100.0, 200.0],
            [50.0, 0.0, 150.0, 200.0],
            [175.0, 0.0, 275.0, 200.0],
        ];
        let mut status = [PpeStatus::Wearing, PpeStatus::Wearing, PpeStatus::NoPpe];
        apply_coat_confirmation(
            &mut status,
            &people,
            &[vec![
                Det {
                    box_: [10.0, 35.0, 45.0, 135.0],
                    score: 0.9,
                },
                Det {
                    box_: [60.0, 40.0, 90.0, 150.0],
                    score: 0.9,
                },
            ]],
        );
        assert_eq!(
            status,
            [PpeStatus::Wearing, PpeStatus::Unknown, PpeStatus::NoPpe]
        );
    }

    #[test]
    fn stronger_alternative_garment_leaves_coat_unknown() {
        let bounds = [10.0, 35.0, 45.0, 135.0];
        let people = [[0.0, 0.0, 100.0, 200.0]];
        let mut status = [PpeStatus::Wearing];
        apply_coat_confirmation(
            &mut status,
            &people,
            &[
                vec![Det {
                    box_: bounds,
                    score: 0.64,
                }],
                vec![Det {
                    box_: bounds,
                    score: 0.82,
                }],
            ],
        );
        assert_eq!(status, [PpeStatus::Unknown]);
    }

    #[test]
    fn violation_behavior_maps_known_classes() {
        assert_eq!(violation_behavior("no_gloves"), "no_gloves");
        assert_eq!(violation_behavior("No-Goggles"), "no_goggles");
        assert_eq!(violation_behavior("without mask"), "no_mask");
        assert_eq!(violation_behavior("no lab coat"), "enter_no_ppe");
        assert_eq!(violation_behavior("something_else"), "enter_no_ppe");
    }

    #[test]
    fn violation_classes_filters_and_keeps_behavior() {
        let names = vec![
            "person".to_string(),
            "lab coat".to_string(),
            "no_gloves".to_string(),
            "no_goggles".to_string(),
            "no_mask".to_string(),
        ];
        let got = violation_classes(&names);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], ("no_gloves".into(), "no_gloves".into()));
        assert_eq!(got[1], ("no_goggles".into(), "no_goggles".into()));
        assert_eq!(got[2], ("no_mask".into(), "no_mask".into()));
    }
}
