//! LS-054：网页端便携标注工作台（数据飞轮）——样本入池 / AI 预标注 / 人工修订 /
//! 标准 YOLO 训练集 ZIP 导出。
//!
//! 存储：`dataset_samples`（v16）。图片统一拷入 `data/dataset_pool/`，
//! 标注框格式 `[{class, x1,y1,x2,y2}]`（像素坐标，导出时归一化）。
//! 导出：仅 `verified` 样本；按 8:2 划分 train/val，输出 images/ + labels/ +
//! data.yaml + classes.txt + manifest.json + README.txt。

use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufReader, BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

const TRAINING_CLASSES: &[&str] = &[
    "lab_coat",
    "no_lab_coat",
    "gloves",
    "no_gloves",
    "goggles",
    "no_goggles",
    "mask",
    "person",
    "no_mask",
];
const TRAINING_SETS: &[(&str, &[&str])] = &[
    ("coat", &["lab_coat", "no_lab_coat"]),
    ("mask", &["mask", "no_mask"]),
    ("gloves", &["gloves", "no_gloves"]),
    ("goggles", &["goggles", "no_goggles"]),
    ("person", &["person"]),
];

fn training_sets(boxes: &Value) -> Vec<&'static str> {
    let Some(boxes) = boxes.as_array() else {
        return Vec::new();
    };
    TRAINING_SETS
        .iter()
        .filter_map(|(name, classes)| {
            boxes
                .iter()
                .any(|b| b["class"].as_str().is_some_and(|c| classes.contains(&c)))
                .then_some(*name)
        })
        .collect()
}

fn pool_dir(st: &Shared) -> PathBuf {
    std::path::Path::new(&st.data_dir).join("dataset_pool")
}

/// 拷贝源图到样本池，返回相对路径（相对 data_dir）。
fn copy_into_pool(st: &Shared, src: &Path, tag: &str) -> Result<(String, u32, u32), String> {
    let (bgr, w, h) =
        ls_ingest::decode_image(src, 1280, 720).map_err(|e| format!("图片解码失败: {e}"))?;
    let dir = pool_dir(st);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建样本池目录失败: {e}"))?;
    let name = format!("{tag}_{}.jpg", uuid_like());
    let dest = dir.join(&name);
    let jpeg = ls_analyzer::evidence::encode_jpeg_bgr(&bgr, w, h)
        .map_err(|e| format!("样本编码失败: {e}"))?;
    std::fs::write(&dest, &jpeg).map_err(|e| format!("写入样本失败: {e}"))?;
    let rel = format!("dataset_pool/{name}");
    Ok((rel, w, h))
}

/// AI 预标注：本地人员检测 → boxes_json。
fn preannotate(st: &Shared, cid: i64, src: &Path) -> String {
    let Ok(Some(cfg)) = ls_store::Store::open(&st.db_path)
        .map_err(|e| e.to_string())
        .and_then(|store| crate::worker::WorkerConfig::from_db(&store, cid))
    else {
        return "[]".into();
    };
    let mut det = ls_analyzer::detector::make_detector(&cfg.detector);
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
    let sets = training_sets(&boxes);
    json!({
        "id": s.id, "source_type": s.source_type, "source_id": s.source_id,
        "camera_id": s.camera_id, "image_path": s.image_path,
        "width": s.width, "height": s.height, "status": s.status,
        "boxes": boxes, "training_sets": sets, "created_at": s.created_at, "updated_at": s.updated_at,
        "lighting": s.lighting, "is_hard_negative": s.is_hard_negative,
        "source_fp_spot_id": s.source_fp_spot_id,
    })
}

pub(crate) async fn samples_from_event(
    State(st): State<Shared>,
    AxPath(eid): AxPath<i64>,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let Ok(store) = ls_store::Store::open(&st.db_path) else {
            return server_err();
        };
        match stage_event_sample(&st, &store, eid, true) {
            Ok(id) => created_sample(&store, id),
            Err((code, error)) => detail_err(code, &error).into_response(),
        }
    })
    .await
    .unwrap_or_else(|_| server_err())
}

