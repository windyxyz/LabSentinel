//! LS-054：网页端便携标注工作台（数据飞轮）——样本入池 / AI 预标注 / 人工修订 /
//! 标准 YOLO 训练集 ZIP 导出。
//!
//! 存储：`dataset_samples`（v16）。图片统一拷入 `data/dataset_pool/`，
//! 标注框格式 `[{class, x1,y1,x2,y2}]`（像素坐标，导出时归一化）。
//! 导出：仅 `verified` 样本；按 8:2 划分 train/val，输出 images/ + labels/ +
//! data.yaml + classes.txt + README.txt。

use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn pool_dir(st: &Shared) -> PathBuf {
    std::path::Path::new(&st.data_dir).join("dataset_pool")
}

/// 拷贝源图到样本池，返回相对路径（相对 data_dir）。
fn copy_into_pool(st: &Shared, src: &Path, tag: &str) -> Result<(String, u32, u32), String> {
    let (bgr, w, h) = ls_ingest::decode_image(src, 1280, 720)
        .map_err(|e| format!("图片解码失败: {e}"))?;
    let dir = pool_dir(st);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建样本池目录失败: {e}"))?;
    let name = format!("{tag}.jpg");
    let dest = dir.join(&name);
    let jpeg = ls_analyzer::evidence::encode_jpeg_bgr(&bgr, w, h)
        .map_err(|e| format!("样本编码失败: {e}"))?;
    std::fs::write(&dest, &jpeg).map_err(|e| format!("写入样本失败: {e}"))?;
    let rel = format!("dataset_pool/{name}");
    Ok((rel, w, h))
}

/// AI 预标注：本地人员检测 → boxes_json。
fn preannotate(_st: &Shared, src: &Path) -> String {
    let cfg = ls_analyzer::detector::DetectorConfig::from_env(None);
    let mut det = ls_analyzer::detector::make_detector(&cfg);
    match ls_ingest::decode_image(src, 1280, 720) {
        Ok((bgr, w, h)) => {
            let frame = ls_ingest::Frame {
                width: w,
                height: h,
                data: bgr,
                index: 1,
            };
            let dets = det.detect(&frame);
            let boxes: Vec<Value> = dets
                .iter()
                .map(|d| {
                    json!({
                        "class": "person",
                        "x1": d.box_[0], "y1": d.box_[1],
                        "x2": d.box_[2], "y2": d.box_[3],
                        "conf": d.score,
                    })
                })
                .collect();
            Value::Array(boxes).to_string()
        }
        Err(_) => "[]".to_string(),
    }
}

fn sample_json(s: &ls_store::DatasetSample) -> Value {
    let boxes: Value = serde_json::from_str(&s.boxes_json).unwrap_or_else(|_| json!([]));
    json!({
        "id": s.id, "source_type": s.source_type, "source_id": s.source_id,
        "camera_id": s.camera_id, "image_path": s.image_path,
        "width": s.width, "height": s.height, "status": s.status,
        "boxes": boxes, "created_at": s.created_at, "updated_at": s.updated_at,
    })
}

pub(crate) async fn samples_from_event(
    State(st): State<Shared>,
    AxPath(eid): AxPath<i64>,
) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let Ok(Some(ev)) = store.event_by_id(eid) else {
        return detail_err(StatusCode::NOT_FOUND, "event not found").into_response();
    };
    let Some(rel) = ev.snapshot.filter(|s| !s.is_empty()) else {
        return detail_err(StatusCode::NOT_FOUND, "event has no snapshot").into_response();
    };
    let src = std::path::Path::new(&st.data_dir).join(rel);
    let tag = format!("event_{eid}");
    let (rel, w, h) = match copy_into_pool(&st, &src, &tag) {
        Ok(v) => v,
        Err(e) => return detail_err(StatusCode::BAD_REQUEST, &e).into_response(),
    };
    let boxes = preannotate(&st, &src);
    match store.add_dataset_sample("event", &eid.to_string(), Some(ev.camera_id), &rel, w as i64, h as i64, &boxes) {
        Ok(id) => Json(json!({"ok": true, "id": id, "boxes": serde_json::from_str::<Value>(&boxes).unwrap_or(json!([]))})).into_response(),
        Err(_) => server_err(),
    }
}

pub(crate) async fn samples_from_folder(
    State(st): State<Shared>,
    AxPath(iid): AxPath<i64>,
) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let Ok(Some(img)) = store.folder_image(iid) else {
        return detail_err(StatusCode::NOT_FOUND, "folder image not found").into_response();
    };
    let src = std::path::Path::new(&st.data_dir).join(&img.path);
    let tag = format!("folder_{iid}");
    let (rel, w, h) = match copy_into_pool(&st, &src, &tag) {
        Ok(v) => v,
        Err(e) => return detail_err(StatusCode::BAD_REQUEST, &e).into_response(),
    };
    let boxes = preannotate(&st, &src);
    match store.add_dataset_sample("folder", &iid.to_string(), Some(img.camera_id), &rel, w as i64, h as i64, &boxes) {
        Ok(id) => Json(json!({"ok": true, "id": id})).into_response(),
        Err(_) => server_err(),
    }
}

