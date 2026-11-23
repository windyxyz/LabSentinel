//! 检测器构造（RA1.2 / ADR-0011）：从 main.rs 参数化迁出——sidecar 走 env 组装
//! `DetectorConfig`，ls-server 进程内 worker 从 DB/settings 组装，共用同一实现。
//!
//! 安全加固（T-325）随迁：模型路径必须 canonicalize 后为已存在普通文件。

use std::path::{Path, PathBuf};

use ls_infer::yolo::YoloModel;
use ls_ingest::Frame;

use crate::pipeline::{Detector, NullDetector, PersonDetection, PpeStatus, SceneObject};

const COCO_CONTEXT: [(usize, f32, f32); 4] = [
    (39, 0.40, 0.0),
    (40, 0.40, 0.0),
    (41, 0.40, 0.0),
    (67, 0.40, 0.0),
];
const COCO_LABELS: [&str; 4] = ["bottle", "wine_glass", "cup", "cell_phone"];

/// 检测器配置（进程级；RA1.2 起不再隐式读 env——组装方负责）。
#[derive(Debug, Clone)]
pub struct DetectorConfig {
    /// None = 未配置模型 → NullDetector（空检测，保留管线/事件通道）
    pub model_path: Option<PathBuf>,
    pub imgsz: u32,
    pub conf: f32,
    pub iou: f32,
    pub min_area: f32,
    /// UX（2026-11-21）：GPU 推理（DirectML——DX12 统一层，N/A/Intel 卡与核显
    /// 全通吃）。默认 true；DML 初始化失败时 ls-infer 自动回落 CPU。
    pub use_gpu: bool,
    pub coco_context: bool,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        // 默认值与 sidecar env 口径一致（LS_IMGSZ/LS_CONF_THRES/LS_IOU_THRES/LS_MIN_BOX_AREA）
        // GPU 默认开启（用户决策 2026-11-21：新设备默认 GPU，失败自动回落 CPU）
        DetectorConfig {
            model_path: None,
            imgsz: 640,
            conf: 0.40,
            iou: 0.50,
            min_area: 1200.0,
            use_gpu: true,
            coco_context: false,
        }
    }
}

impl DetectorConfig {
    /// sidecar 兼容：从进程 env 组装（原 main.rs make_detector 语义原样）。
    pub fn from_env(model_path: Option<PathBuf>) -> Self {
        DetectorConfig {
            model_path,
            imgsz: env_u32("LS_IMGSZ", 640),
            conf: env_f32("LS_CONF_THRES", 0.40),
            iou: env_f32("LS_IOU_THRES", 0.50),
            min_area: env_f32("LS_MIN_BOX_AREA", 1200.0),
            // env LS_GPU=0 可强制 CPU（默认 GPU）
            use_gpu: std::env::var("LS_GPU")
                .map(|v| v.trim() != "0")
                .unwrap_or(true),
            coco_context: false,
        }
    }
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|v: &f32| *v > 0.0)
        .unwrap_or(default)
}

/// 模型路径规范化与锚点校验（T-325 安全加固）：必须是**已存在的普通文件**，
/// 且 canonicalize 成功（消除相对路径/cwd 歧义与符号链接指向目录的读取尝试）。
/// 返回规范化路径；不满足返回 None（调用方回退空检测/仅人员检测）。
pub fn anchor_model_file(raw: &Path) -> Option<PathBuf> {
    let path = std::fs::canonicalize(raw).ok()?;
    if !path.is_file() {
        return None;
    }
    Some(path)
}

/// YOLO 检测器（tract/ort 双后端）。
pub struct YoloDetector {
    model: YoloModel,
    coco_context: bool,
    objects: Vec<SceneObject>,
}

impl Detector for YoloDetector {
    fn detect(&mut self, frame: &Frame) -> Vec<PersonDetection> {
        self.try_detect(frame).unwrap_or_else(|error| {
            eprintln!("[ls-analyzer] 推理失败（本帧跳过）: {error}");
            Vec::new()
        })
    }

    fn try_detect(&mut self, frame: &Frame) -> Result<Vec<PersonDetection>, String> {
        self.objects.clear();
        let detections =
            if self.coco_context {
                let (people, objects) = self.model.detect_with_context(
                    &frame.data,
                    frame.width,
                    frame.height,
                    &COCO_CONTEXT,
                )?;
                self.objects
                    .extend(COCO_LABELS.into_iter().zip(objects).flat_map(
                        |(label, detections)| {
                            detections.into_iter().map(move |detection| SceneObject {
                                label,
                                bbox: detection.box_,
                                confidence: detection.score,
                            })
                        },
                    ));
                people
            } else {
                self.model.detect(&frame.data, frame.width, frame.height)?
            };
        Ok(detections
            .into_iter()
            .map(|d| PersonDetection {
                box_: d.box_,
                score: d.score,
                ppe: PpeStatus::Unknown,
            })
            .collect())
    }

    fn objects(&self) -> &[SceneObject] {
        &self.objects
    }
}

/// 构造检测器：模型缺失/无效/加载失败均回退 `NullDetector`（与 sidecar 原语义一致）。
pub fn make_detector(cfg: &DetectorConfig) -> Box<dyn Detector> {
    load_detector(cfg).unwrap_or_else(|error| {
        eprintln!("[ls-analyzer] 人员检测模型不可用: {error}");
        Box::new(NullDetector)
    })
}

pub fn load_detector(cfg: &DetectorConfig) -> Result<Box<dyn Detector>, String> {
    let raw = cfg.model_path.as_ref().ok_or("未配置人员检测模型")?;
    let path = anchor_model_file(raw)
        .ok_or_else(|| format!("人员检测模型路径无效或不可读: {}", raw.display()))?;
    let model = YoloModel::load(
        &path,
        cfg.imgsz,
        cfg.conf,
        cfg.iou,
        cfg.min_area,
        None,
        cfg.use_gpu,
    )?;
    println!(
        "[ls-analyzer] 模型已加载 {}（imgsz={} conf={} iou={}）",
        path.display(),
        cfg.imgsz,
        cfg.conf,
        cfg.iou,
    );
    Ok(Box::new(YoloDetector {
        model,
        coco_context: cfg.coco_context,
        objects: Vec::new(),
    }))
}
