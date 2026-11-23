use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader, Limits};
use std::io::{Cursor, Read};
use std::path::Path;

const MAX_JPEG: usize = 2 * 1024 * 1024;
const MAX_SNAPSHOT: usize = 24 * 1024 * 1024;

pub fn read_snapshot(path: &Path) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_SNAPSHOT as u64 + 1)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() <= MAX_SNAPSHOT).then_some(data)
}

pub fn decode_image(bytes: &[u8]) -> Option<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().ok()
}

pub fn encode_jpeg(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let mut jpeg = Vec::new();
    let rgb = image.to_rgb8();
    JpegEncoder::new_with_quality(&mut jpeg, quality)
        .encode_image(&rgb)
        .ok()?;
    Some(jpeg)
}

pub fn snapshot_jpeg(path: &Path) -> Option<Vec<u8>> {
    let bytes = read_snapshot(path)?;
    if bytes.starts_with(&[0xff, 0xd8]) && bytes.len() <= MAX_JPEG {
        return Some(bytes);
    }
    let image = decode_image(&bytes)?;
    let image = image.thumbnail(1280, 720);
    let jpeg = encode_jpeg(&image, 82)?;
    (jpeg.len() <= MAX_JPEG).then_some(jpeg)
}