/// 实时抓拍入池：取运行中相机最新 JPEG。
pub(crate) async fn samples_capture(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
) -> Response {
    let Some(handle) = st.workers.get(cid) else {
        return detail_err(StatusCode::CONFLICT, "相机未运行").into_response();
    };
    let jpeg = handle
        .latest_jpeg
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default();
    if jpeg.is_empty() {
        return detail_err(StatusCode::CONFLICT, "暂无可用画面（等待抽帧）").into_response();
    }
    let dir = pool_dir(&st);
    if std::fs::create_dir_all(&dir).is_err() {
        return server_err();
    }
    let name = format!("capture_{cid}_{}.jpg", ls_core::time::local_now_compact());
    let dest = dir.join(&name);
    if std::fs::write(&dest, &jpeg).is_err() {
        return server_err();
    }
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let rel = format!("dataset_pool/{name}");
    let boxes = preannotate(&st, &dest);
    match store.add_dataset_sample("capture", &cid.to_string(), Some(cid), &rel, 1280, 720, &boxes) {
        Ok(id) => Json(json!({"ok": true, "id": id})).into_response(),
        Err(_) => server_err(),
    }
}

pub(crate) async fn samples_list(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let status = q.get("status").map(|s| s.as_str()).filter(|s| !s.is_empty());
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(200)
        .clamp(1, 2000);
    let rows = match store.dataset_samples(status, limit) {
        Ok(r) => r,
        Err(_) => return server_err(),
    };
    let counts = store.dataset_class_counts().unwrap_or_default();
    Json(json!({
        "samples": rows.iter().map(sample_json).collect::<Vec<_>>(),
        "total": rows.len(),
        "class_counts": counts.iter().map(|(k, v)| json!({"class": k, "count": v})).collect::<Vec<_>>(),
    }))
    .into_response()
}

pub(crate) async fn sample_update(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("pending");
    if !matches!(status, "pending" | "verified" | "ambiguous") {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "status 必须是 pending|verified|ambiguous")
            .into_response();
    }
    let boxes = body.get("boxes").cloned().unwrap_or(json!([]));
    if !boxes.is_array() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "boxes 必须是数组").into_response();
    }
    with_store(&st, |s| {
        if s.update_dataset_sample(id, &boxes.to_string(), status)? > 0 {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "sample not found").into_response())
        }
    })
}

/// GET /api/dataset/samples/:id/image——标注弹窗加载样本图（data_dir containment）。
pub(crate) async fn sample_image(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let Ok(Some(s)) = store.dataset_sample_by_id(id) else {
        return detail_err(StatusCode::NOT_FOUND, "sample not found").into_response();
    };
    let base = std::path::Path::new(&st.data_dir);
    let p = base.join(&s.image_path);
    let ok = p
        .canonicalize()
        .ok()
        .zip(base.canonicalize().ok())
        .map(|(a, b)| a.starts_with(&b))
        .unwrap_or(false);
    if !ok {
        return detail_err(StatusCode::NOT_FOUND, "image not found").into_response();
    }
    match std::fs::read(&p) {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, "image/jpeg")],
            bytes,
        )
            .into_response(),
        Err(_) => detail_err(StatusCode::NOT_FOUND, "image not found").into_response(),
    }
}

pub(crate) async fn sample_delete(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let rel = store
        .dataset_sample_by_id(id)
        .ok()
        .flatten()
        .map(|s| s.image_path);
    match store.delete_dataset_sample(id) {
        Ok(n) if n > 0 => {
            if let Some(rel) = rel {
                let base = std::path::Path::new(&st.data_dir);
                let p = base.join(rel);
                // 防越界：规范化后必须仍在 data_dir 内
                let ok = p
                    .canonicalize()
                    .ok()
                    .zip(base.canonicalize().ok())
                    .map(|(a, b)| a.starts_with(&b))
                    .unwrap_or(false);
                if ok {
                    let _ = std::fs::remove_file(p);
                }
            }
            Json(json!({"ok": true})).into_response()
        }
        _ => detail_err(StatusCode::NOT_FOUND, "sample not found").into_response(),
    }
}

