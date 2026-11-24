use std::time::Duration;

use ls_core::lab::{PpeItem, PpePolicy, ReferenceKind, ReferenceLine};

use crate::pipeline::{PpeObservation, PpeStatus};

pub fn evaluate_ppe_policy(rule: &PpePolicy, observed: PpeObservation) -> (PpeStatus, f32) {
    let get = |item| match item {
        PpeItem::Coat => (observed.coat, observed.coat_violation_confidence),
        PpeItem::Mask => (observed.mask, observed.mask_violation_confidence),
        PpeItem::Gloves => (observed.gloves, observed.glove_violation_confidence),
        PpeItem::Goggles => (observed.goggles, observed.goggle_violation_confidence),
    };
    let mut blocked = 0;
    let mut confidence = 1.0_f32;
    for group in &rule.alternatives {
        if !group.is_empty() && group.iter().all(|&item| get(item).0 == PpeStatus::Wearing) {
            return (PpeStatus::Wearing, 0.0);
        }
        if let Some(score) = group
            .iter()
            .map(|&item| get(item))
            .filter_map(|(status, score)| (status == PpeStatus::NoPpe).then_some(score))
            .max_by(f32::total_cmp)
        {
            blocked += 1;
            confidence = confidence.min(score);
        }
    }
    if blocked > 0 && blocked == rule.alternatives.len() {
        (PpeStatus::NoPpe, confidence)
    } else {
        (PpeStatus::Unknown, 0.0)
    }
}

/// Estimate the sash edge near a calibrated horizontal reference. A credible row
/// must have a coherent contrast change across the span and stand out from other rows.
pub fn sash_clearance(line: &ReferenceLine, frame: &ls_ingest::Frame) -> Option<f32> {
    let [width, height] = [frame.width as usize, frame.height as usize];
    let [a, b] = line.points;
    if line.kind != ReferenceKind::SashLimit
        || width < 24
        || height < 8
        || frame.data.len() != width.checked_mul(height)?.checked_mul(3)?
        || (a[1] - b[1]).abs() > 0.03
    {
        return None;
    }
    let horizontal = |x: f32| ((x * width as f32) as usize).min(width - 1);
    let [left, right] = [horizontal(a[0].min(b[0])), horizontal(a[0].max(b[0]))];
    if right - left < 20 {
        return None;
    }
    let reference_y = ((a[1] + b[1]) * 0.5 * height as f32) as usize;
    let search_depth = (height / 7).max(4);
    let begin = reference_y.saturating_sub(search_depth).max(1);
    let end = (reference_y + search_depth).min(height - 2);
    let luminance = |x, y: usize| {
        let index = (y * width + x) * 3;
        let bgr = &frame.data[index..index + 3];
        (u16::from(bgr[0]) * 29 + u16::from(bgr[1]) * 150 + u16::from(bgr[2]) * 77) / 256
    };
    let mut rows = Vec::new();
    for y in begin..=end {
        let mut counts = [0_u32; 2];
        let mut score = 0_u32;
        for column in 0..16 {
            let x = left + (right - left) * (column + 1) / 17;
            let [above, below] = [luminance(x, y - 1), luminance(x, y + 1)];
            let difference = above.abs_diff(below);
            if difference >= 28 {
                counts[usize::from(below > above)] += 1;
                score += u32::from(difference);
            }
        }
        if counts.into_iter().max()? >= 12 {
            rows.push((score, y));
        }
    }
    let &(score, y) = rows.iter().max_by_key(|row| row.0)?;
    if rows
        .iter()
        .any(|&(other_score, other_y)| other_y.abs_diff(y) > 4 && other_score * 5 >= score * 4)
    {
        return None;
    }
    line.signed_distance(
        [(left + right) as f32 * 0.5, y as f32],
        [frame.width, frame.height],
    )
}

#[derive(Default)]
pub struct ComplianceHold {
    seen: Option<(Duration, Duration, u8)>,
}

impl ComplianceHold {
    pub fn in_progress(&self) -> bool {
        self.seen.is_some()
    }

    /// A confirmed recovery needs a two-second span and three distinct observations.
    pub fn observe(&mut self, now: Duration, status: PpeStatus) -> bool {
        if status != PpeStatus::Wearing {
            self.seen = None;
            return false;
        }
        let (first, previous, hits) = self
            .seen
            .filter(|(_, previous, _)| {
                now >= *previous && now - *previous <= Duration::from_secs(3)
            })
            .unwrap_or((now, now, 1));
        let hits = hits.saturating_add(u8::from(now > previous));
        self.seen = Some((first, now, hits));
        hits >= 3 && now - first >= Duration::from_secs(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alternative_needs_positive_evidence_and_unknown_is_not_a_violation() {
        let rule = PpePolicy {
            zone_id: None,
            alternatives: vec![
                vec![PpeItem::Coat, PpeItem::Mask],
                vec![PpeItem::Gloves, PpeItem::Goggles],
            ],
        };
        let mut evidence = PpeObservation {
            coat: PpeStatus::Wearing,
            mask: PpeStatus::NoPpe,
            mask_violation_confidence: 0.9,
            ..Default::default()
        };
        assert_eq!(evaluate_ppe_policy(&rule, evidence).0, PpeStatus::Unknown);
        evidence.goggles = PpeStatus::Wearing;
        evidence.gloves = PpeStatus::Wearing;
        assert_eq!(evaluate_ppe_policy(&rule, evidence).0, PpeStatus::Wearing);
        evidence.gloves = PpeStatus::NoPpe;
        evidence.glove_violation_confidence = 0.7;
        assert_eq!(
            evaluate_ppe_policy(&rule, evidence),
            (PpeStatus::NoPpe, 0.7)
        );
    }

    #[test]
    fn recovery_requires_distinct_frames_and_resets_when_visibility_is_lost() {
        let mut hold = ComplianceHold::default();
        let t = Duration::from_secs;
        assert!(!hold.observe(t(0), PpeStatus::Wearing));
        assert!(!hold.observe(t(0), PpeStatus::Wearing));
        assert!(!hold.observe(t(1), PpeStatus::Wearing));
        assert!(hold.observe(t(2), PpeStatus::Wearing));
        assert!(!hold.observe(t(3), PpeStatus::Unknown));
        assert!(!hold.observe(t(4), PpeStatus::Wearing));
    }

    #[test]
    fn sash_clearance_ignores_a_view_with_competing_edges() {
        let mut line = ReferenceLine {
            name: "Fixture".into(),
            kind: ReferenceKind::SashLimit,
            zone_id: None,
            points: [[0.8, 0.5], [0.2, 0.5]],
        };
        let mut frame = ls_ingest::Frame {
            index: 0,
            width: 120,
            height: 100,
            data: vec![40; 120 * 100 * 3],
        };
        frame.data[42 * 120 * 3..].fill(190);
        assert!(sash_clearance(&line, &frame).is_some_and(|distance| distance > 4.0));
        line.points.swap(0, 1);
        assert!(sash_clearance(&line, &frame).is_some_and(|distance| distance < -4.0));
        frame.data[59 * 120 * 3..].fill(40);
        assert_eq!(sash_clearance(&line, &frame), None);
    }
}
