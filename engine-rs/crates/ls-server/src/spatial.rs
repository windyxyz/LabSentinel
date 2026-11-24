use ls_analyzer::pipeline::LiveTrack;
use ls_store::{CameraMask, FalsePositiveSpot, MaskKind, Store};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub(crate) struct SpatialPolicy {
    pub masks: Vec<CameraMask>,
    pub spots: Vec<FalsePositiveSpot>,
}

impl SpatialPolicy {
    pub fn read(store: &Store, cid: i64) -> rusqlite::Result<Self> {
        Ok(Self {
            masks: store.camera_masks(cid)?,
            spots: store.fp_spots(cid)?,
        })
    }

    pub fn apply_motion_masks(&self, gate: &mut ls_analyzer::motion::MotionGate) {
        gate.set_masks(
            self.masks
                .iter()
                .filter(|m| m.kind == MaskKind::Motion)
                .map(|m| m.polygon.as_slice()),
        );
    }

    fn object_mask(&self, bbox: [f32; 4]) -> Option<i64> {
        let [left, top, right, bottom] = bbox;
        let center = (((left + right) * 0.5) as i64, ((top + bottom) * 0.5) as i64);
        self.masks
            .iter()
            .find(|mask| {
                mask.kind == MaskKind::ObjectFilter
                    && ls_core::point_in_polygon(&mask.polygon, center)
            })
            .map(|mask| mask.id)
    }

    #[cfg(test)]
    pub fn suppress(
        &self,
        store: &Store,
        cid: i64,
        behavior: &str,
        bbox: [f32; 4],
        ts: &str,
    ) -> bool {
        self.suppress_with_evidence(store, cid, behavior, bbox, ts, None)
    }

    pub fn suppress_with_evidence(
        &self,
        store: &Store,
        cid: i64,
        behavior: &str,
        bbox: [f32; 4],
        ts: &str,
        frame: Option<&AuditFrame<'_>>,
    ) -> bool {
        let Some((reason, source)) = self.match_suppression(behavior, bbox) else {
            return false;
        };
        audit(store, cid, reason, source, behavior, ts, frame)
    }

    fn match_suppression(&self, behavior: &str, bbox: [f32; 4]) -> Option<(&'static str, i64)> {
        if !spatial_rule(behavior) {
            return None;
        }
        if let Some(id) = self.object_mask(bbox) {
            return Some(("filter_mask", id));
        }
        self.spots
            .iter()
            .find(|spot| spot.behavior == behavior && overlap(spot.bbox, bbox) >= 0.75)
            .map(|spot| ("remembered_spot", spot.id))
    }
}

pub(crate) fn spatial_rule(behavior: &str) -> bool {
    matches!(
        behavior,
        "enter" | "enter_no_ppe" | "no_mask" | "ppe_policy" | "hood_lean"
    )
}

pub(crate) struct AuditFrame<'a> {
    pub data_dir: &'a str,
    pub jpeg: Option<&'a [u8]>,
    pub pixels: Option<(&'a [u8], u32, u32)>,
}

pub(crate) fn audit(
    store: &Store,
    cid: i64,
    reason: &str,
    source: i64,
    behavior: &str,
    ts: &str,
    frame: Option<&AuditFrame<'_>>,
) -> bool {
    match store.add_suppression(cid, reason, source, behavior, ts) {
        Ok((id, new)) => {
            if let Some(frame) =
                frame.filter(|_| new && store.should_save_suppression_snapshot(id).unwrap_or(false))
            {
                save_audit_frame(store, id, frame);
            }
            true
        }
        Err(error) => {
            crate::logger::error(&format!("[ls-worker] cam-{cid} 抑制留痕失败: {error}"));
            false
        }
    }
}

