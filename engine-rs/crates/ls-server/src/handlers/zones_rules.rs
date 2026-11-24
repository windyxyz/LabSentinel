// zones + rules + rule-templates (incl. apply).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use ls_core::{AnchorMode, ZoneSpatial};
use serde_json::{json, Value};

pub(crate) async fn camera_lab_rules(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
) -> Response {
    with_store(&st, |store| {
        if store.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "监控源不存在").into_response());
        }
        Ok(Json(store.lab_rules(cid)?).into_response())
    })
}

pub(crate) async fn save_camera_lab_rules(
    State(st): State<Shared>,
    AxPath(cid): AxPath<i64>,
    Json(body): Json<ls_core::lab::LabRules>,
) -> Response {
    with_store(&st, |store| match store.replace_lab_rules(cid, &body) {
        Ok(()) => {
            let revision = st.workers.notify_policy_changed();
            Ok(Json(json!({"ok": true, "policy_revision": revision})).into_response())
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            Ok(detail_err(StatusCode::NOT_FOUND, "监控源不存在").into_response())
        }
        Err(rusqlite::Error::InvalidParameterName(message)) => {
            Ok(detail_err(StatusCode::UNPROCESSABLE_ENTITY, &message).into_response())
        }
        Err(error) => Err(error),
    })
}

pub(crate) fn zone_json(zone: &ls_store::StoredZone) -> Value {
    let mut result = json!(zone.spatial);
    result["id"] = json!(zone.id);
    result["camera_id"] = json!(zone.camera_id);
    result["name"] = json!(zone.name);
    result["polygon"] = serde_json::from_str(&zone.polygon).unwrap_or_else(|_| json!(zone.polygon));
    result
}

#[derive(Default, serde::Deserialize)]
pub(crate) struct ZonePatch {
    anchor_mode: Option<AnchorMode>,
    margin_px: Option<u32>,
    leave_grace_frames: Option<u32>,
}

impl ZonePatch {
    pub(crate) fn parse(body: &Value) -> Result<Self, &'static str> {
        let patch: Self = serde_json::from_value(body.clone())
            .map_err(|_| "请输入有效的区域锚点、边界像素和离场帧数")?;
        if patch.margin_px.is_some_and(|v| v > 128) {
            return Err("边界宽容须在 0 至 128 像素内");
        }
        if patch.leave_grace_frames.is_some_and(|v| v > 120) {
            return Err("离场宽限须在 0 至 120 帧内");
        }
        Ok(patch)
    }

    pub(crate) fn apply(self, defaults: ZoneSpatial) -> ZoneSpatial {
        ZoneSpatial {
            anchor_mode: self.anchor_mode.unwrap_or(defaults.anchor_mode),
            margin_px: self.margin_px.unwrap_or(defaults.margin_px),
            leave_grace_frames: self
                .leave_grace_frames
                .unwrap_or(defaults.leave_grace_frames),
        }
    }
}

pub(crate) fn valid_zone_polygon(polygon: &Value) -> bool {
    polygon.as_array().is_some_and(|vertices| {
        (3..=256).contains(&vertices.len())
            && vertices.iter().all(|vertex| {
                vertex.as_array().is_some_and(|xy| {
                    xy.len() == 2
                        && xy
                            .iter()
                            .all(|v| v.as_i64().is_some_and(|p| i32::try_from(p).is_ok()))
                })
            })
    })
}

pub(crate) async fn zones_list(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let rows: Vec<Value> = s.zones_spatial(cid)?.iter().map(zone_json).collect();
        Ok(Json(rows))
    })
}

pub(crate) async fn add_zone(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let cid = body.get("camera_id").and_then(|v| v.as_i64()).unwrap_or(-1);
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let polygon = body.get("polygon").cloned().unwrap_or(Value::Null);
    if !valid_zone_polygon(&polygon) || name.is_empty() {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "请输入区域名称和 3 至 256 个整数坐标顶点",
        )
        .into_response();
    }
    let spatial = match ZonePatch::parse(&body) {
        Ok(patch) => patch.apply(ZoneSpatial::NEW_ZONE),
        Err(error) => return detail_err(StatusCode::UNPROCESSABLE_ENTITY, error).into_response(),
    };
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        let polygon = polygon.to_string();
        let zid = s.add_zone_spatial(cid, &name, &polygon, spatial)?;
        let revision = st.workers.notify_policy_changed();
        let mut response = zone_json(&ls_store::StoredZone {
            id: zid,
            camera_id: cid,
            name,
            polygon,
            spatial,
        });
        response["policy_revision"] = json!(revision);
        Ok(Json(response).into_response())
    })
}

