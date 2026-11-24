use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PpeItem {
    Coat,
    Mask,
    Gloves,
    Goggles,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PpePolicy {
    #[serde(default)]
    pub zone_id: Option<i64>,
    /// Any one group suffices. All equipment in that group must be worn.
    pub alternatives: Vec<Vec<PpeItem>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    HoodLean,
    SashLimit,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ReferenceLine {
    pub name: String,
    pub kind: ReferenceKind,
    #[serde(default)]
    pub zone_id: Option<i64>,
    /// Normalized endpoints. The right side looking from start toward end is unsafe.
    pub points: [[f32; 2]; 2],
}

impl ReferenceLine {
    /// Positive distance is on the marked side. Ignore targets beyond the reference span.
    pub fn signed_distance(&self, point: [f32; 2], frame: [u32; 2]) -> Option<f32> {
        let to_pixels = |[x, y]: [f32; 2]| [x * frame[0] as f32, y * frame[1] as f32];
        let [start, end] = self.points.map(to_pixels);
        let dx = end[0] - start[0];
        let dy = end[1] - start[1];
        let px = point[0] - start[0];
        let py = point[1] - start[1];
        let square = dx * dx + dy * dy;
        let projection = (px * dx + py * dy) / square;
        (square > 0.0 && (0.0..=1.0).contains(&projection))
            .then(|| (dx * py - dy * px) / square.sqrt())
    }

    pub fn lean_ratio(&self, bounds: [f32; 4], frame: [u32; 2]) -> Option<f32> {
        let height = bounds[3] - bounds[1];
        if self.kind != ReferenceKind::HoodLean || !height.is_finite() || height <= 0.0 {
            return None;
        }
        let head = [(bounds[0] + bounds[2]) * 0.5, bounds[1] + height * 0.15];
        self.signed_distance(head, frame)
            .map(|distance| distance / height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixedCheckKind {
    IncubatorDoorOpen,
    SpillOrLeak,
    GloveContamination,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FixedViewCheck {
    pub kind: FixedCheckKind,
    pub roi: [f32; 4],
    pub hold_s: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LabRules {
    pub reference_lines: Vec<ReferenceLine>,
    pub ppe_policies: Vec<PpePolicy>,
    pub fixed_checks: Vec<FixedViewCheck>,
}

impl LabRules {
    pub fn validate(&self) -> Result<(), &'static str> {
        let normalized = |number: &f32| (0.0..=1.0).contains(number);
        let valid_zone = |id: Option<i64>| id.is_none_or(|id| id > 0);
        let mut names = HashSet::new();
        for line in &self.reference_lines {
            if line.name.trim().is_empty() || !names.insert(line.name.trim()) {
                return Err("每条基准线需要不同的名称");
            }
            if !line.points.iter().flatten().all(normalized) || line.points[0] == line.points[1] {
                return Err("基准线的两个端点必须位于画面内且不同");
            }
            if !valid_zone(line.zone_id) {
                return Err("基准线的区域无效");
            }
        }
        let mut policy_zones = HashSet::new();
        for policy in &self.ppe_policies {
            if !valid_zone(policy.zone_id) || !policy_zones.insert(policy.zone_id) {
                return Err("每个区域只允许一份有效的穿戴策略");
            }
            if policy.alternatives.is_empty() || policy.alternatives.iter().any(Vec::is_empty) {
                return Err("每份穿戴策略至少需要一组穿戴要求");
            }
        }
        let mut kinds = HashSet::new();
        for check in &self.fixed_checks {
            let [x1, y1, x2, y2] = check.roi;
            if !kinds.insert(check.kind)
                || !check.roi.iter().all(normalized)
                || x1 >= x2
                || y1 >= y2
            {
                return Err("每种固定机位检查需要一个不同且有效的画面区域");
            }
            if check.hold_s == 0 {
                return Err("固定机位检查的持续时间须大于零");
            }
        }
        Ok(())
    }
}

pub struct AlarmDescription {
    pub code: &'static str,
    pub stage: &'static str,
    pub suggestion: &'static str,
}

pub fn alarm_description(behavior: &str) -> Option<AlarmDescription> {
    let (code, stage, suggestion) = match behavior {
        "ppe_policy" => (
            "PPE_POLICY",
            "local_confirmed",
            "请按本区域要求补齐防护装备",
        ),
        "hood_lean" => ("HOOD_LEAN", "local_candidate", "请退回通风橱安全线外"),
        "sash_limit" => (
            "SASH_LIMIT",
            "local_candidate",
            "请核对拉门位置是否越过安全线",
        ),
        "incubator_door_open" => (
            "INCUBATOR_DOOR_OPEN",
            "vision_confirmed",
            "请检查培养箱门是否需要关闭",
        ),
        "spill_or_leak" => (
            "SPILL_OR_LEAK",
            "vision_confirmed",
            "请远离异常区域并通知值守人员",
        ),
        "glove_contamination" => (
            "GLOVE_CONTAMINATION",
            "vision_candidate",
            "请暂停当前动作并核对手套状态",
        ),
        _ => return None,
    };
    Some(AlarmDescription {
        code,
        stage,
        suggestion,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directed_line_distance_uses_actual_frame_aspect_and_span() {
        let line = ReferenceLine {
            name: "hood".into(),
            kind: ReferenceKind::HoodLean,
            zone_id: None,
            points: [[0.25, 0.4], [0.75, 0.4]],
        };
        assert_eq!(
            line.signed_distance([500.0, 230.0], [1000, 500]),
            Some(30.0)
        );
        assert_eq!(
            line.lean_ratio([400.0, 225.0, 600.0, 325.0], [1000, 500]),
            Some(0.4)
        );
        assert_eq!(line.signed_distance([200.0, 230.0], [1000, 500]), None);
    }

    #[test]
    fn ppe_policy_cannot_succeed_from_an_empty_alternative() {
        let settings = LabRules {
            ppe_policies: vec![PpePolicy {
                zone_id: None,
                alternatives: vec![vec![]],
            }],
            ..Default::default()
        };
        assert!(settings.validate().is_err());
    }
}
