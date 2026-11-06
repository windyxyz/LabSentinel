//! 推理后端（T-325）：YOLO11n 人体检测，双后端可切换。
//!
//! - `tract`（feature=tract）：纯 Rust ONNX 运行时，无 C++/动态库依赖——
//!   windows-gnu 直接构建，"无 ORT 环境"的兜底实现；
//! - `ort`（feature=ort，T-325 原定方案/T-328 验收口径）：`load-dynamic` 运行时加载
//!   `ORT_DYLIB_PATH` 指定的 onnxruntime.dll——与 Python 同后端，黄金语料事件一致率
//!   ≥98% 的验收在此口径下进行。rc.10+ 绑定层纯 Rust（无 bindgen/libclang）。
//!
//! 预处理与 Python `engine.py` 对齐：letterbox 114 灰边 + 双线性缩放 + BGR→RGB +
//! /255 + NCHW；解码/NMS 复用 [`crate::post`]（与 Python 逐位对齐的既有实现）。
//! 后端由 [`YoloModel::load`] 的 `backend` 参数或环境变量 `LS_INFER_BACKEND`
//! （"tract"|"ort"）选择。

use std::path::Path;

#[cfg(feature = "ort")]
use ort::session::Session;
#[cfg(feature = "tract")]
use tract_onnx::prelude::*;

use crate::post::{self, Det, LetterboxGeom};

/// 推理后端种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Tract,
    Ort,
}

impl Backend {
    /// 从环境变量解析（缺省 tract；仅当对应 feature 启用且可用时返回 Ort）。
    pub fn from_env() -> Option<Backend> {
        match std::env::var("LS_INFER_BACKEND")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "ort" => {
                #[cfg(feature = "ort")]
                {
                    Some(Backend::Ort)
                }
                #[cfg(not(feature = "ort"))]
                {
                    eprintln!("[ls-infer] LS_INFER_BACKEND=ort 但 ort feature 未启用——回退 tract");
                    #[cfg(feature = "tract")]
                    {
                        Some(Backend::Tract)
                    }
                    #[cfg(not(feature = "tract"))]
                    {
                        None
                    }
                }
            }
            "tract" => {
                #[cfg(feature = "tract")]
                {
                    Some(Backend::Tract)
                }
                #[cfg(not(feature = "tract"))]
                {
                    None
                }
            }
            _ => {
                #[cfg(feature = "tract")]
                {
                    Some(Backend::Tract) // 缺省 tract（零外部依赖）
                }
                #[cfg(not(feature = "tract"))]
                {
                    None
                }
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Tract => "tract",
            Backend::Ort => "ort",
        }
    }
}

/// YOLO11 ONNX 会话（后端无关；`person_class` = COCO person 通道 0）。
pub struct YoloModel {
    backend: Backend,
    #[cfg(feature = "tract")]
    tract_model: Option<std::sync::Arc<RunnableModel<TypedFact, Box<dyn TypedOp>>>>,
    #[cfg(feature = "ort")]
    ort_session: Option<Session>,
    imgsz: u32,
    person_class: usize,
    conf: f32,
    iou: f32,
    min_box_area: f32,
}