/// 把像素框归一化为 YOLO 行：`cls cx cy w h`（0..1，clamp 防越界）。
pub fn yolo_label_line(b: &Value, img_w: i64, img_h: i64, class_index: usize) -> Option<String> {
    let x1 = b.get("x1")?.as_f64()?;
    let y1 = b.get("y1")?.as_f64()?;
    let x2 = b.get("x2")?.as_f64()?;
    let y2 = b.get("y2")?.as_f64()?;
    if !(img_w > 0 && img_h > 0) {
        return None;
    }
    let (x1, x2) = (x1.min(x2).clamp(0.0, img_w as f64), x1.max(x2).clamp(0.0, img_w as f64));
    let (y1, y2) = (y1.min(y2).clamp(0.0, img_h as f64), y1.max(y2).clamp(0.0, img_h as f64));
    let w = (x2 - x1) / img_w as f64;
    let h = (y2 - y1) / img_h as f64;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let cx = (x1 + x2) / 2.0 / img_w as f64;
    let cy = (y1 + y2) / 2.0 / img_h as f64;
    Some(format!("{class_index} {cx:.6} {cy:.6} {w:.6} {h:.6}"))
}

/// 导出 YOLO 训练集 ZIP（仅 verified；8:2 划分；含 data.yaml/classes.txt/README）。
pub(crate) async fn dataset_export(State(st): State<Shared>) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let samples = match store.dataset_samples(Some("verified"), 10_000) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    if samples.is_empty() {
        return detail_err(
            StatusCode::CONFLICT,
            "没有已核验样本——请先在标注池把样本标记为「已核验」",
        )
        .into_response();
    }
    // 类别表：固定标准词表（与 tools/merge_datasets.py 对齐）
    let classes = [
        "lab_coat", "no_lab_coat", "gloves", "no_gloves", "goggles", "no_goggles", "mask", "person",
    ];
    let class_index = |name: &str| -> usize {
        classes
            .iter()
            .position(|c| *c == name)
            .unwrap_or(classes.len() - 1)
    };
    let mut zip = crate::handlers::events::ZipWriter::new();
    let mut train_n = 0usize;
    let mut val_n = 0usize;
    for (i, s) in samples.iter().enumerate() {
        let src = std::path::Path::new(&st.data_dir).join(&s.image_path);
        let Ok(bytes) = std::fs::read(&src) else {
            continue;
        };
        let split = if i % 5 == 0 { "val" } else { "train" };
        if split == "val" {
            val_n += 1;
        } else {
            train_n += 1;
        }
        let stem = format!("{}_{}", s.source_type, s.id);
        zip.add_file(&format!("images/{split}/{stem}.jpg"), &bytes);
        let boxes: Vec<Value> = serde_json::from_str(&s.boxes_json).unwrap_or_default();
        let mut lines = Vec::new();
        for b in &boxes {
            let cls = b.get("class").and_then(|c| c.as_str()).unwrap_or("person");
            if let Some(line) = yolo_label_line(b, s.width, s.height, class_index(cls)) {
                lines.push(line);
            }
        }
        zip.add_file(
            &format!("labels/{split}/{stem}.txt"),
            lines.join("\n").as_bytes(),
        );
    }
    zip.add_file(
        "data.yaml",
        format!(
            "path: .\ntrain: images/train\nval: images/val\nnames:\n{}",
            classes
                .iter()
                .enumerate()
                .map(|(i, c)| format!("  {i}: {c}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .as_bytes(),
    );
    zip.add_file("classes.txt", classes.join("\n").as_bytes());
    zip.add_file(
        "README.txt",
        format!(
            "LabSentinel YOLO 训练集\n样本：{}（train {train_n} / val {val_n}）\n类别：{}\n\n训练示例：\n  yolo detect train data=data.yaml model=yolo11n.pt imgsz=640 epochs=80\n或使用 tools/train_ppe.py --user-zip <本zip>\n",
            samples.len(),
            classes.join(", ")
        )
        .as_bytes(),
    );
    let bytes = zip.finish();
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/zip"),
            (
                axum::http::header::CONTENT_DISPOSITION,
                "attachment; filename=\"labsentinel-yolo-dataset.zip\"",
            ),
        ],
        bytes,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yolo_label_normalizes_and_clamps() {
        let b = json!({"class":"person","x1":0.0,"y1":0.0,"x2":640.0,"y2":360.0});
        let line = yolo_label_line(&b, 640, 360, 7).unwrap();
        assert_eq!(line, "7 0.500000 0.500000 1.000000 1.000000");
        // 反向坐标自动纠正
        let b2 = json!({"x1":100.0,"y1":50.0,"x2":0.0,"y2":0.0});
        let line2 = yolo_label_line(&b2, 200, 100, 0).unwrap();
        assert!(line2.starts_with("0 "));
        // 退化框返回 None
        let b3 = json!({"x1":10.0,"y1":10.0,"x2":10.0,"y2":20.0});
        assert!(yolo_label_line(&b3, 200, 100, 0).is_none());
    }
}
