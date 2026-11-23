//! YuNet face detection and SFace embedding via the shared ONNX Runtime.
//! The pinned OpenCV Zoo exports use BGR 640×640 and RGB 112×112 respectively.

use std::path::Path;

use ort::session::{Session, SessionOutputs};
use ort::value::{Tensor, TensorElementType, ValueType};

use crate::{post, yolo};

const DETECTOR_SIDE: usize = 640;
const EMBEDDER_SIDE: usize = 112;
pub const EMBEDDING_SIZE: usize = 128;
const LANDMARKS: [[f32; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];

struct FaceSession {
    session: Option<Session>,
    gpu: bool,
}

impl FaceSession {
    fn load(path: &Path, gpu: bool, side: usize) -> Result<Self, String> {
        let (session, gpu) = yolo::load_ort_session(path, gpu)?;
        let model = Self {
            session: Some(session),
            gpu,
        };
        let inputs = model.session.as_ref().expect("loaded").inputs();
        let expected = [1, 3, side as i64, side as i64];
        if !matches!(inputs.first().map(|i| i.dtype()), Some(ValueType::Tensor {
            ty: TensorElementType::Float32, shape, ..
        }) if shape.as_ref() == expected)
        {
            return Err(format!("人脸 ONNX 输入必须是 FP32 {expected:?}"));
        }
        Ok(model)
    }

    fn run<T>(
        &mut self,
        side: usize,
        image: Vec<f32>,
        read: impl FnOnce(&SessionOutputs<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let tensor = Tensor::from_array(([1, 3, side, side], image)).map_err(|e| e.to_string())?;
        let _guard = self
            .gpu
            .then(|| yolo::dml_lock().lock().unwrap_or_else(|e| e.into_inner()));
        let outputs = self
            .session
            .as_mut()
            .expect("loaded")
            .run(ort::inputs![tensor])
            .map_err(|e| format!("人脸模型推理失败: {e}"))?;
        read(&outputs)
    }
}

impl Drop for FaceSession {
    fn drop(&mut self) {
        if self.gpu {
            let _guard = yolo::dml_lock().lock().unwrap_or_else(|e| e.into_inner());
            self.session.take();
        }
    }
}

pub struct LocalFaceModel {
    detector: FaceSession,
    embedder: FaceSession,
}

impl LocalFaceModel {
    pub fn load(detector: &Path, embedder: &Path, gpu: bool) -> Result<Self, String> {
        Ok(Self {
            detector: FaceSession::load(detector, gpu, DETECTOR_SIDE)?,
            embedder: FaceSession::load(embedder, gpu, EMBEDDER_SIDE)?,
        })
    }

    pub fn devices(&self) -> [bool; 2] {
        [self.detector.gpu, self.embedder.gpu]
    }

    pub fn embedding(&mut self, bgr: &[u8], w: u32, h: u32) -> Result<Vec<f32>, String> {
        if w == 0 || h == 0 || w > 4096 || h > 4096 || bgr.len() != w as usize * h as usize * 3 {
            return Err("图片尺寸无效".into());
        }
        let side = DETECTOR_SIDE as f32;
        let scale = (side / w as f32).min(side / h as f32);
        let sw = (w as f32 * scale).round().clamp(1.0, side) as u32;
        let sh = (h as f32 * scale).round().clamp(1.0, side) as u32;
        let pixels = yolo::resize_u8_linear_cv2(bgr, w, h, sw, sh, 3);
        let mut input = vec![0.0; 3 * DETECTOR_SIDE * DETECTOR_SIDE];
        for (row, line) in pixels.chunks_exact(3 * sw as usize).enumerate() {
            for (col, pixel) in line.as_chunks::<3>().0.iter().enumerate() {
                for (channel, value) in pixel.iter().enumerate() {
                    input[(channel * DETECTOR_SIDE + row) * DETECTOR_SIDE + col] =
                        f32::from(*value);
                }
            }
        }
        let raw = self.detector.run(DETECTOR_SIDE, input, decode_yunet)?;
        let faces = suppress_overlaps(raw, sw as f32, sh as f32);
        let face = match faces.as_slice() {
            [] => return Err("未找到清晰的人脸，请正对镜头并靠近一些".into()),
            [face] => face,
            _ => return Err("画面含多张人脸，请只保留一人".into()),
        };
        let mut marks = face.landmarks;
        for [x, y] in &mut marks {
            *x *= w as f32 / sw as f32;
            *y *= h as f32 / sh as f32;
        }
        let aligned =
            align_rgb(bgr, w as usize, h as usize, marks).ok_or("人脸角度过大或关键点不足")?;
        let mut embedding = self.embedder.run(EMBEDDER_SIDE, aligned, |out| {
            Ok(tensor_data(out, "fc1", EMBEDDING_SIZE)?.to_vec())
        })?;
        let norm = embedding
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        if !norm.is_finite() || norm <= f32::EPSILON {
            return Err("人脸模板无效，请换一张清晰照片".into());
        }
        for component in &mut embedding {
            *component /= norm;
        }
        Ok(embedding)
    }
}

struct FaceCandidate {
    bounds: [f32; 4],
    landmarks: [[f32; 2]; 5],
    score: f32,
}

fn tensor_data<'a>(
    out: &'a SessionOutputs<'_>,
    key: &str,
    count: usize,
) -> Result<&'a [f32], String> {
    let value = out
        .get(key)
        .ok_or_else(|| format!("人脸模型缺少输出 {key}"))?;
    let (_, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;
    if data.len() != count {
        return Err(format!("人脸模型输出 {key} 维度不匹配"));
    }
    Ok(data)
}

fn decode_yunet(out: &SessionOutputs<'_>) -> Result<Vec<FaceCandidate>, String> {
    let mut faces = Vec::new();
    for stride in [8, 16, 32] {
        let cols = DETECTOR_SIDE / stride;
        let count = cols * cols;
        let cls = tensor_data(out, &format!("cls_{stride}"), count)?;
        let obj = tensor_data(out, &format!("obj_{stride}"), count)?;
        let boxes = tensor_data(out, &format!("bbox_{stride}"), 4 * count)?;
        let points = tensor_data(out, &format!("kps_{stride}"), 10 * count)?;
        for i in 0..count {
            let score = (cls[i].clamp(0.0, 1.0) * obj[i].clamp(0.0, 1.0)).sqrt();
            if !score.is_finite() || score < 0.8 {
                continue;
            }
            let grid_x = (i % cols) as f32;
            let grid_y = (i / cols) as f32;
            let stride = stride as f32;
            let center_x = (grid_x + boxes[4 * i]) * stride;
            let center_y = (grid_y + boxes[4 * i + 1]) * stride;
            let width = boxes[4 * i + 2].exp() * stride;
            let height = boxes[4 * i + 3].exp() * stride;
            let bounds = [
                center_x - width / 2.0,
                center_y - height / 2.0,
                center_x + width / 2.0,
                center_y + height / 2.0,
            ];
            let landmarks = std::array::from_fn(|j| {
                [
                    (grid_x + points[10 * i + 2 * j]) * stride,
                    (grid_y + points[10 * i + 2 * j + 1]) * stride,
                ]
            });
            if bounds.iter().all(|p| p.is_finite()) {
                faces.push(FaceCandidate {
                    bounds,
                    landmarks,
                    score,
                });
            }
        }
    }
    Ok(faces)
}

fn suppress_overlaps(mut candidates: Vec<FaceCandidate>, w: f32, h: f32) -> Vec<FaceCandidate> {
    candidates.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
    let mut faces: Vec<FaceCandidate> = Vec::new();
    for mut face in candidates.into_iter().take(128) {
        let [x1, y1, x2, y2] = face.bounds;
        face.bounds = [
            x1.clamp(0.0, w),
            y1.clamp(0.0, h),
            x2.clamp(0.0, w),
            y2.clamp(0.0, h),
        ];
        let [x1, y1, x2, y2] = face.bounds;
        if x2 - x1 < 24.0
            || y2 - y1 < 24.0
            || faces
                .iter()
                .any(|f| post::iou(&f.bounds, &face.bounds) > 0.3)
        {
            continue;
        }
        faces.push(face);
        if faces.len() == 2 {
            break;
        }
    }
    faces
}

fn align_rgb(bgr: &[u8], w: usize, h: usize, landmarks: [[f32; 2]; 5]) -> Option<Vec<f32>> {
    let center = |points: &[[f32; 2]; 5]| {
        points
            .iter()
            .fold([0.0, 0.0], |[x, y], p| [x + p[0] / 5.0, y + p[1] / 5.0])
    };
    let [sx, sy] = center(&landmarks);
    let [tx, ty] = center(&LANDMARKS);
    let (mut a, mut b, mut spread) = (0.0, 0.0, 0.0);
    for ([x, y], [u, v]) in landmarks.into_iter().zip(LANDMARKS) {
        let (x, y, u, v) = (x - sx, y - sy, u - tx, v - ty);
        a += x * u + y * v;
        b += x * v - y * u;
        spread += x * x + y * y;
    }
    let square = a * a + b * b;
    if !square.is_finite() || square <= f32::EPSILON || spread <= f32::EPSILON {
        return None;
    }
    (a, b) = (a * spread / square, b * spread / square);
    let mut input = vec![0.0; 3 * EMBEDDER_SIDE * EMBEDDER_SIDE];
    for row in 0..EMBEDDER_SIDE {
        for col in 0..EMBEDDER_SIDE {
            let (u, v) = (col as f32 - tx, row as f32 - ty);
            let (x, y) = (a * u + b * v + sx, -b * u + a * v + sy);
            if !(-1.0..w as f32).contains(&x) || !(-1.0..h as f32).contains(&y) {
                continue;
            }
            let (ix, iy) = (x.floor() as i32, y.floor() as i32);
            let (fx, fy) = (x - ix as f32, y - iy as f32);
            let sample = |x: i32, y: i32, channel: usize| {
                if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
                    return 0.0;
                }
                f32::from(bgr[(y as usize * w + x as usize) * 3 + channel])
            };
            for ch in 0..3 {
                let channel = 2 - ch;
                let top = (1.0 - fx) * sample(ix, iy, channel) + fx * sample(ix + 1, iy, channel);
                let bottom =
                    (1.0 - fx) * sample(ix, iy + 1, channel) + fx * sample(ix + 1, iy + 1, channel);
                input[(ch * EMBEDDER_SIDE + row) * EMBEDDER_SIDE + col] =
                    ((1.0 - fy) * top + fy * bottom).round().clamp(0.0, 255.0);
            }
        }
    }
    Some(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_keeps_rgb_order_and_rejects_degenerate_landmarks() {
        let bgr = [10, 20, 40].repeat(112 * 112);
        let rgb = align_rgb(&bgr, 112, 112, LANDMARKS).unwrap();
        let plane = 112 * 112;
        assert_eq!(rgb[56 * 112 + 56], 40.0);
        assert_eq!(rgb[plane + 56 * 112 + 56], 20.0);
        assert_eq!(rgb[plane * 2 + 56 * 112 + 56], 10.0);
        assert!(align_rgb(&bgr, 112, 112, [[10.0, 10.0]; 5]).is_none());
    }
}
