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
                file = anchored_child(dir, c); // 越界为 None → 使用回退名
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
}

fn boxes_overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

impl PpeRunner for YoloPpeRunner {
    fn statuses(
        &mut self,
        bgr: &[u8],
        w: u32,
        h: u32,
        person_boxes: &[[f32; 4]],
    ) -> Vec<PpeStatus> {
        let (Ok(no_boxes_raw), Ok(wear_raw)) = (
            self.model
                .detect_class(bgr, w, h, self.violation_class, self.violation_conf, 0.0),
            self.model.detect_class(
                bgr,
                w,
                h,
                self.wearing_classes.first().copied().unwrap_or(0),
                self.wearing_conf,
                0.0,
            ),
        ) else {
            return vec![PpeStatus::Unknown; person_boxes.len()];
        };
        let no_boxes: Vec<[f32; 4]> = no_boxes_raw.iter().map(|d| d.box_).collect();
        let mut wear_boxes: Vec<[f32; 4]> = wear_raw.iter().map(|d| d.box_).collect();
        // 多个穿戴类：额外每类一次推理（当前配置通常 1 类；合并模型 Q-14 后降为 1 次）
        for class_id in self.wearing_classes.iter().skip(1) {
            if let Ok(dets) = self
                .model
                .detect_class(bgr, w, h, *class_id, self.wearing_conf, 0.0)
            {
                wear_boxes.extend(dets.iter().map(|d| d.box_));
            }
        }
        person_boxes
            .iter()
            .map(|pb| {
                if no_boxes.iter().any(|nb| boxes_overlap(*nb, *pb)) {
                    PpeStatus::NoPpe
                } else if wear_boxes.iter().any(|wb| boxes_overlap(*wb, *pb)) {
                    PpeStatus::Wearing
                } else {
                    PpeStatus::Unknown
                }
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
            }))
        }
        Err(e) => {
            println!("[ls-analyzer] PPE 模型加载失败（仅人员检测）: {e}");
            None
        }
    }
}
