use std::path::Path;

use crate::{ort_session::OrtSession, yolo::letterbox_nchw};

pub const POSENET_FILE: &str = "yolo26n-pose.onnx";
const SIZE: u32 = 640;
const COLUMNS: usize = 57;

pub struct Pose {
    pub bounds: [f32; 4],
    pub keypoints: [[f32; 3]; 17],
}

impl Pose {
    pub fn overlap(&self, other: &[f32; 4]) -> f32 {
        crate::post::iou(&self.bounds, other)
    }
}

pub struct PoseModel(OrtSession);

impl PoseModel {
    pub fn load(path: &Path, gpu: bool) -> Result<Self, String> {
        Ok(Self(OrtSession::load(path, gpu)?))
    }

    pub fn detect(&mut self, bgr: &[u8], width: u32, height: u32) -> Result<Vec<Pose>, String> {
        let (data, geom) = letterbox_nchw(bgr, width, height, SIZE)?;
        let input = ort::value::Tensor::from_array(([1, 3, SIZE as usize, SIZE as usize], data))
            .map_err(|err| err.to_string())?;
        self.0.with(None, move |session| {
            let out = session
                .run(ort::inputs![input])
                .map_err(|err| err.to_string())?;
            let (shape, values) = out[0]
                .try_extract_tensor::<f32>()
                .map_err(|err| err.to_string())?;
            if shape.len() != 3 || shape[0] != 1 || shape[2] != COLUMNS as i64 {
                return Err(format!("Unsupported pose output: {shape:?}"));
            }
            let x = |x: f32| (x - geom.dx) / geom.r;
            let y = |y: f32| (y - geom.dy) / geom.r;
            Ok(values
                .as_chunks::<COLUMNS>()
                .0
                .iter()
                .filter_map(|row| {
                    if row[4] < 0.35 || row[5] != 0.0 || !row.iter().all(|v| v.is_finite()) {
                        return None;
                    }
                    let mut keypoints = [[0.0; 3]; 17];
                    for (point, src) in keypoints.iter_mut().zip(row[6..].as_chunks::<3>().0) {
                        *point = [x(src[0]), y(src[1]), src[2]];
                    }
                    Some(Pose {
                        bounds: [x(row[0]), y(row[1]), x(row[2]), y(row[3])],
                        keypoints,
                    })
                })
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the released model and an acceptance frame"]
    fn pose_onnx_reads_real_keypoints() {
        let path = std::env::var_os("LS_POSE_SMOKE_MODEL").expect("pose model path");
        let frame =
            std::fs::read(std::env::var_os("LS_POSE_SMOKE_FRAME").expect("1280×720 BGR frame"))
                .unwrap();
        let gpu = std::env::var_os("LS_POSE_SMOKE_GPU").is_some();
        let mut model = PoseModel::load(Path::new(&path), gpu).unwrap();
        let poses = model.detect(&frame, 1280, 720).unwrap();
        assert!(poses
            .iter()
            .any(|pose| pose.keypoints[5][2] > 0.40 && pose.keypoints[11][2] > 0.40));
    }
}