pub(super) fn stage_event_sample(
    st: &Shared,
    store: &ls_store::Store,
    eid: i64,
    annotate: bool,
) -> Result<i64, (StatusCode, String)> {
    let db_error = |_| (StatusCode::INTERNAL_SERVER_ERROR, "暂时无法保存样本".into());
    let root = Path::new(&st.data_dir);
    if let Some(id) = store.event_sample_id(eid).map_err(db_error)? {
        return Ok(id);
    }
    let Some(event) = store.event_by_id(eid).map_err(db_error)? else {
        return Err((StatusCode::NOT_FOUND, "事件不存在".into()));
    };
    let Some(src) = super::events::event_image_path(st, store, &event) else {
        return Err((StatusCode::NOT_FOUND, "事件没有可用截图".into()));
    };
    let (rel, w, h) = copy_into_pool(st, &src, &format!("event_{eid}"))
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    let photo = root.join(&rel);
    let boxes = if annotate {
        preannotate(st, event.camera_id, &photo)
    } else {
        "[]".into()
    };
    match store.add_event_sample_once(&event, &rel, (w, h), &boxes) {
        Ok((id, true)) => Ok(id),
        result => {
            let _ = std::fs::remove_file(&photo);
            result.map(|(id, _)| id).map_err(db_error)
        }
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
    let Some(src) = store
        .camera(img.camera_id)
        .ok()
        .flatten()
        .filter(|c| c.type_ == "folder")
        .and_then(|c| rooted_file(Path::new(&c.source), Path::new(&img.path)))
    else {
        return detail_err(StatusCode::NOT_FOUND, "folder image not found").into_response();
    };
    let tag = format!("folder_{iid}");
    let (rel, w, h) = match copy_into_pool(&st, &src, &tag) {
        Ok(v) => v,
        Err(e) => return detail_err(StatusCode::BAD_REQUEST, &e).into_response(),
    };
    let boxes = preannotate(&st, img.camera_id, &Path::new(&st.data_dir).join(&rel));
    match store.add_dataset_sample(
        "folder",
        &iid.to_string(),
        Some(img.camera_id),
        &rel,
        w as i64,
        h as i64,
        &boxes,
    ) {
        Ok(id) => created_sample(&store, id),
        Err(_) => {
            let _ = std::fs::remove_file(Path::new(&st.data_dir).join(&rel));
            server_err()
        }
    }
}

pub(crate) async fn samples_from_suppression(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let Ok(store) = ls_store::Store::open(&st.db_path) else {
            return server_err();
        };
        match stage_suppression_sample(&st, &store, id) {
            Ok(id) => created_sample(&store, id),
            Err((status, error)) => detail_err(status, &error).into_response(),
        }
    })
    .await
    .unwrap_or_else(|_| server_err())
}

