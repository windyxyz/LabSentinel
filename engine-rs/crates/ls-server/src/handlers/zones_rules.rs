// zones + rules + rule-templates (incl. apply).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

pub(crate) fn zone_json(t: &(i64, i64, String, String)) -> Value {
    let polygon = serde_json::from_str::<Value>(&t.3).unwrap_or(Value::String(t.3.clone()));
    json!({"id": t.0, "camera_id": t.1, "name": t.2, "polygon": polygon})
}

pub(crate) async fn zones_list(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let all = s.zones_all()?;
        let rows: Vec<Value> = all
            .iter()
            .filter(|z| cid.map(|c| z.1 == c).unwrap_or(true))
            .map(zone_json)
            .collect();
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
    if !polygon.is_array() || name.is_empty() {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "name 与 polygon(数组) 必填",
        )
        .into_response();
    }
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        let zid = s.add_zone(cid, &name, &polygon.to_string())?;
        Ok(
            Json(json!({"id": zid, "camera_id": cid, "name": name, "polygon": polygon}))
                .into_response(),
        )
    })
}

pub(crate) async fn delete_zone(State(st): State<Shared>, AxPath(zid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_zone(zid)? > 0 {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "zone not found").into_response())
        }
    })
}

#[allow(clippy::type_complexity)]
pub(crate) fn rule_json(
    t: &(
        i64,
        i64,
        Option<i64>,
        String,
        i64,
        i64,
        Option<String>,
        Option<String>,
        i64,
        i64,
        Option<String>, // days_of_week（RA1.1 补列，与 Python store.rules 同口径）
        bool, // holiday_only（RA4.2 补列，节假日限定规则）
    ),
) -> Value {
    json!({"id": t.0, "camera_id": t.1, "zone_id": t.2, "behavior": t.3,
           "level": t.4, "cooldown_s": t.5, "time_start": t.6, "time_end": t.7,
           "threshold": t.8, "enabled": t.9 != 0, "days_of_week": t.10,
           "holiday_only": t.11})
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
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok(detail_err(StatusCode::NOT_FOUND, "camera not found").into_response());
        }
        let rid = s.add_rule(
            cid,
            body.get("zone_id").and_then(|v| v.as_i64()),
            &behavior,
            body.get("level").and_then(|v| v.as_i64()).unwrap_or(2),
            body.get("cooldown_s").and_then(|v| v.as_i64()).unwrap_or(0),
            body.get("time_start").and_then(|v| v.as_str()),
            body.get("time_end").and_then(|v| v.as_str()),
            body.get("threshold").and_then(|v| v.as_i64()).unwrap_or(0),
            body.get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        )?;
        Ok(Json(json!({"id": rid, "camera_id": cid, "behavior": behavior})).into_response())
    })
}

pub(crate) async fn delete_rule(State(st): State<Shared>, AxPath(rid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_rule(rid)? > 0 {
            Ok(Json(json!({"ok": true})).into_response())
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
    let config = body.get("config").cloned();
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
        let Some(rules) = parsed.as_array() else {
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
        for cid in &cam_ids {
            for r in rules {
                let behavior = r.get("behavior").and_then(|v| v.as_str()).unwrap_or("");
                let t_start = r.get("time_start").and_then(|v| v.as_str());
                let t_end = r.get("time_end").and_then(|v| v.as_str());
                let dup = s.rules_all()?.iter().any(|x| {
                    x.1 == *cid
                        && x.2 == zone_id
                        && x.3 == behavior
                        && x.6 == t_start.map(String::from)
                        && x.7 == t_end.map(String::from)
                });
                if dup {
                    skipped += 1;
                    continue;
                }
                s.add_rule(
                    *cid,
                    zone_id,
                    behavior,
                    r.get("level").and_then(|v| v.as_i64()).unwrap_or(2),
                    r.get("cooldown_s").and_then(|v| v.as_i64()).unwrap_or(0),
                    t_start,
                    t_end,
                    r.get("threshold").and_then(|v| v.as_i64()).unwrap_or(0),
                    r.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                )?;
                created += 1;
            }
        }
        Ok(Json(json!({"ok": true, "created": created, "skipped": skipped})).into_response())
    })
}
