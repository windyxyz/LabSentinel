// cameras endpoints (CRUD/group/analyzer/start/bulk/usb/copy-config/snapshot).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

pub(crate) async fn cameras(State(st): State<Shared>) -> Response {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.cameras() {
        Ok(rows) => Json(
            rows.iter()
                .map(|c| {
                    json!({"id": c.id, "name": c.name,
            "source": c.source, "type": c.type_, "status": c.status,
            "group_name": c.group_name, "analyzer": c.analyzer})
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => server_err(),
    }
}

pub(crate) async fn camera_status(State(st): State<Shared>, AxPath(id): AxPath<i64>) -> Response {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.camera(id) {
        Ok(Some(c)) => Json(json!({"id": c.id, "name": c.name, "type": c.type_,
            "status": c.status, "analyzer": c.analyzer, "fps": null}))
        .into_response(),
        Ok(None) => detail_err(StatusCode::NOT_FOUND, "camera not found").into_response(),
        Err(_) => server_err(),
    }
}

pub(crate) async fn add_camera(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    // T-103 等价：source 白名单（白名单外拒绝；file 源不强制存在，worker 报 error）
    let type_ = body.get("type").and_then(|v| v.as_str()).unwrap_or("file");
    let source = body.get("source").and_then(|v| v.as_str()).unwrap_or("");
    let valid = match type_ {
        "rtsp" => source.starts_with("rtsp://") || source.starts_with("rtsps://"),
        "file" => [".mp4", ".mkv", ".avi", ".mov", ".ts", ".flv"]
            .iter()
            .any(|e| source.to_ascii_lowercase().ends_with(e)),
        "usb" => source.chars().all(|c| c.is_ascii_digit()) && !source.is_empty(),
        "folder" => !source.is_empty(),
        _ => false,
    };
    if !valid || source.contains('#') || source.contains('\n') {
        return detail_err(StatusCode::BAD_REQUEST, "source 非法").into_response();
    }
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("cam");
    match ls_store::Store::open(&st.db_path) {
        Ok(s) => match s.conn_add_camera(name, source, type_) {
            Ok(id) => Json(json!({"id": id, "name": name, "source": source,
                                  "type": type_, "status": "stopped"}))
            .into_response(),
            Err(_) => server_err(),
        },
        Err(_) => server_err(),
    }
}

pub(crate) async fn set_camera_group(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let group = body
        .get("group_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        s.set_camera_group(cid, &group)?;
        Ok(Json(json!({"id": cid, "group_name": group})).into_response())
    })
}

pub(crate) async fn set_camera_analyzer(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let impl_ = body
        .get("analyzer")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if impl_ != "python" && impl_ != "rust" {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "analyzer 必须是 python|rust",
        )
        .into_response();
    }
    with_store(&st, |s| {
        let cam = match s.camera(cid)? {
            Some(c) => c,
            None => {
                return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response())
            }
        };
        if cam.analyzer == impl_ {
            return Ok(
                Json(json!({"id": cid, "analyzer": impl_, "restarted": false})).into_response(),
            );
        }
        s.set_camera_analyzer(cid, &impl_)?;
        // 运行中的摄像头需要重启才生效；rust 控制面独立模式无 worker，
        // 如实标注 restarted=false 并提示（F2-02：切换前提是分析 worker 可用）。
        Ok(
            Json(json!({"id": cid, "analyzer": impl_, "restarted": false,
                       "note": "重启该路摄像头后生效"}))
            .into_response(),
        )
    })
}

pub(crate) async fn start_camera(State(st): State<Shared>, AxPath(cid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        // rust 控制面独立模式没有分析 worker（数据面在 Python 服务 / sidecar）；
        // 如实返回 503 而不是假装启动成功（§11.3「声明完成必须附证据」约束）。
        Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                json!({"detail": "rust 控制面独立模式无分析 worker；启动摄像头请使用 Python 服务",
                        "code": "no_worker"}),
            ),
        )
            .into_response())
    })
}

pub(crate) async fn camera_webrtc(State(st): State<Shared>, AxPath(cid): AxPath<i64>) -> Response {
    with_store(&st, |s| match s.camera(cid)? {
        None => Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response()),
        Some(c) if c.type_ == "folder" => Ok(detail_err(
            StatusCode::BAD_REQUEST,
            "图片文件夹源没有实时流；请在「图片识别」页查看关键图片",
        )
        .into_response()),
        Some(_) => Ok(
            Json(json!({"camera_id": cid, "ws_url": format!("/ws/webrtc/{cid}")})).into_response(),
        ),
    })
}

pub(crate) async fn camera_groups(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let groups = s.camera_groups()?;
        Ok(Json(json!({"groups": groups})))
    })
}

pub(crate) async fn cameras_bulk(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let ids: Vec<i64> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default();
    let action = body
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if ids.is_empty()
        || ids.len() > 100
        || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
        || (action != "start" && action != "stop")
    {
        return detail_err(
            StatusCode::BAD_REQUEST,
            "ids 必须为去重非空列表且 ≤100 个；action=start|stop",
        )
        .into_response();
    }
    with_store(&st, |s| {
        let mut missing = Vec::new();
        let mut failed = Vec::new();
        for cid in &ids {
            if s.camera(*cid)?.is_none() {
                missing.push(*cid);
                continue;
            }
            // 独立控制面无分析 worker：如实计入 failed（与 Python failed 列表同形）
            failed.push(json!({"id": cid, "error":
                "rust 控制面独立模式无分析 worker（启停请使用 Python 服务）"}));
        }
        let ok: Vec<i64> = ids
            .iter()
            .filter(|i| !missing.contains(i) && !failed.iter().any(|f| f["id"] == **i))
            .copied()
            .collect();
        Ok(Json(json!({"ok": ok, "missing": missing, "failed": failed})).into_response())
    })
}