fn save_audit_frame(store: &Store, id: i64, frame: &AuditFrame<'_>) {
    let jpeg = frame
        .jpeg
        .filter(|bytes| bytes.starts_with(&[0xff, 0xd8]))
        .map(std::borrow::Cow::Borrowed)
        .or_else(|| {
            let (pixels, width, height) = frame.pixels?;
            ls_analyzer::evidence::encode_jpeg_bgr(pixels, width, height)
                .ok()
                .map(std::borrow::Cow::Owned)
        });
    let Some(jpeg) = jpeg else { return };
    let root = std::path::Path::new(frame.data_dir);
    let relative = format!(
        "snapshots/suppression_{id}_{}.jpg",
        crate::util::uuid_like()
    );
    let path = root.join(&relative);
    if std::fs::create_dir_all(root.join("snapshots")).is_err() {
        return;
    }
    if std::fs::write(&path, &jpeg).is_err()
        || !store
            .attach_suppression_snapshot(id, &relative)
            .unwrap_or(false)
    {
        let _ = std::fs::remove_file(path);
    }
}
/// Only masked, repeatedly observed objects can relax the active-track motion guard.
#[derive(Default)]
pub(crate) struct StationaryTracks(HashMap<i64, ([f32; 4], u8)>);

impl StationaryTracks {
    pub fn update(&mut self, policy: &SpatialPolicy, tracks: &[LiveTrack]) -> bool {
        const OBSERVATIONS: u8 = 12;
        self.0
            .retain(|id, _| tracks.iter().any(|track| track.id == *id));
        let mut all_stationary = !tracks.is_empty();
        for track in tracks {
            if track.occluded || policy.object_mask(track.bbox).is_none() {
                self.0.remove(&track.id);
                all_stationary = false;
                continue;
            }
            let (anchor, frames) = self.0.entry(track.id).or_insert((track.bbox, 0));
            if overlap(*anchor, track.bbox) < 0.9 {
                *anchor = track.bbox;
                *frames = 0;
            }
            *frames = frames.saturating_add(1);
            all_stationary &= *frames >= OBSERVATIONS;
        }
        all_stationary
    }
}

fn overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    let width = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let height = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let intersection = width * height;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - intersection;
    if union > 0.0 {
        intersection / union
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_filters_never_apply_to_life_safety_rules() {
        let policy = SpatialPolicy {
            masks: vec![CameraMask {
                id: 3,
                camera_id: 1,
                name: "玻璃".into(),
                kind: MaskKind::ObjectFilter,
                polygon: vec![(0, 0), (90, 0), (90, 90), (0, 90)],
            }],
            spots: vec![FalsePositiveSpot {
                id: 4,
                event_id: 6,
                behavior: "no_mask".into(),
                bbox: [95.0, 20.0, 120.0, 80.0],
                created_at: String::new(),
            }],
        };
        let bbox = [20.0, 20.0, 60.0, 80.0];
        assert_eq!(
            policy.match_suppression("enter_no_ppe", bbox),
            Some(("filter_mask", 3))
        );
        assert_eq!(policy.match_suppression("person_fall", bbox), None);
        assert_eq!(policy.match_suppression("thermal_overheat", bbox), None);
        let prior = [96.0, 20.0, 121.0, 80.0];
        assert_eq!(
            policy.match_suppression("no_mask", prior),
            Some(("remembered_spot", 4))
        );
        assert_eq!(policy.match_suppression("enter", prior), None);
        let mut static_tracks = StationaryTracks::default();
        let mut track = LiveTrack {
            id: 1,
            bbox,
            ppe: ls_analyzer::pipeline::PpeStatus::Unknown,
            confidence: 0.9,
            occluded: false,
        };
        for _ in 0..11 {
            assert!(!static_tracks.update(&policy, std::slice::from_ref(&track)));
        }
        assert!(static_tracks.update(&policy, std::slice::from_ref(&track)));
        track.bbox = [35.0, 20.0, 75.0, 80.0];
        assert!(!static_tracks.update(&policy, &[track]));
    }
}