impl YoloModel {
    /// 加载模型。`backend=None` 时按 `LS_INFER_BACKEND` 解析（缺省 tract）。
    pub fn load(
        path: &Path,
        imgsz: u32,
        conf: f32,
        iou: f32,
        min_box_area: f32,
        backend: Option<Backend>,
    ) -> Result<Self, String> {
        let backend = backend
            .or_else(Backend::from_env)
            .ok_or("无可用推理后端（tract/ort feature 均未启用）")?;
        let mut model = YoloModel {
            backend,
            #[cfg(feature = "tract")]
            tract_model: None,
            #[cfg(feature = "ort")]
            ort_session: None,
            imgsz,
            person_class: 0,
            conf,
            iou,
            min_box_area,
        };
        match backend {
            #[cfg(feature = "tract")]
            Backend::Tract => {
                let m = tract_onnx::onnx()
                    .model_for_path(path)
                    .map_err(|e| format!("加载模型失败: {e}"))?
                    .into_optimized()
                    .map_err(|e| format!("模型优化失败: {e}"))?
                    .into_runnable()
                    .map_err(|e| format!("模型编译失败: {e}"))?;
                model.tract_model = Some(m);
            }
            #[cfg(not(feature = "tract"))]
            Backend::Tract => return Err("tract feature 未启用".into()),
            #[cfg(feature = "ort")]
            Backend::Ort => {
                let bytes = std::fs::read(path).map_err(|e| format!("读取模型失败: {e}"))?;
                let environment = ort::environment::init()
                    .with_name("labsentinel")
                    .build()
                    .map_err(|e| format!("ORT 环境初始化失败: {e}（ORT_DYLIB_PATH 是否设置？）"))?;
                let session = Session::builder(&environment)
                    .map_err(|e| format!("ORT builder 失败: {e}"))?
                    .commit_from_memory(&bytes)
                    .map_err(|e| format!("ORT 加载模型失败: {e}"))?;
                model.ort_session = Some(session);
            }
            #[cfg(not(feature = "ort"))]
            Backend::Ort => return Err("ort feature 未启用".into()),
        }
        Ok(model)
    }