pub(crate) async fn usb_devices() -> Response {
    // rust 控制面不做 DirectShow 枚举（前端回退手输索引，与"未找到 ffmpeg"路径一致）
    Json(json!({"devices": [],
                "error": "rust 控制面暂不支持 USB 枚举；启动 Python 服务后可用"}))
    .into_response()
}

pub(crate) async fn copy_camera_config(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let targets: Vec<i64> = body
        .get("target_ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            let mut ids: Vec<i64> = a.iter().filter_map(|x| x.as_i64()).collect();
            ids.sort();
            ids.dedup();
            ids
        })
        .unwrap_or_default();
    let copy_zones = body
        .get("copy_zones")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let copy_rules = body
        .get("copy_rules")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let suffix = body
        .get("name_suffix")
        .and_then(|v| v.as_str())
        .unwrap_or("-copy")
        .to_string();
    if targets.is_empty() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "target_ids 必填").into_response();
    }
    if targets.contains(&cid) {
        return detail_err(StatusCode::BAD_REQUEST, "target_ids 不能包含源摄像头自身")
            .into_response();
    }
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        for t in &targets {
            if s.camera(*t)?.is_none() {
                return Ok((
                    StatusCode::NOT_FOUND,
                    Json(json!({"detail": format!("目标摄像头不存在: #{t}")})),
                )
                    .into_response());
            }
        }
        // Python 语义：先全量校验再写库；规则 zone_id 按区域名映射到目标
        let src_zones = s.zones_all()?;
        let src_zones: Vec<_> = src_zones.into_iter().filter(|z| z.1 == cid).collect();
        let src_rules = s.rules_all()?;
        let src_rules: Vec<_> = src_rules.into_iter().filter(|r| r.1 == cid).collect();
        let mut zones_created = 0i64;
        let mut zones_skipped = 0i64;
        let mut rules_created = 0i64;
        let mut rules_skipped = 0i64;
        let mut warnings: Vec<String> = Vec::new();
        for tid in &targets {
            let tgt_zones = s.zones_all()?;
            let mut zone_map: std::collections::HashMap<String, i64> = tgt_zones
                .iter()
                .filter(|z| z.1 == *tid)
                .map(|z| (z.2.clone(), z.0))
                .collect();
            if copy_zones {
                for z in &src_zones {
                    let new_name = format!("{}{}", z.2, suffix);
                    if let Some(existing) = zone_map.get(&new_name) {
                        zone_map.insert(z.2.clone(), *existing);
                        zones_skipped += 1;
                        continue;
                    }
                    let zid = s.add_zone(*tid, &new_name, &z.3)?;
                    zone_map.insert(z.2.clone(), zid);
                    zone_map.insert(new_name, zid);
                    zones_created += 1;
                }
            } else {
                zones_skipped += src_zones.len() as i64;
            }
            if copy_rules {
                for r in &src_rules {
                    let zid = r.2.and_then(|zid| {
                        let src_name = src_zones.iter().find(|z| z.0 == zid).map(|z| z.2.clone());
                        src_name.and_then(|n| zone_map.get(&n).copied())
                    });
                    if r.2.is_some() && zid.is_none() {
                        rules_skipped += 1;
                        warnings.push(format!(
                            "目标 #{tid}：规则 {} 的源区域在目标上不存在（未复制区域或同名缺失）——已跳过",
                            r.3
                        ));
                        continue;
                    }
                    // 去重：同 behavior+zone+时段
                    let dup = s.rules_all()?.iter().any(|x| {
                        x.1 == *tid && x.2 == zid && x.3 == r.3 && x.6 == r.6 && x.7 == r.7
                    });
                    if dup {
                        rules_skipped += 1;
                        continue;
                    }
                    s.add_rule(
                        *tid,
                        zid,
                        &r.3,
                        r.4,
                        r.5,
                        r.6.as_deref(),
                        r.7.as_deref(),
                        r.8,
                        r.9 != 0,
                    )?;
                    rules_created += 1;
                }
            } else {
                rules_skipped += src_rules.len() as i64;
            }
        }
        Ok(Json(json!({"ok": true, "zones_created": zones_created,
                       "zones_skipped": zones_skipped, "rules_created": rules_created,
                       "rules_skipped": rules_skipped, "warnings": warnings}))
        .into_response())
    })
}

pub(crate) async fn camera_snapshot(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
) -> Response {
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        // rust 控制面独立模式无分析 worker（无实时帧缓冲）；与 start_camera 同口径
        Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"detail": "rust 控制面独立模式无分析 worker（无实时帧）；实时画面/截图请使用 Python 服务",
                        "code": "no_worker"})),
        )
            .into_response())
    })
}

// ==== R2-309 批次 3：配置导入/存证打包/插件占位/face 分支/告警自检/PPE 下载 ====