fn stage_suppression_sample(
    st: &Shared,
    store: &ls_store::Store,
    id: i64,
) -> Result<i64, (StatusCode, String)> {
    let db_error = |_| (StatusCode::INTERNAL_SERVER_ERROR, "暂时无法保存样本".into());
    if let Some(existing) = store.suppression_sample_id(id).map_err(db_error)? {
        return Ok(existing);
    }
    let unavailable = || (StatusCode::NOT_FOUND, "抑制记录没有可用截图".to_owned());
    let (cid, source) = store
        .suppression_snapshot(id)
        .map_err(db_error)?
        .ok_or_else(unavailable)?;
    let root = Path::new(&st.data_dir);
    let source = rooted_file(root, Path::new(&source)).ok_or_else(unavailable)?;
    let jpeg = ls_jobs::media::read_snapshot(&source)
        .filter(|bytes| bytes.starts_with(&[0xff, 0xd8]))
        .ok_or_else(unavailable)?;
    let image = ls_jobs::media::decode_image(&jpeg).ok_or_else(unavailable)?;
    let path = format!("dataset_pool/suppression_{id}_{}.jpg", uuid_like());
    let absolute = root.join(&path);
    std::fs::create_dir_all(pool_dir(st)).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("创建样本目录失败: {error}"),
        )
    })?;
    if std::fs::write(&absolute, &jpeg).is_err() {
        let _ = std::fs::remove_file(&absolute);
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "写入样本失败".into()));
    }
    let result = (|| {
        let tx = rusqlite::Transaction::new_unchecked(
            store.conn(),
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(db_error)?;
        if let Some(existing) = store.suppression_sample_id(id).map_err(db_error)? {
            return Ok((existing, false));
        }
        if store.suppression_snapshot(id).map_err(db_error)?.is_none() {
            return Err(unavailable());
        }
        let sample = store
            .add_dataset_sample(
                "suppression",
                &id.to_string(),
                Some(cid),
                &path,
                i64::from(image.width()),
                i64::from(image.height()),
                "[]",
            )
            .map_err(db_error)?;
        tx.execute(
            "UPDATE dataset_samples SET source_fp_spot_id=(SELECT source_id FROM suppression_audit
             WHERE id=? AND reason='remembered_spot') WHERE id=?",
            rusqlite::params![id, sample],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok((sample, true))
    })();
    if !matches!(&result, Ok((_, true))) {
        let _ = std::fs::remove_file(absolute);
    }
    result.map(|(sample, _)| sample)
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
        .map(|frame| frame.jpeg)
        .unwrap_or_default();
    if jpeg.is_empty() {
        return detail_err(StatusCode::CONFLICT, "暂无可用画面（等待抽帧）").into_response();
    }
    let dir = pool_dir(&st);
    if std::fs::create_dir_all(&dir).is_err() {
        return server_err();
    }
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let name = format!("capture_{cid}_{}.jpg", uuid_like());
    let dest = dir.join(&name);
    if std::fs::write(&dest, &jpeg).is_err() {
        return server_err();
    }
    let captured = copy_into_pool(&st, &dest, &format!("capture_{cid}"));
    let _ = std::fs::remove_file(&dest);
    let Ok((rel, w, h)) = captured else {
        return server_err();
    };
    let boxes = preannotate(&st, cid, &Path::new(&st.data_dir).join(&rel));
    match store.add_dataset_sample(
        "capture",
        &cid.to_string(),
        Some(cid),
        &rel,
        w.into(),
        h.into(),
        &boxes,
    ) {
        Ok(id) => created_sample(&store, id),
        Err(_) => {
            let _ = std::fs::remove_file(Path::new(&st.data_dir).join(&rel));
            server_err()
        }
    }
}

fn created_sample(store: &ls_store::Store, id: i64) -> Response {
    match store.dataset_sample_by_id(id) {
        Ok(Some(sample)) => {
            Json(json!({"ok": true, "id": id, "sample": sample_json(&sample)})).into_response()
        }
        _ => server_err(),
    }
}

pub(crate) async fn samples_list(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let status = q
        .get("status")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty());
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(200)
        .clamp(1, 2000);
    let rows = match store.dataset_samples(status, limit) {
        Ok(r) => r,
        Err(_) => return server_err(),
    };
    let class = q.get("class").filter(|class| !class.is_empty());
    let lighting = q.get("lighting").filter(|value| !value.is_empty());
    let training_set = q
        .get("set")
        .filter(|name| !name.is_empty() && name.as_str() != "all");
    let samples: Vec<_> = rows
        .iter()
        .map(sample_json)
        .filter(|sample| {
            lighting.is_none_or(|value| sample["lighting"].as_str() == Some(value))
                && training_set.is_none_or(|name| {
                    sample["training_sets"]
                        .as_array()
                        .is_some_and(|sets| sets.iter().any(|s| s.as_str() == Some(name)))
                })
                && class.is_none_or(|class| {
                    sample["boxes"].as_array().is_some_and(|boxes| {
                        boxes
                            .iter()
                            .any(|b| b["class"].as_str() == Some(class.as_str()))
                    })
                })
        })
        .collect();
    let counts = store.dataset_class_counts().unwrap_or_default();
    Json(json!({
        "total": samples.len(),
        "samples": samples,
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
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "status 必须是 pending|verified|ambiguous",
        )
        .into_response();
    }
    let boxes = body.get("boxes").cloned().unwrap_or(json!([]));
    if !boxes.is_array() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "boxes 必须是数组").into_response();
    }
    let lighting = body.get("lighting").and_then(Value::as_str);
    let hard_negative = body.get("is_hard_negative").and_then(Value::as_bool);
    if (body.get("lighting").is_some()
        && !matches!(lighting, Some("unknown" | "normal" | "low_light")))
        || (body.get("is_hard_negative").is_some() && hard_negative.is_none())
    {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "样本光照或困难负样本标记无效",
        )
        .into_response();
    }
    with_store(&st, |s| {
        if s.update_dataset_annotations(id, &boxes.to_string(), status, lighting, hard_negative)?
            > 0
        {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "sample not found").into_response())
        }
    })
}