pub(crate) async fn delete_zone(State(st): State<Shared>, AxPath(zid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_zone(zid)? > 0 {
            let revision = st.workers.notify_policy_changed();
            Ok(Json(json!({"ok": true, "policy_revision": revision})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "zone not found").into_response())
        }
    })
}

/// LS-052：PUT /api/zones/:zid——拖拽顶点后保存多边形（≥3 顶点）。
pub(crate) async fn update_zone(
    State(st): State<Shared>,
    AxPath(zid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let polygon = body.get("polygon");
    if polygon.is_some_and(|value| !valid_zone_polygon(value)) {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "polygon 必须为 3 至 256 个整数坐标顶点的数组",
        )
        .into_response();
    }
    let patch = match ZonePatch::parse(&body) {
        Ok(patch) => patch,
        Err(error) => return detail_err(StatusCode::UNPROCESSABLE_ENTITY, error).into_response(),
    };
    if polygon.is_none()
        && patch.anchor_mode.is_none()
        && patch.margin_px.is_none()
        && patch.leave_grace_frames.is_none()
    {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "请选择要修改的区域设置")
            .into_response();
    }
    let polygon_json = polygon.map(Value::to_string);
    with_store(&st, |s| {
        if s.update_zone_spatial(
            zid,
            polygon_json.as_deref(),
            patch.anchor_mode,
            patch.margin_px,
            patch.leave_grace_frames,
        )? > 0
        {
            let revision = st.workers.notify_policy_changed();
            Ok(Json(
                json!({"ok": true, "id": zid, "polygon": polygon, "policy_revision": revision}),
            )
            .into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "zone not found").into_response())
        }
    })
}

pub(crate) fn rule_json(t: &ls_store::RuleFullRow) -> Value {
    json!({"id": t.0, "camera_id": t.1, "zone_id": t.2, "behavior": t.3,
           "level": t.4, "cooldown_s": t.5, "time_start": t.6, "time_end": t.7,
           "threshold": t.8, "enabled": t.9 != 0, "days_of_week": t.10,
           "holiday_only": t.11, "grace_s": t.12, "warn_ratio": t.13})
}

pub(super) fn parse_rule_grace(body: &Value) -> Result<(u32, f64), &'static str> {
    let seconds = match body.get("grace_s") {
        Some(value) => value
            .as_u64()
            .filter(|value| *value <= 3600)
            .ok_or("宽限必须是 0 至 3600 秒的整数")? as u32,
        None => 0,
    };
    let ratio = match body.get("warn_ratio") {
        Some(value) => value
            .as_f64()
            .filter(|value| (0.01..=0.99).contains(value))
            .ok_or("提前提示比例须在 1% 至 99%")?,
        None => 0.70,
    };
    Ok((seconds, ratio))
}

pub(crate) async fn rules_list(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let all = s.rules_all()?;
        let rows: Vec<Value> = all
            .iter()
            .filter(|r| cid.map(|c| r.1 == c).unwrap_or(true))
            .map(rule_json)
            .collect();
        Ok(Json(rows))
    })
}

