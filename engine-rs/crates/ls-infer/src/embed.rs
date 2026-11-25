use std::path::Path;

use ort::{session::SessionOutputs, value::Tensor};
use tokenizers::{Tokenizer, TruncationParams};

use crate::{ort_session::OrtSession, yolo::resize_u8_linear_cv2};

pub const DIMS: usize = 768;
pub const ENCODER: &str = "siglip2-base-patch16-224-int8";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
pub const TEXT_FILE: &str = "text_model_quantized.onnx";
pub const VISION_FILE: &str = "vision_model_quantized.onnx";
const IMAGE_SIZE: u32 = 224;
const TOKENS: usize = 64;

pub struct TextEncoder {
    tokenizer: Tokenizer,
    session: OrtSession,
}

impl TextEncoder {
    pub fn load(root: &Path) -> Result<Self, String> {
        let mut tokenizer =
            Tokenizer::from_file(root.join(TOKENIZER_FILE)).map_err(|err| err.to_string())?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: TOKENS,
                ..Default::default()
            }))
            .map_err(|err| err.to_string())?;
        let session = OrtSession::load_background_cpu(&root.join(TEXT_FILE))?;
        Ok(Self { tokenizer, session })
    }

    pub fn encode(&mut self, text: &str) -> Result<Vec<f32>, String> {
        let text = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let tokens = self
            .tokenizer
            .encode(text, true)
            .map_err(|err| err.to_string())?;
        let ids: Vec<_> = tokens.get_ids().iter().copied().map(i64::from).collect();
        if ids.len() != TOKENS {
            return Err("分词配置应输出 64 个 token".into());
        }
        let input = Tensor::from_array(([1, TOKENS], ids)).map_err(|err| err.to_string())?;
        self.session.with(None, |session| {
            let output = session
                .run(ort::inputs![input])
                .map_err(|err| err.to_string())?;
            embedding(&output, "pooler_output")
        })
    }
}

pub struct ImageEncoder(OrtSession);

impl ImageEncoder {
    pub fn load(root: &Path) -> Result<Self, String> {
        OrtSession::load_background_cpu(&root.join(VISION_FILE)).map(Self)
    }

    pub fn encode(&mut self, bgr: &[u8], width: u32, height: u32) -> Result<Vec<f32>, String> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|len| len.checked_mul(3));
        if width == 0 || height == 0 || expected != Some(bgr.len()) {
            return Err("语义索引图像尺寸无效".into());
        }
        let resized = resize_u8_linear_cv2(bgr, width, height, IMAGE_SIZE, IMAGE_SIZE, 3);
        let size = IMAGE_SIZE as usize;
        let pixels = size * size;
        let mut rgb = vec![0.0; pixels * 3];
        for (i, bgr) in resized.as_chunks::<3>().0.iter().enumerate() {
            for channel in 0..3 {
                rgb[channel * pixels + i] = f32::from(bgr[2 - channel]) / 127.5 - 1.0;
            }
        }
        let input = Tensor::from_array(([1, 3, size, size], rgb)).map_err(|err| err.to_string())?;
        self.0.with(None, |session| {
            let output = session
                .run(ort::inputs![input])
                .map_err(|err| err.to_string())?;
            embedding(&output, "pooler_output")
        })
    }
}

fn embedding(output: &SessionOutputs<'_>, name: &str) -> Result<Vec<f32>, String> {
    let embedding = output.get(name).ok_or_else(|| {
        format!(
            "ONNX 缺少 {name} 输出，现有输出 {:?}",
            output.keys().collect::<Vec<_>>()
        )
    })?;
    let (shape, values) = embedding
        .try_extract_tensor::<f32>()
        .map_err(|err| err.to_string())?;
    if shape.as_ref() != [1, DIMS as i64] {
        return Err(format!("不支持的编码输出形状: {shape:?}"));
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm < 1e-8 {
        return Err("语义编码输出无效".into());
    }
    Ok(values.iter().map(|value| value / norm).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires optional SigLIP 2 ONNX assets and the packaged ONNX Runtime"]
    fn siglip2_encodes_project_images_and_bilingual_queries() {
        let root = std::env::var_os("LS_SIGLIP_SMOKE_DIR").expect("asset directory");
        let root = Path::new(&root);
        let mut text = TextEncoder::load(root).unwrap();
        let mut image = ImageEncoder::load(root).unwrap();
        let person = image
            .encode(&std::fs::read(root.join("person.bgr")).unwrap(), 224, 224)
            .unwrap();
        let blue = image
            .encode(&std::fs::read(root.join("empty.bgr")).unwrap(), 224, 224)
            .unwrap();
        for query in ["a person wearing a white lab coat", "穿白大褂的人"] {
            let embedding = text.encode(query).unwrap();
            let cosine = |pixels: &[f32]| {
                pixels
                    .iter()
                    .zip(&embedding)
                    .map(|(a, b)| a * b)
                    .sum::<f32>()
            };
            let (white_coat, blue_coat) = (cosine(&person), cosine(&blue));
            eprintln!("{query}: white={white_coat:.4}, blue={blue_coat:.4}");
            assert!(white_coat > blue_coat);
        }
    }
}