/// GET /api/dataset/samples/:id/image——标注弹窗加载样本图（data_dir containment）。
pub(crate) async fn sample_image(State(st): State<Shared>, AxPath(id): AxPath<i64>) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let Ok(Some(s)) = store.dataset_sample_by_id(id) else {
        return detail_err(StatusCode::NOT_FOUND, "sample not found").into_response();
    };
    let Some(p) = sample_path(&st, &s.image_path) else {
        return detail_err(StatusCode::NOT_FOUND, "image not found").into_response();
    };
    match std::fs::read(&p) {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        Err(_) => detail_err(StatusCode::NOT_FOUND, "image not found").into_response(),
    }
}

pub(crate) async fn sample_delete(State(st): State<Shared>, AxPath(id): AxPath<i64>) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let rel = store
        .dataset_sample_by_id(id)
        .ok()
        .flatten()
        .map(|s| s.image_path);
    if let Some(path) = rel.and_then(|rel| sample_path(&st, &rel)) {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return detail_err(
                    StatusCode::CONFLICT,
                    "无法删除标注池副本，请检查文件占用或权限",
                )
                .into_response();
            }
        }
    }
    match store.delete_dataset_sample(id) {
        Ok(n) if n > 0 => Json(json!({"ok": true})).into_response(),
        _ => detail_err(StatusCode::NOT_FOUND, "sample not found").into_response(),
    }
}

pub(super) fn sample_path(st: &Shared, relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative).strip_prefix("dataset_pool").ok()?;
    rooted_file(&pool_dir(st), path)
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
    let (x1, x2) = (
        x1.min(x2).clamp(0.0, img_w as f64),
        x1.max(x2).clamp(0.0, img_w as f64),
    );
    let (y1, y2) = (
        y1.min(y2).clamp(0.0, img_h as f64),
        y1.max(y2).clamp(0.0, img_h as f64),
    );
    let w = (x2 - x1) / img_w as f64;
    let h = (y2 - y1) / img_h as f64;
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let cx = (x1 + x2) / 2.0 / img_w as f64;
    let cy = (y1 + y2) / 2.0 / img_h as f64;
    Some(format!("{class_index} {cx:.6} {cy:.6} {w:.6} {h:.6}"))
}

#[derive(Default, serde::Deserialize)]
pub(crate) struct DatasetExport {
    set: Option<String>,
    exclude: Option<String>,
    lighting: Option<String>,
}

/// 在阻塞线程准备 ZIP，再以有界缓冲输出到浏览器。
pub(crate) async fn dataset_export(
    State(st): State<Shared>,
    Query(query): Query<DatasetExport>,
) -> Response {
    use axum::http::header;
    let (training_set, classes) = match query.set.as_deref() {
        None | Some("" | "all") => ("all", TRAINING_CLASSES),
        Some(id) => {
            let Some(&(name, classes)) = TRAINING_SETS.iter().find(|(name, _)| *name == id) else {
                return detail_err(StatusCode::BAD_REQUEST, "未知训练集").into_response();
            };
            (name, classes)
        }
    };
    let exclude = query.exclude.as_deref().unwrap_or("");
    let excluded = exclude
        .split(',')
        .filter(|id| !id.is_empty())
        .map(str::parse::<i64>)
        .collect::<Result<std::collections::HashSet<_>, _>>();
    let Ok(excluded) = excluded else {
        return detail_err(StatusCode::BAD_REQUEST, "排除样本 ID 无效").into_response();
    };
    if !matches!(
        query.lighting.as_deref(),
        None | Some("unknown" | "normal" | "low_light")
    ) {
        return detail_err(StatusCode::BAD_REQUEST, "未知光照筛选").into_response();
    }
    let DatasetArchive { file, size } = match tokio::task::spawn_blocking(move || {
        build_dataset_zip(
            &st,
            training_set,
            classes,
            query.lighting.as_deref(),
            &excluded,
        )
    })
    .await
    {
        Ok(Ok(Some(zip))) => zip,
        Ok(Ok(None)) => {
            return detail_err(
                StatusCode::CONFLICT,
                "没有可导出的图片。请先核验样本，并检查已核验样本的原图是否仍在标注池。",
            )
            .into_response()
        }
        _ => {
            return detail_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "导出中断。请检查磁盘空间或样本是否刚被移除，再重试。",
            )
            .into_response()
        }
    };
    let chunks =
        tokio_util::io::ReaderStream::with_capacity(tokio::fs::File::from_std(file), 65_536);
    (
        [
            (header::CONTENT_TYPE, "application/zip".into()),
            (header::CONTENT_LENGTH, size.to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"labsentinel-yolo-dataset.zip\"".into(),
            ),
        ],
        axum::body::Body::from_stream(chunks),
    )
        .into_response()
}