pub(crate) async fn add_rule(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let cid = body.get("camera_id").and_then(|v| v.as_i64()).unwrap_or(-1);
    let behavior = body
        .get("behavior")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if behavior.is_empty() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "behavior 必填").into_response();
    }
    let (grace, ratio) = match parse_rule_grace(&body) {
        Ok(settings) => settings,
        Err(error) => return detail_err(StatusCode::UNPROCESSABLE_ENTITY, error).into_response(),
    };
    if grace > 0 && !matches!(behavior.as_str(), "enter_no_ppe" | "no_mask") {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "此规则不支持穿戴宽限")
            .into_response();
    }
    with_store(&st, |s| {
        let Some(camera) = s.camera(cid)? else {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        };
        if behavior == "person_fall" && camera.type_ == "folder" {
            return Ok(
                detail_err(StatusCode::UNPROCESSABLE_ENTITY, "跌倒检测需要连续视频画面")
                    .into_response(),
            );
        }
        if behavior == "camera_tamper" && !matches!(camera.type_.as_str(), "rtsp" | "usb") {
            return Ok(detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "画面健康告警需要实时摄像头",
            )
            .into_response());
        }
        let zone_id = (!matches!(behavior.as_str(), "thermal_overheat" | "camera_tamper"))
            .then(|| body.get("zone_id").and_then(|v| v.as_i64()))
            .flatten();
        let tx = s.conn().unchecked_transaction()?;
        let rid = s.add_rule_with_schedule(
            cid,
            zone_id,
            &behavior,
            body.get("level").and_then(|v| v.as_i64()).unwrap_or(2),
            body.get("cooldown_s").and_then(|v| v.as_i64()).unwrap_or(0),
            body.get("time_start").and_then(|v| v.as_str()),
            body.get("time_end").and_then(|v| v.as_str()),
            body.get("threshold").and_then(|v| v.as_i64()).unwrap_or(0),
            body.get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            body.get("days_of_week").and_then(|v| v.as_str()),
            body.get("holiday_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        )?;
        s.set_rule_grace(rid, grace, ratio)?;
        tx.commit()?;
        let revision = st.workers.notify_policy_changed();
        Ok(Json(
            json!({"id": rid, "camera_id": cid, "behavior": behavior, "policy_revision": revision}),
        )
        .into_response())
    })
}

pub(crate) async fn update_rule_grace(
    State(st): State<Shared>,
    AxPath(rid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let (grace, ratio) = match parse_rule_grace(&body) {
        Ok(settings) if body.get("grace_s").is_some() => settings,
        Ok(_) => {
            return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "请输入宽限秒数").into_response()
        }
        Err(error) => return detail_err(StatusCode::UNPROCESSABLE_ENTITY, error).into_response(),
    };
    with_store(&st, |s| {
        if s.set_rule_grace(rid, grace, ratio)? == 0 {
            return Ok(detail_err(StatusCode::NOT_FOUND, "找不到支持宽限的规则").into_response());
        }
        let revision = st.workers.notify_policy_changed();
        Ok(Json(json!({"ok": true, "policy_revision": revision})).into_response())
    })
}

pub(crate) async fn delete_rule(State(st): State<Shared>, AxPath(rid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_rule(rid)? > 0 {
            let revision = st.workers.notify_policy_changed();
            Ok(Json(json!({"ok": true, "policy_revision": revision})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "rule not found").into_response())
        }
    })
}

pub(crate) async fn templates_list(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.rule_templates()?;
        Ok(Json(
            rows.iter()
                .map(|t| {
                    json!({"id": t.id, "name": t.name,
                            "config": serde_json::from_str::<Value>(&t.config_json)
                                .unwrap_or(Value::String(t.config_json.clone())),
                            "created_ts": t.created_ts, "updated_ts": t.updated_ts})
                })
                .collect::<Vec<_>>(),
        ))
    })
}

pub(crate) async fn add_template(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // LS-042：兼容前端传 `{name, rules: [...]}` 与 `{name, config: [...]}` 两种形态
    let config = body
        .get("config")
        .cloned()
        .or_else(|| body.get("rules").cloned());
    if name.is_empty() || config.is_none() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "name 与 config 必填").into_response();
    }
    with_store(&st, |s| {
        match s.add_rule_template(&name, &config.unwrap().to_string()) {
            Ok(id) => Ok(Json(json!({"id": id, "name": name})).into_response()),
            Err(_) => Ok(detail_err(StatusCode::CONFLICT, "模板名已存在").into_response()),
        }
    })
}

