//! 区域多边形判定：射线法 + 边界算内，与 `cv2.pointPolygonTest(poly, p, False) >= 0`
//! 的语义对齐（T-323；向量见 `testdata/vectors/point_in_polygon.json`）。

use crate::Point;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorMode {
    #[default]
    BottomCenter,
    BodyCenter,
    UpperBody,
    FourPointAny,
}

impl AnchorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BottomCenter => "bottom_center",
            Self::BodyCenter => "body_center",
            Self::UpperBody => "upper_body",
            Self::FourPointAny => "four_point_any",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "bottom_center" => Some(Self::BottomCenter),
            "body_center" => Some(Self::BodyCenter),
            "upper_body" => Some(Self::UpperBody),
            "four_point_any" => Some(Self::FourPointAny),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ZoneSpatial {
    pub anchor_mode: AnchorMode,
    pub margin_px: u32,
    pub leave_grace_frames: u32,
}

impl ZoneSpatial {
    pub const NEW_ZONE: Self = Self {
        anchor_mode: AnchorMode::BottomCenter,
        margin_px: 0,
        leave_grace_frames: 12,
    };

    pub fn hysteresis(self, mut default: crate::state::Hysteresis) -> crate::state::Hysteresis {
        if self.leave_grace_frames > 0 {
            default.exit_frames = self.leave_grace_frames;
        }
        default
    }

    /// None means the anchor is close enough to an edge to keep the current state.
    pub fn observe(self, polygon: &[Point], bbox: [f32; 4]) -> Option<bool> {
        if polygon.len() < 3 || !bbox.iter().all(|v| v.is_finite()) {
            return Some(false);
        }
        let [left, top, right, bottom] = bbox;
        let x = (left + right) * 0.5;
        let mid_y = (top + bottom) * 0.5;
        let upper_y = top + (bottom - top) * 0.25;
        let bottom_center = (x as i64, bottom as i64);
        let body_center = (x as i64, mid_y as i64);
        let upper_body = (x as i64, upper_y as i64);
        let four = [
            bottom_center,
            body_center,
            ((left + (right - left) * 0.25) as i64, upper_y as i64),
            ((left + (right - left) * 0.75) as i64, upper_y as i64),
        ];
        let anchors: &[Point] = match self.anchor_mode {
            AnchorMode::BottomCenter => std::slice::from_ref(&bottom_center),
            AnchorMode::BodyCenter => std::slice::from_ref(&body_center),
            AnchorMode::UpperBody => std::slice::from_ref(&upper_body),
            AnchorMode::FourPointAny => &four,
        };
        let mut uncertain = false;
        for &anchor in anchors {
            if self.margin_px > 0 && near_edge(polygon, anchor, f64::from(self.margin_px)) {
                uncertain = true;
            } else if point_in_polygon(polygon, anchor) {
                return Some(true);
            }
        }
        if uncertain {
            None
        } else {
            Some(false)
        }
    }
}

fn near_edge(polygon: &[Point], (x, y): Point, margin: f64) -> bool {
    let limit = margin * margin;
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .any(|(&(ax, ay), &(bx, by))| {
            let (dx, dy) = (bx as f64 - ax as f64, by as f64 - ay as f64);
            let (px, py) = (x as f64 - ax as f64, y as f64 - ay as f64);
            let length = dx * dx + dy * dy;
            let t = if length == 0.0 {
                0.0
            } else {
                ((px * dx + py * dy) / length).clamp(0.0, 1.0)
            };
            (px - dx * t).powi(2) + (py - dy * t).powi(2) <= limit
        })
}

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
            // LS-043：用 i128 交叉乘消除整数除法截断与大坐标溢出
            let dx = i128::from(b.0) - i128::from(a.0);
            let dy = i128::from(b.1) - i128::from(a.1);
            let order = compare_products(
                i128::from(p.0) - i128::from(a.0),
                dy,
                i128::from(p.1) - i128::from(a.1),
                dx,
            );
            let strictly_left = if dy > 0 { order.is_lt() } else { order.is_gt() };
            if strictly_left {
                inside = !inside;
            }
        }
    }
    inside
}

/// 点是否在线段 a-b 上（含端点；线段平行于坐标轴或斜线均支持）。
fn on_segment(a: Point, b: Point, p: Point) -> bool {
    if p.0 < a.0.min(b.0) || p.0 > a.0.max(b.0) || p.1 < a.1.min(b.1) || p.1 > a.1.max(b.1) {
        return false;
    }
    compare_products(
        i128::from(p.0) - i128::from(a.0),
        i128::from(b.1) - i128::from(a.1),
        i128::from(p.1) - i128::from(a.1),
        i128::from(b.0) - i128::from(a.0),
    )
    .is_eq()
}

fn compare_products(a: i128, b: i128, c: i128, d: i128) -> std::cmp::Ordering {
    if let (Some(lhs), Some(rhs)) = (a.checked_mul(b), c.checked_mul(d)) {
        return lhs.cmp(&rhs);
    }
    let lhs = a.unsigned_abs() * b.unsigned_abs();
    let rhs = c.unsigned_abs() * d.unsigned_abs();
    let lhs_negative = lhs != 0 && (a < 0) != (b < 0);
    let rhs_negative = rhs != 0 && (c < 0) != (d < 0);
    rhs_negative.cmp(&lhs_negative).then_with(|| {
        if lhs_negative {
            rhs.cmp(&lhs)
        } else {
            lhs.cmp(&rhs)
        }
    })
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

    #[test]
    fn spatial_anchors_and_deadband() {
        let bbox = [400.0, 200.0, 500.0, 651.0];
        let default = ZoneSpatial::default();
        assert_eq!(default.observe(&RECT, bbox), Some(false));
        let margin = ZoneSpatial {
            margin_px: 5,
            ..default
        };
        assert_eq!(margin.observe(&RECT, bbox), None);
        assert_eq!(
            margin.observe(&RECT, [400.0, 200.0, 500.0, 644.0]),
            Some(true)
        );
        for anchor_mode in [
            AnchorMode::BodyCenter,
            AnchorMode::UpperBody,
            AnchorMode::FourPointAny,
        ] {
            assert_eq!(
                ZoneSpatial {
                    anchor_mode,
                    ..margin
                }
                .observe(&RECT, bbox),
                Some(true)
            );
        }
    }

    #[test]
    fn spatial_leave_grace_needs_twelve_consecutive_observations() {
        let mut state = crate::state::ZoneState::new(ZoneSpatial::NEW_ZONE.hysteresis(
            crate::state::Hysteresis {
                enter_frames: 1,
                exit_frames: 3,
            },
        ));
        assert_eq!(state.update(1, true), Some(crate::ZoneChange::Enter));
        for _ in 0..11 {
            assert_eq!(state.update(1, false), None);
        }
        state.hold(1);
        assert!(state.is_inside(1));
        for _ in 0..11 {
            assert_eq!(state.update(1, false), None);
        }
        assert_eq!(state.update(1, false), Some(crate::ZoneChange::Exit));
    }
}
