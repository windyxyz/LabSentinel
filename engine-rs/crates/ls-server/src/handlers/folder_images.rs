// folder-images endpoints + CSV.
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

pub(crate) async fn folder_images_list(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let parse_i64 = |k: &str| q.get(k).and_then(|v| v.parse().ok());
    let camera_id = parse_i64("camera_id");
    let key = parse_i64("key").map(|v| v != 0);
    let violation = parse_i64("violation").map(|v| v != 0);
    let start_ts = q.get("start_ts").map(|s| s.as_str());
    let end_ts = q.get("end_ts").map(|s| s.as_str());
    let error_only = q.get("error_only").map(|v| v == "1").unwrap_or(false);
    let limit = parse_i64("limit").unwrap_or(60).clamp(1, 500);
    let offset = parse_i64("offset").unwrap_or(0).max(0);
    with_store(&st, |s| {
        let f = ls_store::FolderImageFilter {
            camera_id,
            key,
            violation,
            start_ts,
            end_ts,
            error_only,
            limit,
            offset,
        };
        let rows = s.folder_images(&f)?;
        let total =
            s.count_folder_images(camera_id, key, violation, start_ts, end_ts, error_only)?;
        let names: std::collections::HashMap<i64, String> =
            s.cameras()?.into_iter().map(|c| (c.id, c.name)).collect();
        Ok(Json(json!({"items": rows.iter().map(|r| json!({
                "id": r.id, "camera_id": r.camera_id, "path": r.path, "name": r.name,
                "ts": r.ts, "processed_ts": r.processed_ts, "width": r.width,
                "height": r.height, "persons": r.persons, "in_zone": r.in_zone,
                "ppe_no": r.ppe_no, "ppe_wearing": r.ppe_wearing, "key": r.key,
                "violation": r.violation, "event_id": r.event_id,
                "snapshot": r.snapshot, "error": r.error}))
            .collect::<Vec<_>>(), "total": total, "limit": limit, "offset": offset,
            "camera_names": names}))
        .into_response())
    })
}

pub(crate) async fn folder_images_summary(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let days: i64 = q
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(7)
        .clamp(1, 3650);
    let camera_id: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let cams = s.cameras()?;
        let rows = s.folder_image_summary(days, camera_id)?;
        let mut items: Vec<Value> = Vec::new();
        let mut totals = std::collections::BTreeMap::<String, i64>::new();
        for k in ["total", "key", "violation", "persons", "errors"] {
            totals.insert(k.into(), 0);
        }
        let mut seen: std::collections::HashSet<i64> = Default::default();
        for r in &rows {
            let cid = r.camera_id;
            let cam = cams.iter().find(|c| c.id == cid);
            items.push(
                json!({"camera_id": cid, "camera_name": cam.map(|c| &c.name),
                "status": cam.map(|c| &c.status), "total": r.total, "key": r.key_n,
                "violation": r.violation_n, "persons": r.persons_sum,
                "in_zone": r.in_zone_sum, "errors": r.error_n,
                "last_image_ts": r.last_image_ts, "last_processed_ts": r.last_processed_ts}),
            );
            seen.insert(cid);
            for (k, val) in [
                ("total", r.total),
                ("key", r.key_n),
                ("violation", r.violation_n),
                ("persons", r.persons_sum),
                ("errors", r.error_n),
            ] {
                *totals.entry(k.into()).or_insert(0) += val;
            }
        }
        // 尚无识别记录的 folder 摄像头也列出（UI 显示"等待图片"）
        for c in &cams {
            if c.type_ != "folder" || seen.contains(&c.id) {
                continue;
            }
            if camera_id.map(|x| x != c.id).unwrap_or(false) {
                continue;
            }
            items.push(json!({"camera_id": c.id, "camera_name": c.name,
                "status": c.status, "total": 0, "key": 0, "violation": 0,
                "persons": 0, "in_zone": 0, "errors": 0,
                "last_image_ts": null, "last_processed_ts": null}));
        }
        items.sort_by_key(|x| x["camera_id"].as_i64().unwrap_or(0));
        Ok(Json(
            json!({"days": days, "items": items, "totals": totals}),
        ))
    })
}

/// POST /folders/{cid}/scan（RA1.3 摘 503）：wake-or-start（Python folder_scan 契约）。
/// 运行中的 folder worker → 置 wake（下轮立即扫描）；未运行 → 启动 folder worker。
pub(crate) async fn folder_scan(State(st): State<Shared>, AxPath(cid): AxPath<i64>) -> Response {
    use std::sync::atomic::Ordering;
    with_store(&st, |s| {
        let Some(cam) = s.camera(cid)? else {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        };
        if cam.type_ != "folder" {
            return Ok(
                detail_err(StatusCode::BAD_REQUEST, "该摄像头不是图片文件夹源").into_response(),
            );
        }
        if let Some(h) = st.workers.get(cid) {
            h.wake.store(true, Ordering::SeqCst);
            return Ok(
                Json(json!({"ok": true, "started": false, "msg": "已触发立即扫描"}))
                    .into_response(),
            );
        }
        match crate::worker::WorkerConfig::from_db(s, cid) {
            Err(e) => Ok(detail_err(StatusCode::INTERNAL_SERVER_ERROR, &e).into_response()),
            Ok(None) => Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response()),
            Ok(Some(cfg)) => match crate::worker::start_worker(&st, cfg) {
                Ok(()) => Ok(Json(
                    json!({"ok": true, "started": true, "msg": "图片文件夹接入已启动"}),
                )
                .into_response()),
                Err(e) => Ok(detail_err(StatusCode::INTERNAL_SERVER_ERROR, &e).into_response()),
            },
        }
    })
}

