//! LS-055：纯 Rust 轻量 CLAHE（限制对比度自适应直方图均衡）——灰度/低照度
//! 监控图 ROI 增强。零依赖、无 unsafe；输出尺寸与输入一致，像素范围 0..=255。
//!
//! 算法：把 ROI 划为 tiles_x × tiles_y 块，逐块计算灰度直方图并按 clip_limit
//! 截断重分配，得到块级映射表；对每个像素做双线性插值（块中心）取映射值。
//! 只对 Y（灰度）通道生效，色度保持不变（BGR 三通道按 Y 缩放比例同步调整，
//! 简化实现：对每个通道按同一增益缩放并 clamp）。

/// CLAHE 参数。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClaheConfig {
    pub tiles_x: usize,
    pub tiles_y: usize,
    /// 直方图裁剪上限（相对于平均每 bin 的倍数，越大对比度越强）。
    pub clip_limit: f32,
}

impl Default for ClaheConfig {
    fn default() -> Self {
        Self {
            tiles_x: 8,
            tiles_y: 8,
            clip_limit: 2.0,
        }
    }
}

/// 对 BGR ROI 做 CLAHE 增强（就地返回新 Vec，尺寸不变）。
pub fn clahe_bgr(bgr: &[u8], w: u32, h: u32, cfg: ClaheConfig) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || bgr.len() < w * h * 3 {
        return bgr.to_vec();
    }
    let tx = cfg.tiles_x.clamp(1, 32);
    let ty = cfg.tiles_y.clamp(1, 32);
    // 1) 灰度 + 块直方图 + 映射表
    let gray: Vec<u8> = (0..w * h)
        .map(|i| {
            let (b, g, r) = (
                bgr[i * 3] as u32,
                bgr[i * 3 + 1] as u32,
                bgr[i * 3 + 2] as u32,
            );
            // ITU-R BT.601 灰度（BGR 通道顺序：0.114*B + 0.587*G + 0.299*R）
            ((29 * b + 150 * g + 77 * r) / 256) as u8
        })
        .collect();
    // 全局直方图映射（与块映射混合，避免"单色块"下局部 CDF 退化）
    let mut ghist = [0u32; 256];
    for g in &gray {
        ghist[*g as usize] += 1;
    }
    let mut gcdf = 0u32;
    let total = (w * h) as u32;
    let mut gmap = [0u8; 256];
    for i in 0..256 {
        gcdf += ghist[i];
        gmap[i] = ((gcdf as f32 / total.max(1) as f32) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    let mut maps: Vec<[u8; 256]> = Vec::with_capacity(tx * ty);
    for tyi in 0..ty {
        let y0 = tyi * h / ty;
        let y1 = ((tyi + 1) * h / ty).max(y0 + 1).min(h);
        for txi in 0..tx {
            let x0 = txi * w / tx;
            let x1 = ((txi + 1) * w / tx).max(x0 + 1).min(w);
            let mut hh = [0u32; 256];
            let mut n = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    hh[gray[y * w + x] as usize] += 1;
                    n += 1;
                }
            }
            let avg = (n / 256).max(1) as f32;
            let clip = (avg * cfg.clip_limit.max(1.0)) as u32;
            let mut excess = 0u32;
            for v in hh.iter_mut() {
                if *v > clip {
                    excess += *v - clip;
                    *v = clip;
                }
            }
            // 均匀重分配截断量（含余数，避免小 ROI 块 excess<256 时被丢弃导致整块变黑）
            let add = excess / 256;
            let rem = (excess % 256) as usize;
            for (idx, v) in hh.iter_mut().enumerate() {
                *v += add + u32::from(idx < rem);
            }
            // 累计分布 → 映射；与全局映射 50/50 混合（单色块退化保护）
            let mut cdf = 0u32;
            let mut map = [0u8; 256];
            for i in 0..256 {
                cdf += hh[i];
                let local = if n > 0 {
                    ((cdf as f32 / n as f32) * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    i as u8
                };
                map[i] = ((local as u16 + gmap[i] as u16) / 2) as u8;
            }
            maps.push(map);
        }
    }
    // 2) 双线性插值取块映射
    let mut out = vec![0u8; w * h * 3];
    for y in 0..h {
        // 块坐标（以块中心为参考）
        let fy = (y as f32 + 0.5) * ty as f32 / h as f32 - 0.5;
        let ty0 = fy.floor().max(0.0) as usize;
        let ty1 = (ty0 + 1).min(ty - 1);
        let wy = (fy - ty0 as f32).clamp(0.0, 1.0);
        for x in 0..w {
            let fx = (x as f32 + 0.5) * tx as f32 / w as f32 - 0.5;
            let tx0 = fx.floor().max(0.0) as usize;
            let tx1 = (tx0 + 1).min(tx - 1);
            let wx = (fx - tx0 as f32).clamp(0.0, 1.0);
            let g = gray[y * w + x] as usize;
            let m00 = maps[ty0 * tx + tx0][g] as f32;
            let m01 = maps[ty0 * tx + tx1][g] as f32;
            let m10 = maps[ty1 * tx + tx0][g] as f32;
            let m11 = maps[ty1 * tx + tx1][g] as f32;
            let m0 = m00 * (1.0 - wx) + m01 * wx;
            let m1 = m10 * (1.0 - wx) + m11 * wx;
            let target = m0 * (1.0 - wy) + m1 * wy;
            let src = (g as f32).max(1.0);
            let gain = target / src;
            for c in 0..3 {
                let i = (y * w + x) * 3 + c;
                out[i] = ((bgr[i] as f32) * gain).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// 判断 ROI 是否接近灰度图（逐像素 B/G/R 通道平均离散度很小）——用于自动启用 CLAHE。
pub fn looks_grayscale(bgr: &[u8], w: u32, h: u32) -> bool {
    let count = (w as usize * h as usize).min(bgr.len() / 3);
    if count == 0 {
        return false;
    }
    let mut spread_sum = 0u64;
    for i in 0..count {
        let b = bgr[i * 3] as i32;
        let g = bgr[i * 3 + 1] as i32;
        let r = bgr[i * 3 + 2] as i32;
        spread_sum += ((b - g).abs() + (g - r).abs() + (b - r).abs()) as u64;
    }
    // 平均每像素通道差 ≤ 12 视为灰度/夜视红外图
    (spread_sum / (count as u64 * 3)) <= 12
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray_frame(w: u32, h: u32, lo: u8, hi: u8) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 3) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                // 左暗右亮
                let t = if x < w as usize / 2 { lo } else { hi };
                let i = (y * w as usize + x) * 3;
                v[i] = t;
                v[i + 1] = t;
                v[i + 2] = t;
            }
        }
        v
    }

    #[test]
    fn output_shape_and_range_valid() {
        let (w, h) = (64u32, 48u32);
        let src = gray_frame(w, h, 60, 160);
        let out = clahe_bgr(&src, w, h, ClaheConfig::default());
        assert_eq!(out.len(), src.len());
        // 全像素在合法范围（u8 天然满足，检查不 panic 且有变化）
        assert!(out.iter().any(|&v| v != src[0]));
    }

    #[test]
    fn low_contrast_gets_expanded() {
        let (w, h) = (64u32, 64u32);
        let src = gray_frame(w, h, 100, 110); // 对比度极低
        let out = clahe_bgr(&src, w, h, ClaheConfig::default());
        let max_in = 110u8;
        let min_in = 100u8;
        let max_out = out.iter().copied().max().unwrap();
        let min_out = out.iter().copied().min().unwrap();
        assert!(
            (max_out as i32 - min_out as i32) > (max_in as i32 - min_in as i32),
            "CLAHE 应扩大对比度: in {}..{}, out {}..{}",
            min_in,
            max_in,
            min_out,
            max_out
        );
    }

    #[test]
    fn degenerate_input_passthrough() {
        let empty: Vec<u8> = Vec::new();
        assert_eq!(clahe_bgr(&empty, 0, 0, ClaheConfig::default()), empty);
    }

    #[test]
    fn grayscale_detection() {
        let g = gray_frame(16, 16, 80, 80);
        assert!(looks_grayscale(&g, 16, 16));
        let mut c = vec![0u8; 16 * 16 * 3];
        for i in 0..(16 * 16) {
            c[i * 3] = 10;
            c[i * 3 + 1] = 80;
            c[i * 3 + 2] = 200;
        }
        assert!(!looks_grayscale(&c, 16, 16));
    }
}