    /// 对一帧 BGR raw 做检测，返回原图坐标 xyxy（已 NMS + 最小面积过滤）。
    pub fn detect(&mut self, bgr: &[u8], w: u32, h: u32) -> Result<Vec<Det>, String> {
        let (data, geom) = letterbox_nchw(bgr, w, h, self.imgsz)?;
        match self.backend {
            #[cfg(feature = "tract")]
            Backend::Tract => {
                let input = tract_ndarray::Array4::<f32>::from_shape_vec(
                    (1, 3, self.imgsz as usize, self.imgsz as usize),
                    data,
                )
                .map_err(|e| format!("输入张量形状错误: {e}"))?;
                let outputs = self
                    .tract_model
                    .as_ref()
                    .ok_or("tract 会话未初始化")?
                    .run(tvec!(input.into_tvalue()))
                    .map_err(|e| format!("推理失败: {e}"))?;
                let view = outputs
                    .first()
                    .ok_or("模型无输出")?
                    .to_plain_array_view::<f32>()
                    .map_err(|e| format!("输出类型错误: {e}"))?;
                let shape: Vec<usize> = view.shape().to_vec();
                let rows = rows_from_dyn(&shape, &mut |idx| {
                    view.get(tract_ndarray::IxDyn(idx))
                        .copied()
                        .ok_or_else(|| "输出索引越界".to_string())
                })?;
                Self::decode_static(
                    &rows,
                    &geom,
                    self.person_class,
                    self.conf,
                    self.iou,
                    self.min_box_area,
                )
            }
            #[cfg(not(feature = "tract"))]
            Backend::Tract => Err("tract feature 未启用".into()),
            #[cfg(feature = "ort")]
            Backend::Ort => {
                let session = self.ort_session.as_mut().ok_or("ORT 会话未初始化")?;
                let tensor = ort::value::Tensor::from_array((
                    [1usize, 3, self.imgsz as usize, self.imgsz as usize],
                    data,
                ))
                .map_err(|e| format!("输入张量创建失败: {e}"))?;
                let outputs = session
                    .run(ort::inputs![tensor])
                    .map_err(|e| format!("推理失败: {e}"))?;
                // YOLO 单输出：取首个输出（SessionOutputs 可迭代，取第一个键值对）
                let (_name, v) = outputs
                    .iter()
                    .next()
                    .ok_or("模型无输出")?;
                let (shape, flat) = v
                    .try_extract_tensor::<f32>()
                    .map_err(|e| format!("输出类型错误: {e}"))?;
                let shape: Vec<usize> = shape.iter().map(|d| *d as usize).collect();
                let rows = rows_from_flat(&shape, flat)?;
                Self::decode_static(
                    &rows,
                    &geom,
                    self.person_class,
                    self.conf,
                    self.iou,
                    self.min_box_area,
                )
            }
            #[cfg(not(feature = "ort"))]
            Backend::Ort => Err("ort feature 未启用".into()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_static(
        rows: &[Vec<f32>],
        geom: &LetterboxGeom,
        person_class: usize,
        conf: f32,
        iou: f32,
        min_box_area: f32,
    ) -> Result<Vec<Det>, String> {
        let mut dets = post::pick_class(rows, &[person_class], conf, geom);
        dets.retain(|d| (d.box_[2] - d.box_[0]) * (d.box_[3] - d.box_[1]) >= min_box_area);
        Ok(post::nms(dets, iou))
    }
}

/// 从动态维度视图取行（tract 路径）。
#[cfg(feature = "tract")]
fn rows_from_dyn(
    shape: &[usize],
    get: &mut dyn FnMut(&[usize]) -> Result<f32, String>,
) -> Result<Vec<Vec<f32>>, String> {
    if shape.len() != 3 || shape[0] != 1 {
        return Err(format!("输出维度异常: {shape:?}"));
    }
    let (c_major, channels, anchors) = if shape[1] <= shape[2] {
        (true, shape[1], shape[2])
    } else {
        (false, shape[2], shape[1])
    };
    let mut rows = Vec::with_capacity(anchors);
    for i in 0..anchors {
        let mut row = Vec::with_capacity(channels);
        for c in 0..channels {
            let idx: &[usize] = if c_major { &[0, c, i] } else { &[0, i, c] };
            row.push(get(idx)?);
        }
        rows.push(row);
    }
    Ok(rows)
}

/// 从平坦数据 + 形状取行（ort 路径）。
fn rows_from_flat(shape: &[usize], flat: &[f32]) -> Result<Vec<Vec<f32>>, String> {
    if shape.len() != 3 || shape[0] != 1 {
        return Err(format!("输出维度异常: {shape:?}"));
    }
    rows_from_shape_dims(shape, |channels, anchors, c_major, i, c| {
        let idx = if c_major {
            c * anchors + i
        } else {
            i * channels + c
        };
        flat.get(idx)
            .copied()
            .ok_or_else(|| "输出索引越界".to_string())
    })
}

/// (1, C, N) / (1, N, C) 双布局的公共行提取（YOLO 导出通常 (1, 4+classes, anchors)）。
fn rows_from_shape_dims(
    shape: &[usize],
    mut get: impl FnMut(usize, usize, bool, usize, usize) -> Result<f32, String>,
) -> Result<Vec<Vec<f32>>, String> {
    if shape.len() != 3 || shape[0] != 1 {
        return Err(format!("输出维度异常: {shape:?}"));
    }
    let (c_major, channels, anchors) = if shape[1] <= shape[2] {
        (true, shape[1], shape[2])
    } else {
        (false, shape[2], shape[1])
    };
    let mut rows = Vec::with_capacity(anchors);
    for i in 0..anchors {
        let mut row = Vec::with_capacity(channels);
        for c in 0..channels {
            row.push(get(channels, anchors, c_major, i, c)?);
        }
        rows.push(row);
    }
    Ok(rows)
}

/// letterbox + BGR→RGB + /255 + NCHW（与 Python `_letterbox` 同几何）。
pub fn letterbox_nchw(
    bgr: &[u8],
    w: u32,
    h: u32,
    s: u32,
) -> Result<(Vec<f32>, LetterboxGeom), String> {
    let expect = (w as usize) * (h as usize) * 3;
    if bgr.len() < expect || w == 0 || h == 0 || s == 0 {
        return Err(format!(
            "帧尺寸不符: len={} expect={expect} w={w} h={h}",
            bgr.len()
        ));
    }
    let g = post::letterbox_geom(w, h, s);
    let nw = ((w as f32 * g.r).floor() as u32).max(1).min(s);
    let nh = ((h as f32 * g.r).floor() as u32).max(1).min(s);
    let pad = 114.0f32 / 255.0;
    let mut data = vec![pad; (s as usize) * (s as usize) * 3];
    let (sw, sh) = (w as f32, h as f32);
    let plane = (s as usize) * (s as usize);
    for y in 0..nh {
        let sy = (((y as f32) + 0.5) * sh / nh as f32 - 0.5).max(0.0);
        let y0 = sy.floor() as u32;
        let y1 = (y0 + 1).min(h - 1);
        let wy = sy - y0 as f32;
        let py = (g.dy as u32 + y) as usize;
        for x in 0..nw {
            let sx = (((x as f32) + 0.5) * sw / nw as f32 - 0.5).max(0.0);
            let x0 = sx.floor() as u32;
            let x1 = (x0 + 1).min(w - 1);
            let wx = sx - x0 as f32;
            let px = (g.dx as u32 + x) as usize;
            for c in 0..3usize {
                let i00 = (((y0 * w + x0) * 3) as usize) + c;
                let i10 = (((y0 * w + x1) * 3) as usize) + c;
                let i01 = (((y1 * w + x0) * 3) as usize) + c;
                let i11 = (((y1 * w + x1) * 3) as usize) + c;
                let v00 = bgr[i00] as f32;
                let v10 = bgr[i10] as f32;
                let v01 = bgr[i01] as f32;
                let v11 = bgr[i11] as f32;
                let v = (v00 * (1.0 - wx) * (1.0 - wy)
                    + v10 * wx * (1.0 - wy)
                    + v01 * (1.0 - wx) * wy
                    + v11 * wx * wy)
                    / 255.0;
                let out_c = 2 - c; // BGR → RGB
                data[out_c * plane + py * s as usize + px] = v;
            }
        }
    }
    Ok((data, g))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_letterbox_padding_is_114() {
        let frame = vec![0u8; 320 * 240 * 3];
        let (data, g) = letterbox_nchw(&frame, 320, 240, 640).unwrap();
        assert_eq!(data.len(), 3 * 640 * 640);
        // 320x240 → r=2, nw=640, nh=480, dy=80：顶部 80 行是 114 灰边
        assert!((data[0] - 114.0 / 255.0).abs() < 1e-6);
        // 画布中心落在缩放后的图像内（黑）
        let center = (2 * 640 * 640) + 320 * 640 + 320;
        assert!((data[center]).abs() < 1e-6);
        assert!((g.dy - 80.0).abs() < 1e-6);
        assert!((g.dx - 0.0).abs() < 1e-6);
    }

    #[test]
    fn rows_from_flat_handles_both_layouts() {
        // 启发式 shape[1] <= shape[2] → C-major：(1,2,3) channels=2 anchors=3
        let rows = rows_from_flat(&[1, 2, 3], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec![1.0, 4.0]);
        assert_eq!(rows[1], vec![2.0, 5.0]);
        assert_eq!(rows[2], vec![3.0, 6.0]);
        // (1,3,2) → N-major：channels=2 anchors=3
        let rows = rows_from_flat(&[1, 3, 2], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        assert_eq!(rows[0], vec![1.0, 2.0]);
        assert_eq!(rows[1], vec![3.0, 4.0]);
        assert_eq!(rows[2], vec![5.0, 6.0]);
    }

    #[test]
    fn detect_runs_repo_model_on_blank_frame() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let model = repo.join("m0-spike").join("yolo11n.onnx");
        if !model.exists() {
            eprintln!("skip: 仓库模型不存在");
            return;
        }
        let mut m = YoloModel::load(&model, 640, 0.4, 0.5, 1200.0, None).expect("模型可加载");
        let frame = vec![114u8; 640 * 640 * 3];
        let dets = m.detect(&frame, 640, 640).expect("推理可执行");
        assert!(dets.is_empty(), "纯灰图不应检出目标，实得 {dets:?}");
    }
}
