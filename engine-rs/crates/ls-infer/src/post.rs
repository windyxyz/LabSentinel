//! YOLO 后处理纯 Rust：letterbox 反变换 + 置信度过滤 + NMS——与 Python
//! `engine.py::_letterbox/_decode_boxes/_nms` 逐位对齐（T-325，向量对齐先行，
//! ort 推理层 feature=ort 留接口）。

/// 检测框（原图坐标 xyxy）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Det {
    pub box_: [f32; 4],
    pub score: f32,
}

/// letterbox 几何（与 Python `_letterbox` 返回一致：缩放比 + x/y padding）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LetterboxGeom {
    pub r: f32,
    pub dx: f32,
    pub dy: f32,
}

/// letterbox 到 s×s 灰边画布的几何参数（对齐 Python `_letterbox`：
/// r=min(s/h,s/w)，nw=w*r，nh=h*r，dx=left=(s-nw)//2，dy=top=(s-nh)//2）。
pub fn letterbox_geom(w: u32, h: u32, s: u32) -> LetterboxGeom {
    let (w, h, s) = (w as f32, h as f32, s as f32);
    let r = (s / h).min(s / w);
    let nw = (w * r).floor();
    let nh = (h * r).floor();
    LetterboxGeom { r, dx: ((s - nw) / 2.0).floor(), dy: ((s - nh) / 2.0).floor() }
}

/// YOLO 输出行 (cx, cy, w, h, cls...) → 原图 xyxy（反 letterbox）。
pub fn decode_row(cx: f32, cy: f32, bw: f32, bh: f32, g: &LetterboxGeom) -> [f32; 4] {
    [
        (cx - bw / 2.0 - g.dx) / g.r,
        (cy - bh / 2.0 - g.dy) / g.r,
        (cx + bw / 2.0 - g.dx) / g.r,
        (cy + bh / 2.0 - g.dy) / g.r,
    ]
}

/// 类别最优分（多列取 max）+ 阈值筛选；与 Python `detect_class` 的 best/m 语义一致。
pub fn pick_class(rows: &[Vec<f32>], class_cols: &[usize], conf: f32,
                  g: &LetterboxGeom) -> Vec<Det> {
    let mut out = Vec::new();
    for row in rows {
        let best = class_cols.iter().filter_map(|&i| row.get(4 + i).copied())
            .fold(0.0f32, f32::max);
        if best > conf {
            out.push(Det { box_: decode_row(row[0], row[1], row[2], row[3], g), score: best });
        }
    }
    out
}

/// NMS（与 Python `_nms` 同序：分高优先、IoU<=thres 保留）。
pub fn nms(mut dets: Vec<Det>, iou_thres: f32) -> Vec<Det> {
    dets.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut keep: Vec<Det> = Vec::new();
    for d in dets {
        let suppressed = keep.iter().any(|k| iou(&k.box_, &d.box_) > iou_thres);
        if !suppressed {
            keep.push(d);
        }
    }
    keep
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let area_a = (a[2] - a[0]) * (a[3] - a[1]);
    let area_b = (b[2] - b[0]) * (b[3] - b[1]);
    let ix = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let iy = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = ix * iy;
    let denom = area_a + area_b - inter;
    if denom <= 0.0 { 0.0 } else { inter / denom }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_matches_python() {
        // 1280x720 → 640 画布：r=0.5，nw=640(dx=0)，nh=360(dy=140)（Python top=(640-360)//2）
        let g = letterbox_geom(1280, 720, 640);
        assert!((g.r - 0.5).abs() < 1e-6);
        assert!((g.dx - 0.0).abs() < 1e-6);
        assert!((g.dy - 140.0).abs() < 1e-6);
    }

    #[test]
    fn decode_reverses_letterbox() {
        let g = letterbox_geom(1280, 720, 640);
        // 画布中心 (320,320) 尺寸 20：x1=(320-10-0)/0.5=620, y1=(320-10-140)/0.5=340
        let b = decode_row(320.0, 320.0, 20.0, 20.0, &g);
        assert!((b[0] - 620.0).abs() < 1e-4, "x1={}", b[0]);
        assert!((b[1] - 340.0).abs() < 1e-4, "y1={}", b[1]);
        assert!((b[2] - 660.0).abs() < 1e-4, "x2={}", b[2]);
        assert!((b[3] - 380.0).abs() < 1e-4, "y2={}", b[3]);
    }

    #[test]
    fn nms_suppresses_overlap_keeps_distinct() {
        let dets = vec![
            Det { box_: [0.0, 0.0, 100.0, 100.0], score: 0.9 },
            Det { box_: [5.0, 5.0, 105.0, 105.0], score: 0.8 },  // 高重叠 → 抑制
            Det { box_: [300.0, 300.0, 400.0, 400.0], score: 0.7 }, // 独立 → 保留
        ];
        let kept = nms(dets, 0.5);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].score, 0.9);
    }
}
