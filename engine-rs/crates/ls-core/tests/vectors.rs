//! T-323：读取与 Python 共享的测试向量（`testdata/vectors/*.json`），
//! 保证 Rust 与 Python 领域行为逐用例一致（L0 对齐基准）。

use ls_core::state::{Hysteresis, ZoneState};
use ls_core::{point_in_polygon, ZoneChange};
use serde::Deserialize;
use std::path::PathBuf;

fn vectors_dir() -> PathBuf {
    // crate 在 engine-rs/crates/ls-core，向量在仓库根 testdata/vectors
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("testdata/vectors")
}

#[derive(Deserialize)]
struct ZoneStateVector {
    #[allow(dead_code)]
    comment: String,
    cases: Vec<ZoneStateCase>,
}

#[derive(Deserialize)]
struct ZoneStateCase {
    #[allow(dead_code)]
    name: String,
    enter_frames: u32,
    exit_frames: u32,
    observations: Vec<(i64, bool)>,
    expected: Vec<Option<String>>,
}

#[test]
fn zone_state_hysteresis_matches_python_shared_vectors() {
    let path = vectors_dir().join("zone_state_hysteresis.json");
    let v: ZoneStateVector =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("读取共享向量失败"))
            .expect("向量 JSON 解析失败");
    assert!(!v.cases.is_empty());
    for case in &v.cases {
        let mut st = ZoneState::new(Hysteresis {
            enter_frames: case.enter_frames,
            exit_frames: case.exit_frames,
        });
        let got: Vec<Option<ZoneChange>> = case
            .observations
            .iter()
            .map(|&(tid, inside)| st.update(tid, inside))
            .collect();
        let want: Vec<Option<ZoneChange>> = case
            .expected
            .iter()
            .map(|e| match e.as_deref() {
                None => None,
                Some("enter") => Some(ZoneChange::Enter),
                Some("exit") => Some(ZoneChange::Exit),
                other => panic!("未知期望值: {other:?}"),
            })
            .collect();
        assert_eq!(got, want, "向量用例 {} 不一致", case.name);
    }
}

#[derive(Deserialize)]
struct PolygonVector {
    polygons: std::collections::HashMap<String, Vec<(i64, i64)>>,
    cases: Vec<PolygonCase>,
}

#[derive(Deserialize)]
struct PolygonCase {
    poly: String,
    point: (i64, i64),
    inside: bool,
}

#[test]
fn point_in_polygon_matches_opencv_shared_vectors() {
    let path = vectors_dir().join("point_in_polygon.json");
    let v: PolygonVector =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("读取共享向量失败"))
            .expect("向量 JSON 解析失败");
    assert!(!v.cases.is_empty());
    for case in &v.cases {
        let poly = v
            .polygons
            .get(&case.poly)
            .unwrap_or_else(|| panic!("未知多边形 {}", case.poly));
        assert_eq!(
            point_in_polygon(poly, case.point),
            case.inside,
            "多边形 {} 点 {:?} 判定不一致",
            case.poly,
            case.point
        );
    }
}