struct DatasetArchive {
    file: File,
    size: u64,
}

fn build_dataset_zip(
    st: &Shared,
    training_set: &str,
    classes: &[&str],
    lighting: Option<&str>,
    excluded: &std::collections::HashSet<i64>,
) -> Result<Option<DatasetArchive>, Box<dyn std::error::Error + Send + Sync>> {
    let store = ls_store::Store::open(&st.db_path)?;
    let samples = store.dataset_samples(Some("verified"), 10_000)?;
    let selected = samples
        .into_iter()
        .filter(|sample| !excluded.contains(&sample.id))
        .filter(|sample| lighting.is_none_or(|name| sample.lighting == name))
        .map(|sample| {
            let boxes = serde_json::from_str::<Vec<Value>>(&sample.boxes_json).unwrap_or_default();
            let labels = boxes
                .iter()
                .filter_map(|b| {
                    let name = b["class"].as_str()?;
                    let index = classes.iter().position(|c| *c == name)?;
                    yolo_label_line(b, sample.width, sample.height, index)
                })
                .collect::<Vec<_>>();
            (sample, labels)
        })
        .filter(|(_, labels)| training_set == "all" || !labels.is_empty())
        .collect::<Vec<_>>();
    let mut unavailable = Vec::new();
    let samples: Vec<_> = selected
        .into_iter()
        .filter_map(|(sample, labels)| {
            let Some(path) = sample_path(st, &sample.image_path).filter(|path| {
                File::open(path).is_ok_and(|file| {
                    image::ImageReader::with_format(BufReader::new(file), image::ImageFormat::Jpeg)
                        .into_dimensions()
                        .is_ok()
                })
            }) else {
                unavailable.push(sample.id);
                return None;
            };
            Some((sample, path, labels))
        })
        .collect();
    if samples.is_empty() {
        return Ok(None);
    }
    let mut zip = zip::ZipWriter::new(BufWriter::new(tempfile::tempfile()?));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let count = samples.len();
    let weak_n = samples
        .iter()
        .filter(|(sample, _, _)| sample.lighting == "low_light")
        .count();
    let ordinary_n = count - weak_n;
    let val_n = if ordinary_n < 2 {
        0
    } else {
        ordinary_n.div_ceil(5)
    };
    let train_n = ordinary_n - val_n;
    let mut entries = Vec::with_capacity(count);
    let mut ordinary_index = 0;
    for (s, path, lines) in &samples {
        let split = if s.lighting == "low_light" {
            "val_weak"
        } else if val_n > 0 && ordinary_index % 5 == 0 {
            "val"
        } else {
            "train"
        };
        if split != "val_weak" {
            ordinary_index += 1;
        }
        let stem = format!("sample_{}", s.id);
        let image_path = format!("images/{split}/{stem}.jpg");
        let label_path = format!("labels/{split}/{stem}.txt");
        let image = std::fs::read(path)?;
        let labels = lines.join("\n");
        zip.start_file(&image_path, options)?;
        zip.write_all(&image)?;
        zip.start_file(&label_path, options)?;
        zip.write_all(labels.as_bytes())?;
        entries.push(json!({
            "id": s.id, "source_type": s.source_type, "source_id": s.source_id,
            "camera_id": s.camera_id, "split": split,
            "lighting": s.lighting, "is_hard_negative": s.is_hard_negative,
            "source_fp_spot_id": s.source_fp_spot_id,
            "image": image_path, "labels": label_path,
            "image_sha256": sha256_hex(&image), "labels_sha256": sha256_hex(labels.as_bytes()),
        }));
    }
    let mut excluded_ids = excluded.iter().copied().collect::<Vec<_>>();
    excluded_ids.sort_unstable();
    let mut manifest = json!({
        "format_version": 1, "training_set": training_set, "classes": classes, "lighting_filter": lighting,
        "samples": entries, "excluded_sample_ids": excluded_ids, "unavailable_sample_ids": unavailable,
    });
    let version = sha256_hex(&serde_json::to_vec(&manifest)?);
    manifest["version"] = json!(version);
    zip.start_file("manifest.json", options)?;
    serde_json::to_writer_pretty(&mut zip, &manifest)?;
    let instructions = if val_n > 0 {
        zip.start_file("data.yaml", options)?;
        write!(
            zip,
            "train: images/train\nval: images/val\nnames:\n{}\n",
            classes
                .iter()
                .enumerate()
                .map(|(i, c)| format!("  {i}: {c}"))
                .collect::<Vec<_>>()
                .join("\n")
        )?;
        "解压后执行 python tools/train_ppe.py --data <解压目录>/data.yaml --preflight。"
    } else {
        "常规训练和验证样本不足。低光样本单独置于 val_weak，可供独立评测。"
    };
    zip.start_file("classes.txt", options)?;
    zip.write_all(classes.join("\n").as_bytes())?;
    zip.start_file("README.txt", options)?;
    write!(
        zip,
        "LabSentinel YOLO 样本\n已导出 {count} 张（train {train_n} / val {val_n} / val_weak {weak_n}）\n缺失或不可读 {} 张\n样本、排除项和文件摘要见 manifest.json。\n\n{instructions}\n",
        unavailable.len(),
    )?;
    let mut file = zip.finish()?.into_inner()?;
    let size = file.metadata()?.len();
    file.rewind()?;
    Ok(Some(DatasetArchive { file, size }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn export_by_training_set_skips_selected_samples_without_mutating_pool() {
        use crate::tests::helpers::{req, test_state_cfg_dir};
        use http_body_util::BodyExt;
        use std::io::{Cursor, Read};

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("training.db");
        let store = ls_store::Store::open(db.to_str().unwrap()).unwrap();
        store.ensure_schema().unwrap();
        let pool = dir.path().join("dataset_pool");
        std::fs::create_dir(&pool).unwrap();
        let jpeg = ls_analyzer::evidence::encode_jpeg_bgr(&[64; 48], 4, 4).unwrap();
        for (name, classes) in [
            ("combined", &["lab_coat", "no_mask"][..]),
            ("skip", &["mask"][..]),
            ("coat", &["no_lab_coat"][..]),
        ] {
            std::fs::write(pool.join(format!("{name}.jpg")), &jpeg).unwrap();
            let boxes = classes
                .iter()
                .map(|class| json!({"class":class,"x1":0,"y1":0,"x2":3,"y2":3}))
                .collect::<Vec<_>>();
            let id = store
                .add_dataset_sample(
                    "event",
                    name,
                    None,
                    &format!("dataset_pool/{name}.jpg"),
                    4,
                    4,
                    "[]",
                )
                .unwrap();
            store
                .update_dataset_sample(id, &json!(boxes).to_string(), "verified")
                .unwrap();
        }
        let app = crate::build_router(test_state_cfg_dir(
            db.to_str().unwrap(),
            "",
            false,
            dir.path().to_str().unwrap(),
        ));
        let response = req(
            &app,
            "GET",
            "/api/dataset/export.zip?set=mask&exclude=2",
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(
            zip.file_names()
                .map(Result::unwrap)
                .filter(|name| name.ends_with(".jpg"))
                .collect::<Vec<_>>(),
            ["images/train/sample_1.jpg"]
        );
        let mut labels = String::new();
        zip.by_name("labels/train/sample_1.txt")
            .unwrap()
            .read_to_string(&mut labels)
            .unwrap();
        assert_eq!(labels.lines().count(), 1);
        assert!(labels.starts_with("1 "));
        let manifest: Value =
            serde_json::from_reader(zip.by_name("manifest.json").unwrap()).unwrap();
        assert_eq!(manifest["format_version"], 1);
        assert_eq!(manifest["training_set"], "mask");
        assert_eq!(manifest["excluded_sample_ids"], json!([2]));
        let exported = manifest["samples"].as_array().unwrap();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0]["id"], 1);
        assert_eq!(exported[0]["image"], "images/train/sample_1.jpg");
        assert_eq!(exported[0]["image_sha256"], sha256_hex(&jpeg));
        assert_eq!(exported[0]["labels_sha256"], sha256_hex(labels.as_bytes()));
        assert_eq!(exported[0]["source_type"], "event");
        assert_eq!(exported[0]["source_id"], "combined");
        let mut payload = manifest.clone();
        let version = payload.as_object_mut().unwrap().remove("version").unwrap();
        assert_eq!(version, sha256_hex(&serde_json::to_vec(&payload).unwrap()));
        assert!(pool.join("skip.jpg").is_file());
        let response = req(
            &app, "PUT", "/api/dataset/samples/2",
            Some(json!({"status":"verified","boxes":[],"lighting":"low_light","is_hard_negative":true})),
        ).await;
        assert_eq!(response.status(), StatusCode::OK);
        let response = req(
            &app,
            "GET",
            "/api/dataset/export.zip?lighting=low_light",
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let mut weak = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(weak.by_name("images/val_weak/sample_2.jpg").is_ok());
        assert!(weak.by_name("data.yaml").is_err());
        let manifest: Value =
            serde_json::from_reader(weak.by_name("manifest.json").unwrap()).unwrap();
        assert_eq!(manifest["samples"][0]["lighting"], "low_light");
        assert_eq!(manifest["samples"][0]["is_hard_negative"], true);
        let response = req(&app, "DELETE", "/api/dataset/samples/2", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!pool.join("skip.jpg").exists());
        assert!(pool.join("combined.jpg").is_file());
        assert!(store.dataset_sample_by_id(2).unwrap().is_none());
    }

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

    #[tokio::test]
    async fn export_splits_only_readable_verified_images() {
        use crate::tests::helpers::{req, test_state_cfg_dir};
        use http_body_util::BodyExt;
        use std::io::Cursor;

        for count in [0, 1, 2, 4, 5] {
            let dir = tempfile::tempdir().unwrap();
            let db = dir.path().join("export.db");
            let store = ls_store::Store::open(db.to_str().unwrap()).unwrap();
            store.ensure_schema().unwrap();
            let pool = dir.path().join("dataset_pool");
            std::fs::create_dir(&pool).unwrap();
            let photo = ls_analyzer::evidence::encode_jpeg_bgr(&[64; 48], 4, 4).unwrap();
            for i in 0..=count {
                let name = format!("test{i}.jpg");
                if i < count {
                    std::fs::write(pool.join(&name), &photo).unwrap();
                }
                let id = store
                    .add_dataset_sample(
                        "event",
                        "1",
                        None,
                        &format!("dataset_pool/{name}"),
                        4,
                        4,
                        "[]",
                    )
                    .unwrap();
                store
                    .conn()
                    .execute(
                        "UPDATE dataset_samples SET status='verified' WHERE id=?",
                        [id],
                    )
                    .unwrap();
            }
            let st = test_state_cfg_dir(
                db.to_str().unwrap(),
                "",
                false,
                dir.path().to_str().unwrap(),
            );
            let router = crate::router::build_router(st);
            let response = req(&router, "GET", "/api/dataset/export.zip", None).await;
            if count == 0 {
                assert_eq!(response.status(), StatusCode::CONFLICT);
                continue;
            }
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            let names: Vec<_> = zip.file_names().map(Result::unwrap).collect();
            let val = usize::from(count > 1);
            assert_eq!(
                names
                    .iter()
                    .filter(|name| name.starts_with("images/train/"))
                    .count(),
                count - val
            );
            assert_eq!(
                names
                    .iter()
                    .filter(|name| name.starts_with("images/val/"))
                    .count(),
                val
            );
            assert_eq!(names.iter().any(|name| name == "data.yaml"), count > 1);
            assert!(names.iter().any(|name| name == "classes.txt"));
        }
    }
}
