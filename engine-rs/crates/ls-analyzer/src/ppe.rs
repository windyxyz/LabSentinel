//! PPE 级联检测器（RA2-S1 / ADR-0011）：从 bin 参数化迁出——sidecar 走 env 组装
//! `PpeConfig`，ls-server/folder worker 从 settings 组装，共用同一实现。
//!
//! 判定口径与 Python `vision.ppe_boxes` 一致：违规框交集 → NoPpe；穿戴框 →
//! Wearing；皆无 → Unknown。已知差异（有意）：Rust 每类一次整图推理（违规 1 +
//! 穿戴 ≥1 次），Python 单次共享推理（T-301）——语义等价、性能差异。

use std::path::{Path, PathBuf};

use ls_infer::yolo::YoloModel;

use crate::pipeline::{PpeRunner, PpeStatus};

/// PPE 级联配置（模型路径须已过 `crate::detector::anchor_model_file` 锚点）。
#[derive(Debug, Clone)]
pub struct PpeConfig {
    pub model_path: PathBuf,
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
}

impl Default for PpeConfig {
    fn default() -> Self {
        // 默认值与 sidecar env 口径一致（LS_IMGSZ/LS_IOU_THRES/LS_PPE_CONF/
        // LS_PPE_WEARING_CONF/LS_PPE_VIOLATION_CLASS/LS_PPE_CLASS_NAMES）
        PpeConfig {
            model_path: PathBuf::new(),
            imgsz: 640,
            iou: 0.50,
            violation_conf: 0.30,
            wearing_conf: 0.20,
            violation_class_name: "no lab coat".into(),
            wearing_class_names: vec!["lab coat".into()],
            use_gpu: true,
            clahe_enabled: true,
        }
    }
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

/// YOLO PPE 级联（违规类一次整图 + 穿戴类逐类一次）。
pub struct YoloPpeRunner {
    model: YoloModel,
    violation_class: usize,
    wearing_classes: Vec<usize>,
    violation_conf: f32,
    wearing_conf: f32,
    clahe_enabled: bool,
}

/// LS-043：升级框归属判定——要求交集占较小框面积 ≥ 0.20 或 PPE 框中心落入人框内，
/// 消除密集场景下 1 像素擦边误判。
fn boxes_overlap(ppe_box: [f32; 4], person_box: [f32; 4]) -> bool {
    let ix1 = ppe_box[0].max(person_box[0]);
    let iy1 = ppe_box[1].max(person_box[1]);
    let ix2 = ppe_box[2].min(person_box[2]);
    let iy2 = ppe_box[3].min(person_box[3]);
    if ix2 <= ix1 || iy2 <= iy1 {
        return false;
    }
    let cx = (ppe_box[0] + ppe_box[2]) * 0.5;
    let cy = (ppe_box[1] + ppe_box[3]) * 0.5;
    if cx >= person_box[0] && cx <= person_box[2] && cy >= person_box[1] && cy <= person_box[3] {
        return true;
    }
    let inter = (ix2 - ix1) * (iy2 - iy1);
    let area_ppe = ((ppe_box[2] - ppe_box[0]) * (ppe_box[3] - ppe_box[1])).max(1.0);
    let area_per = ((person_box[2] - person_box[0]) * (person_box[3] - person_box[1])).max(1.0);
    inter / area_ppe.min(area_per) >= 0.20
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
        if person_boxes.is_empty() {
            return Vec::new();
        }
        // LS-043：单次前向推理同时解码 violation + 全部 wearing 类（消除 2× 重复推理）
        let mut specs: Vec<(usize, f32, f32)> = Vec::with_capacity(1 + self.wearing_classes.len());
        specs.push((self.violation_class, self.violation_conf, 0.0));
        for &wc in &self.wearing_classes {
            specs.push((wc, self.wearing_conf, 0.0));
        }
        let Ok(mut all_dets) = self.model.detect_classes(bgr, w, h, &specs) else {
            return vec![PpeStatus::Unknown; person_boxes.len()];
        };
        let no_boxes_raw = if !all_dets.is_empty() {
            all_dets.remove(0)
        } else {
            Vec::new()
        };
        let no_boxes: Vec<[f32; 4]> = no_boxes_raw.iter().map(|d| d.box_).collect();
        let wear_boxes: Vec<[f32; 4]> = all_dets
            .into_iter()
            .flat_map(|v| v.into_iter().map(|d| d.box_))
            .collect();
        let frame_area = ((w as f32) * (h as f32)).max(1.0);
        person_boxes
            .iter()
            .map(|pb| {
                if no_boxes.iter().any(|nb| boxes_overlap(*nb, *pb)) {
                    return PpeStatus::NoPpe;
                }
                if wear_boxes.iter().any(|wb| boxes_overlap(*wb, *pb)) {
                    return PpeStatus::Wearing;
                }
                // LS-043：远景/灰度图整图无检出时，对较小人框裁切 ROI 二次精检（提升远景召回率）
                let box_area = ((pb[2] - pb[0]).max(0.0)) * ((pb[3] - pb[1]).max(0.0));
                if box_area / frame_area < 0.35 {
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
                        if let Ok(roi_dets) = self.model.detect_classes(&roi, rw, rh, &specs) {
                            let has_no = roi_dets.first().is_some_and(|v| !v.is_empty());
                            let has_wear = roi_dets.iter().skip(1).any(|v| !v.is_empty());
                            if has_no {
                                return PpeStatus::NoPpe;
                            }
                            if has_wear {
                                return PpeStatus::Wearing;
                            }
                        }
                    }
                }
                PpeStatus::Unknown
            })
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
    let find = |name: &str| -> Option<usize> {
        names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name.trim()))
    };
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
        Ok(model) => {
            println!(
                "[ls-analyzer] PPE 模型已加载: {}（违规类={violation_class} 穿戴类={wearing_classes:?}）",
                cfg.model_path.display()
            );
            Some(Box::new(YoloPpeRunner {
                model,
                violation_class,
                wearing_classes,
                violation_conf: cfg.violation_conf,
                wearing_conf: cfg.wearing_conf,
                clahe_enabled: cfg.clahe_enabled,
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
    let n = class_name.trim().to_ascii_lowercase().replace([' ', '-'], "_");
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

