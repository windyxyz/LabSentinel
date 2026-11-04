//! 区域多边形判定：射线法 + 边界算内，与 `cv2.pointPolygonTest(poly, p, False) >= 0`
//! 的语义对齐（T-323；向量见 `testdata/vectors/point_in_polygon.json`）。

use crate::Point;

/// 判断点是否在多边形内（含边界）。
///
/// 与 OpenCV 的约定一致：边界（顶点/边）算在内；顶点数 < 3 视为空区域（永假）。
pub fn point_in_polygon(poly: &[Point], p: Point) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        if on_segment(a, b, p) {
            return true;
        }
        if (a.1 > p.1) != (b.1 > p.1) {
            // 交点 x 用 i64 中间量计算（坐标 ≤ 2^31 时无溢出），与 cv2 的整数语义对齐
            let dx = b.0 - a.0;
            let dy = b.1 - a.1;
            let x_int = a.0 + (p.1 - a.1) * dx / dy;
            if p.0 < x_int {
                inside = !inside;
            }
        }
    }
    inside
}

/// 点是否在线段 a-b 上（含端点；线段平行于坐标轴或斜线均支持）。
fn on_segment(a: Point, b: Point, p: Point) -> bool {
    let cross = (p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0);
    if cross != 0 {
        return false;
    }
    p.0 >= a.0.min(b.0) && p.0 <= a.0.max(b.0) && p.1 >= a.1.min(b.1) && p.1 <= a.1.max(b.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: [Point; 4] = [(100, 50), (1100, 50), (1100, 650), (100, 650)];

    #[test]
    fn rect_interior_edge_vertex() {
        assert!(point_in_polygon(&RECT, (600, 300)));
        assert!(point_in_polygon(&RECT, (100, 50))); // 顶点算内
        assert!(point_in_polygon(&RECT, (1100, 300))); // 竖边算内
        assert!(point_in_polygon(&RECT, (600, 50))); // 横边算内
        assert!(!point_in_polygon(&RECT, (99, 300)));
        assert!(!point_in_polygon(&RECT, (600, 651)));
    }

    #[test]
    fn degenerate_polygon_is_empty() {
        assert!(!point_in_polygon(&[(0, 0), (10, 10)], (5, 5)));
        assert!(!point_in_polygon(&[], (0, 0)));
    }
}
