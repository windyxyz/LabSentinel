//! tract 纯 Rust ONNX 推理（T-325 的可用实现）：YOLO11n 人体检测。
//!
//! 为什么是 tract：76 文档原定 ort，但本机 windows-gnu 目标缺 libclang，ort-sys 无法
//! 生成绑定；tract 纯 Rust、无 C++/动态库依赖，先打通"真实检测→跟踪→事件"闭环。
//! 预处理与 Python `engine.py` 对齐：letterbox 114 灰边 + 双线性缩放 + BGR→RGB +
//! /255 + NCHW；解码/NMS 复用 [`crate::post`]（与 Python 逐位对齐的既有实现）。

use std::path::Path;

use tract_onnx::prelude::*;

use crate::post::{self, Det, LetterboxGeom};

/// YOLO11 ONNX 会话（CPU，tract 优化图）。
pub struct YoloModel {
    model: std::sync::Arc<RunnableModel<TypedFact, Box<dyn TypedOp>>>,
    imgsz: u32,
    person_class: usize,
    conf: f32,
    iou: f32,
    min_box_area: f32,
}

impl YoloModel {
    /// 加载模型。`person_class` = COCO person 通道（0）。
    pub fn load(
        path: &Path,
        imgsz: u32,
        conf: f32,
        iou: f32,
        min_box_area: f32,
    ) -> Result<Self, String> {
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .map_err(|e| format!("加载模型失败: {e}"))?
            .into_optimized()
            .map_err(|e| format!("模型优化失败: {e}"))?
            .into_runnable()
            .map_err(|e| format!("模型编译失败: {e}"))?;
        Ok(YoloModel {
            model,
            imgsz,
            person_class: 0,
            conf,
            iou,
            min_box_area,
        })
    }

    /// 对一帧 BGR raw 做检测，返回原图坐标 xyxy（已 NMS + 最小面积过滤）。
    pub fn detect(&self, bgr: &[u8], w: u32, h: u32) -> Result<Vec<Det>, String> {
        let (data, geom) = letterbox_nchw(bgr, w, h, self.imgsz)?;
        let input = tract_ndarray::Array4::<f32>::from_shape_vec(
            (1, 3, self.imgsz as usize, self.imgsz as usize),
            data,
        )
        .map_err(|e| format!("输入张量形状错误: {e}"))?;
        let outputs = self
            .model
            .run(tvec!(input.into_tvalue()))
            .map_err(|e| format!("推理失败: {e}"))?;
        let view = outputs
            .first()
            .ok_or("模型无输出")?
            .to_plain_array_view::<f32>()
            .map_err(|e| format!("输出类型错误: {e}"))?;
        let shape: Vec<usize> = view.shape().to_vec();
        if shape.len() != 3 {
            return Err(format!("输出维度异常: {shape:?}"));
        }
        // (1, C, N) 或 (1, N, C)：YOLO 导出通常是 (1, 4+classes, anchors)
        let (c_major, channels, anchors) = if shape[1] <= shape[2] {
            (true, shape[1], shape[2])
        } else {
            (false, shape[2], shape[1])
        };
        let mut rows: Vec<Vec<f32>> = Vec::with_capacity(anchors);
        for i in 0..anchors {
            let mut row = Vec::with_capacity(channels);
            for c in 0..channels {
                let idx = if c_major {
                    tract_ndarray::IxDyn(&[0, c, i])
                } else {
                    tract_ndarray::IxDyn(&[0, i, c])
                };
                row.push(*view.get(idx).ok_or("输出索引越界")?);
            }
            rows.push(row);
        }
        let mut dets = post::pick_class(&rows, &[self.person_class], self.conf, &geom);
        dets.retain(|d| (d.box_[2] - d.box_[0]) * (d.box_[3] - d.box_[1]) >= self.min_box_area);
        Ok(post::nms(dets, self.iou))
    }
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
    fn detect_runs_repo_model_on_blank_frame() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let model = repo.join("m0-spike").join("yolo11n.onnx");
        if !model.exists() {
            eprintln!("skip: 仓库模型不存在");
            return;
        }
        let m = YoloModel::load(&model, 640, 0.4, 0.5, 1200.0).expect("模型可加载");
        let frame = vec![114u8; 640 * 640 * 3];
        let dets = m.detect(&frame, 640, 640).expect("推理可执行");
        assert!(dets.is_empty(), "纯灰图不应检出目标，实得 {dets:?}");
    }
}