pub(crate) async fn update_template(
    State(st): State<Shared>,
    AxPath(tid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let config = body.get("config").cloned().unwrap_or(Value::Null);
    with_store(&st, |s| {
        let n = s.update_rule_template(tid, &name, &config.to_string())?;
        if n > 0 {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "template not found").into_response())
        }
    })
}

pub(crate) async fn delete_template(
    State(st): State<Shared>,
    AxPath(tid): AxPath<i64>,
) -> Response {
    with_store(&st, |s| {
        if s.delete_rule_template(tid)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "template not found").into_response())
        }
    })
}

// ---- 导出 / 指标 / 诊断 / meta ----

pub(crate) async fn apply_rule_template(
    State(st): State<Shared>,
    AxPath(tid): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let cam_ids: Vec<i64> = body
        .get("camera_ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default();
    let zone_id: Option<i64> = body.get("zone_id").and_then(|v| v.as_i64());
    with_store(&st, |s| {
        let Some(tpl) = s.get_rule_template(tid)? else {
            return Ok(detail_err(StatusCode::NOT_FOUND, "template not found").into_response());
        };
        let parsed: Value =
            serde_json::from_str(&tpl.config_json).unwrap_or(Value::Array(Vec::new()));
        // LS-042：兼容数组形态 `[...]` 与包裹对象 `{rules: [...]}`
        let Some(rules) = parsed
            .as_array()
            .or_else(|| parsed.get("rules").and_then(|v| v.as_array()))
        else {
            return Ok(detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "模板 config_json 不是数组",
            )
            .into_response());
        };
        // 先校验全部目标（Python 同语义：任一摄像头缺失/区域不归属即 4xx，不半写）
        for cid in &cam_ids {
            if s.camera(*cid)?.is_none() {
                return Ok((
                    StatusCode::NOT_FOUND,
                    Json(json!({"detail": format!("camera {cid} not found")})),
                )
                    .into_response());
            }
            if let Some(zid) = zone_id {
                match s.zone(zid)? {
                    Some(z) if z.1 != *cid => {
                        return Ok((
                            StatusCode::BAD_REQUEST,
                            Json(json!({"detail": format!("区域 {zid} 不属于摄像头 #{cid}")})),
                        )
                            .into_response());
                    }
                    Some(_) => {}
                    None => {
                        return Ok((
                            StatusCode::NOT_FOUND,
                            Json(json!({"detail": format!("zone {zid} not found")})),
                        )
                            .into_response());
                    }
                }
            }
        }
        let mut created = 0i64;
        let mut skipped = 0i64;
        let tx = s.conn().unchecked_transaction()?;
        for cid in &cam_ids {
            for r in rules {
                let behavior = r.get("behavior").and_then(|v| v.as_str()).unwrap_or("");
                let t_start = r.get("time_start").and_then(|v| v.as_str());
                let t_end = r.get("time_end").and_then(|v| v.as_str());
                let days = r.get("days_of_week").and_then(|v| v.as_str());
                let holiday = r
                    .get("holiday_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let dup = s.rules_all()?.iter().any(|x| {
                    x.1 == *cid
                        && x.2 == zone_id
                        && x.3 == behavior
                        && x.6.as_deref() == t_start
                        && x.7.as_deref() == t_end
                        && x.10.as_deref() == days
                        && x.11 == holiday
                });
                if dup {
                    skipped += 1;
                    continue;
                }
                let (grace, ratio) = match parse_rule_grace(r) {
                    Ok(settings) => settings,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let rid = s.add_rule_with_schedule(
                    *cid,
                    zone_id,
                    behavior,
                    r.get("level").and_then(|v| v.as_i64()).unwrap_or(2),
                    r.get("cooldown_s").and_then(|v| v.as_i64()).unwrap_or(0),
                    t_start,
                    t_end,
                    r.get("threshold").and_then(|v| v.as_i64()).unwrap_or(0),
                    r.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    days,
                    holiday,
                )?;
                s.set_rule_grace(rid, grace, ratio)?;
                created += 1;
            }
        }
        tx.commit()?;
        let revision = (created > 0).then(|| st.workers.notify_policy_changed());
        Ok(Json(json!({"ok": true, "created": created, "skipped": skipped, "policy_revision": revision})).into_response())
    })
}
