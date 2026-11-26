//! Seed an isolated event store and check exact, scoped nearest neighbors.
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ls_store::{vec::EMBEDDING_DIMS, NewEvent, Store};
use serde_json::json;

const COUNT: i64 = 10_000;
const SAMPLE_COUNT: usize = 35;
const NAMES: [&str; 2] = ["no_mask", "enter_no_ppe"];
const ENCODER: &str = "siglip2-base-patch16-224-int8";

fn embedding(id: i64) -> Vec<f32> {
    let mut seed = id as u32;
    let mut vector = vec![0.0; EMBEDDING_DIMS];
    for item in &mut vector[1..] {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *item = (seed >> 16) as f32 - 32_768.0;
    }
    let score = id as f32 / (COUNT + 1) as f32;
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    let scale = (1.0 - score * score).sqrt() / norm;
    for item in &mut vector[1..] {
        *item *= scale;
    }
    vector[0] = score;
    vector
}

fn camera(id: i64) -> i64 {
    id % 3 + 1
}

fn kind(id: i64) -> &'static str {
    NAMES[(id % 2) as usize]
}

fn seed(path: &Path) -> Result<Store, Box<dyn std::error::Error>> {
    File::options().write(true).create_new(true).open(path)?;
    let store = Store::open(path.to_str().ok_or("database path must be UTF-8")?)?;
    store.ensure_schema()?;
    store.migrate()?;
    for id in 1..=3 {
        assert_eq!(
            store.conn_add_camera(&format!("fixture {id}"), "0", "usb")?,
            id
        );
    }
    for id in 1..=COUNT {
        let added = store.add_event(&NewEvent {
            camera_id: camera(id),
            type_: kind(id),
            level: 2,
            track_id: id,
            detail: "synthetic vector acceptance",
            snapshot: Some("snapshots/reference.jpg"),
            ts: "2026-11-26T12:00:00",
        })?;
        assert_eq!(added, id);
        let event = store
            .event_by_id(added)?
            .ok_or("event missing after insert")?;
        assert!(store.index_event_embedding(&event, ENCODER, "synthetic", &embedding(id))?);
    }
    assert_eq!(store.integrity_check()?, "ok");
    Ok(store)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mode = args
        .next()
        .ok_or("use seed or check followed by a database path")?;
    let path = PathBuf::from(args.next().ok_or("provide a database path")?);
    if mode == "seed" {
        drop(seed(&path)?);
    } else if mode != "check" || !path.is_file() {
        return Err("use seed for a new database or check for an existing fixture".into());
    }
    let store = Store::open(path.to_str().ok_or("database path must be UTF-8")?)?;
    let mut vector = vec![0.0; EMBEDDING_DIMS];
    vector[0] = 1.0;
    let mut report = serde_json::Map::new();
    for (name, scope, skip, types) in [
        ("all", None, None, None),
        ("one_camera", Some([2].as_slice()), Some(10_000), None),
        (
            "two_cameras_one_kind",
            Some([2, 3].as_slice()),
            Some(10_000),
            Some(["no_mask"].as_slice()),
        ),
    ] {
        let expect: Vec<_> = (1..=COUNT)
            .rev()
            .filter(|&id| {
                scope.is_none_or(|list| list.contains(&camera(id)))
                    && skip != Some(id)
                    && types.is_none_or(|list| list.contains(&kind(id)))
            })
            .take(30)
            .collect();
        let mut millis = Vec::with_capacity(SAMPLE_COUNT);
        for run in 0..SAMPLE_COUNT + 5 {
            let started = Instant::now();
            let ranked =
                store.rank_event_embeddings_scoped(ENCODER, &vector, scope, skip, 30, types)?;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(ranked.iter().map(|(id, _)| *id).collect::<Vec<_>>(), expect);
            if run >= 5 {
                millis.push(elapsed);
            }
        }
        millis.sort_by(f64::total_cmp);
        report.insert(
            name.into(),
            json!({
                "p50_ms": millis[SAMPLE_COUNT / 2],
                "p95_ms": millis[(SAMPLE_COUNT * 95).div_ceil(100) - 1],
                "max_ms": millis[SAMPLE_COUNT - 1],
                "query_count": SAMPLE_COUNT,
            }),
        );
    }
    println!(
        "{}",
        json!({"vectors": COUNT, "dimensions": EMBEDDING_DIMS, "results": report})
    );
    Ok(())
}