pub(crate) fn folder_row_json(r: &ls_store::FolderImageRow, path_full: bool) -> Value {
    let mut v = json!({"id": r.id, "camera_id": r.camera_id, "name": r.name,
        "ts": r.ts, "processed_ts": r.processed_ts, "width": r.width,
        "height": r.height, "persons": r.persons, "in_zone": r.in_zone,
        "ppe_no": r.ppe_no, "ppe_wearing": r.ppe_wearing, "key": r.key,
        "violation": r.violation, "event_id": r.event_id,
        "snapshot": r.snapshot, "error": r.error});
    if path_full {
        v["path"] = json!(r.path);
    }
    v
}

pub(crate) async fn folder_image_detail(
    State(st): State<Shared>,
    AxPath(iid): AxPath<i64>,
) -> Response {
    with_store(&st, |s| match s.folder_image(iid)? {
        Some(r) => Ok(Json(folder_row_json(&r, true)).into_response()),
        None => Ok(detail_err(StatusCode::NOT_FOUND, "image record not found").into_response()),
    })
}

/// 与 Python folder_readable_path 同口径：记录的 path 是真实文件路径
/// P1-3（安全审计）：containment 校验——path 必须 resolve 后位于该相机的
/// source 目录内（或 data_dir 内的 snapshot 回退），否则 403（DB path 被污染
/// 时防任意文件读取；对齐 Python commonpath 语义）。
pub(crate) async fn folder_image_file(
    State(st): State<Shared>,
    AxPath(iid): AxPath<i64>,
) -> Response {
    with_store(&st, |s| match s.folder_image(iid)? {
        Some(r) => {
            // containment：记录路径与相机 source 目录（或 data_dir）前缀比对
            let allowed = match s.camera(r.camera_id).ok().flatten() {
                Some(cam) => vec![cam.source.clone(), st.data_dir.clone()],
                None => vec![st.data_dir.clone()],
            };
            let norm = |p: &str| {
                std::path::Path::new(p)
                    .canonicalize()
                    .map(|c| c.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default()
            };
            let rp = norm(&r.path);
            let contained = !rp.is_empty()
                && allowed.iter().any(|base| {
                    let b = norm(base);
                    !b.is_empty() && rp.starts_with(&b)
                });
            if !contained {
                return Ok(detail_err(
                    StatusCode::FORBIDDEN,
                    "记录路径不在该摄像头源目录内（已拒绝）",
                )
                .into_response());
            }
            match std::fs::read(&r.path) {
                Ok(data) => {
                    let mt = if r.path.to_ascii_lowercase().ends_with(".png") {
                        "image/png"
                    } else {
                        "image/jpeg"
                    };
                    Ok((
                        [
                            (header::CONTENT_TYPE, mt),
                            (header::CACHE_CONTROL, "private, max-age=3600"),
                        ],
                        data,
                    )
                        .into_response())
                }
                Err(_) => Ok(detail_err(
                    StatusCode::NOT_FOUND,
                    "原图片文件不存在（可能已被系统清理）",
                )
                .into_response()),
            }
        }
        None => Ok(detail_err(StatusCode::NOT_FOUND, "image record not found").into_response()),
    })
}

pub(crate) async fn folder_image_thumb(
    State(st): State<Shared>,
    AxPath(iid): AxPath<i64>,
) -> Response {
    with_store(&st, |s| match s.folder_image(iid)? {
        Some(_r) => {
            // Python 侧缩略图缓存在 <data>/thumbs/img_{id}.jpg；rust 独立模式
            // 不生成缩略图（无图像处理依赖），命中缓存则回，否则如实 415
            let thumb = std::path::Path::new(&st.data_dir)
                .join("thumbs")
                .join(format!("img_{iid}.jpg"));
            match std::fs::read(&thumb) {
                Ok(data) => Ok((
                    [
                        (header::CONTENT_TYPE, "image/jpeg"),
                        (header::CACHE_CONTROL, "private, max-age=3600"),
                    ],
                    data,
                )
                    .into_response()),
                Err(_) => Ok(detail_err(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "缩略图缓存不存在（rust 控制面不生成缩略图；原图走 /file）",
                )
                .into_response()),
            }
        }
        None => Ok(detail_err(StatusCode::NOT_FOUND, "image record not found").into_response()),
    })
}

pub(crate) async fn folder_images_csv(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let parse_i64 = |k: &str| q.get(k).and_then(|v| v.parse().ok());
    let camera_id = parse_i64("camera_id");
    let key = parse_i64("key").map(|v| v != 0);
    let violation = parse_i64("violation").map(|v| v != 0);
    let start_ts = q.get("start_ts").map(|s| s.as_str());
    let end_ts = q.get("end_ts").map(|s| s.as_str());
    let error_only = q.get("error_only").map(|v| v == "1").unwrap_or(false);
    with_store(&st, |s| {
        let f = ls_store::FolderImageFilter {
            camera_id,
            key,
            violation,
            start_ts,
            end_ts,
            error_only,
            limit: 10_000,
            offset: 0,
        };
        let rows = s.folder_images(&f)?;
        let mut out = String::from(
            "id,camera_id,ts,name,persons,in_zone,ppe_no,ppe_wearing,key,violation,error\n",
        );
        for r in &rows {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                r.id,
                r.camera_id,
                csv_escape(&r.ts),
                csv_escape(r.name.as_deref().unwrap_or("")),
                r.persons,
                r.in_zone,
                r.ppe_no,
                r.ppe_wearing,
                r.key as i64,
                r.violation as i64,
                csv_escape(r.error.as_deref().unwrap_or(""))
            ));
        }
        Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8")], out).into_response())
    })
}
