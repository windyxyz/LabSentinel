//! 检测器构造（RA1.2 / ADR-0011）：从 main.rs 参数化迁出——sidecar 走 env 组装
//! `DetectorConfig`，ls-server 进程内 worker 从 DB/settings 组装，共用同一实现。
//!
//! 安全加固（T-325）随迁：模型路径必须 canonicalize 后为已存在普通文件。

use std::path::{Path, PathBuf};

use ls_infer::yolo::YoloModel;
use ls_ingest::Frame;

use crate::pipeline::{Detector, NullDetector, PersonDetection, PpeStatus};

/// 检测器配置（进程级；RA1.2 起不再隐式读 env——组装方负责）。
#[derive(Debug, Clone)]
pub struct DetectorConfig {
    /// None = 未配置模型 → NullDetector（空检测，保留管线/事件通道）
    pub model_path: Option<PathBuf>,
    pub imgsz: u32,
    pub conf: f32,
    pub iou: f32,
    pub min_area: f32,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        // 默认值与 sidecar env 口径一致（LS_IMGSZ/LS_CONF_THRES/LS_IOU_THRES/LS_MIN_BOX_AREA）
        DetectorConfig {
            model_path: None,
            imgsz: 640,
            conf: 0.40,
            iou: 0.50,
            min_area: 1200.0,
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
}

impl Detector for YoloDetector {
    fn detect(&mut self, frame: &Frame) -> Vec<PersonDetection> {
        match self.model.detect(&frame.data, frame.width, frame.height) {
            Ok(dets) => dets
                .into_iter()
                .map(|d| PersonDetection {
                    box_: d.box_,
                    score: d.score,
                    ppe: PpeStatus::Unknown,
                })
                .collect(),
            Err(e) => {
                eprintln!("[ls-analyzer] 推理失败（本帧跳过）: {e}");
                Vec::new()
            }
        }
    }
}

/// 构造检测器：模型缺失/无效/加载失败均回退 `NullDetector`（与 sidecar 原语义一致）。
pub fn make_detector(cfg: &DetectorConfig) -> Box<dyn Detector> {
    let Some(raw) = &cfg.model_path else {
        println!("[ls-analyzer] 未配置模型——使用空检测器");
        return Box::new(NullDetector);
    };
    // T-325 安全加固：模型路径规范化 + 普通文件锚点（相对路径/目录/符号链接逃逸拒绝）
    let Some(path) = anchor_model_file(raw) else {
        println!(
            "[ls-analyzer] 模型路径无效/不可读（空检测）: {}",
            raw.display()
        );
        return Box::new(NullDetector);
    };
    match YoloModel::load(&path, cfg.imgsz, cfg.conf, cfg.iou, cfg.min_area, None) {
        Ok(model) => {
            println!(
                "[ls-analyzer] 模型已加载 {}（imgsz={} conf={} iou={}）",
                path.display(),
                cfg.imgsz,
                cfg.conf,
                cfg.iou
            );
            Box::new(YoloDetector { model })
        }
        Err(e) => {
            println!("[ls-analyzer] 模型加载失败（回退空检测）: {e}");
            Box::new(NullDetector)
        }
    }
}
