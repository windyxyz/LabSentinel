// config summary/export/validate/import.
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use crate::{discovery, fernet, pdf};
use axum::extract::{
    ws::{WebSocket, WebSocketUpgrade},
    Path as AxPath, Query, State,
};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) async fn config_summary(State(st): State<Shared>) -> Json<Value> {
    // 不回显敏感值（T-109 等价）
    Json(
        json!({"version": st.version, "auth_read_protected": st.read_protected,
                "admin_token": {"configured": !st.admin_token.is_empty()},
                "webhook": {"configured": false}}),
    )
}

pub(crate) async fn config_export(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let with_secrets = q.get("with_secrets").map(|v| v == "1").unwrap_or(false);
    with_store(&st, |s| {
        let cams = s.cameras()?;
        let cname: std::collections::HashMap<i64, String> =
            cams.iter().map(|c| (c.id, c.name.clone())).collect();
        let zones = s.zones_all()?;
        let rules = s.rules_all()?;
        let cameras: Vec<Value> = cams
            .iter()
            .map(|c| {
                let src = if with_secrets {
                    c.source.clone()
                } else {
                    mask_rtsp_creds(&c.source)
                };
                json!({"name": c.name, "source": src, "type": c.type_,
                       "group_name": c.group_name})
            })
            .collect();
        let zones_out: Vec<Value> = zones
            .iter()
            .map(|z| {
                let polygon =
                    serde_json::from_str::<Value>(&z.3).unwrap_or(Value::String(z.3.clone()));
                json!({"camera_name": cname.get(&z.1).cloned().unwrap_or(format!("#{}", z.1)),
                       "name": z.2, "polygon": polygon})
            })
            .collect();
        let rules_out: Vec<Value> = rules
            .iter()
            .map(|r| {
                json!({"camera_name": cname.get(&r.1).cloned().unwrap_or(format!("#{}", r.1)),
                       "zone_id": r.2, "behavior": r.3, "level": r.4, "cooldown_s": r.5,
                       "time_start": r.6, "time_end": r.7, "threshold": r.8,
                       "enabled": r.9 != 0})
            })
            .collect();
        Ok(Json(json!({"version": st.version, "cameras": cameras,
                       "zones": zones_out, "rules": rules_out}))
        .into_response())
    })
}

pub(crate) async fn config_validate(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let cams = s.cameras()?;
        let mut issues: Vec<Value> = Vec::new();
        for c in &cams {
            if c.source.is_empty() {
                issues.push(json!({"kind": "camera", "id": c.id, "name": c.name,
                                    "message": "source 为空"}));
            }
        }
        // 无区域的摄像头提示（配置完整性，非错误）
        let zones = s.zones_all()?;
        let with_zones: std::collections::HashSet<i64> = zones.iter().map(|z| z.1).collect();
        for c in &cams {
            if !with_zones.contains(&c.id) {
                issues.push(json!({"kind": "camera", "id": c.id, "name": c.name,
                                    "message": "尚无区域（全画幅规则仍可用）"}));
            }
        }
        Ok(Json(json!({"ok": true, "cameras": cams.len(),
                       "zones": zones.len(), "issues": issues})))
    })
}

pub(crate) async fn config_import(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let mode = body
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("merge")
        .to_string();
    if mode != "merge" && mode != "replace" {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "mode 必须是 merge|replace",
        )
        .into_response();
    }
    if mode == "replace"
        && !body
            .get("confirm_replace")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return detail_err(StatusCode::BAD_REQUEST, "replace 模式必须 confirm_replace=true（将清空现有摄像头/区域/规则；事件保留但失去对应摄像头）")
            .into_response();
    }
    let data = body.get("data").cloned().unwrap_or(Value::Null);
    let cams = data
        .get("cameras")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let zones = data
        .get("zones")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let rules = data
        .get("rules")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if cams.is_empty() && zones.is_empty() && rules.is_empty() {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "data 至少包含 cameras/zones/rules 之一",
        )
        .into_response();
    }
    with_store(&st, |s| {
        if mode == "replace" {
            s.clear_config()?;
        }
        let mut created = (0i64, 0i64, 0i64);
        let mut skipped_cam: Vec<String> = Vec::new();
        let mut cam_id_by_name: std::collections::HashMap<String, i64> =
            s.cameras()?.into_iter().map(|c| (c.name, c.id)).collect();
        // cameras：merge 按名称去重
        for c in &cams {
            let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let source = c.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let type_ = c.get("type").and_then(|v| v.as_str()).unwrap_or("file");
            if name.is_empty() {
                skipped_cam.push("<unnamed>".into());
                continue;
            }
            if cam_id_by_name.contains_key(name) {
                skipped_cam.push(name.to_string());
                continue;
            }
            let group = c.get("group_name").and_then(|v| v.as_str()).unwrap_or("");
            let cid = s.add_camera_full(name, source, type_, group)?;
            cam_id_by_name.insert(name.to_string(), cid);
            created.0 += 1;
        }
        // zones：按 camera_name 映射
        let mut zone_id_by_name: std::collections::HashMap<(i64, String), i64> = s
            .zones_all()?
            .into_iter()
            .map(|z| ((z.1, z.2), z.0))
            .collect();
        let mut skipped_zone: Vec<String> = Vec::new();
        for z in &zones {
            let cname = z.get("camera_name").and_then(|v| v.as_str()).unwrap_or("");
            let name = z.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let Some(cid) = cam_id_by_name.get(cname).copied() else {
                skipped_zone.push(format!("{cname}/{name}"));
                continue;
            };
            let polygon = z.get("polygon").cloned().unwrap_or(Value::Null);
            if !polygon.is_array() || name.is_empty() {
                skipped_zone.push(format!("{cname}/{name}"));
                continue;
            }
            let key = (cid, name.to_string());
            if zone_id_by_name.contains_key(&key) {
                skipped_zone.push(format!("{cname}/{name}"));
                continue;
            }
            let zid = s.add_zone(cid, name, &polygon.to_string())?;
            zone_id_by_name.insert(key, zid);
            created.1 += 1;
        }
        // rules：camera_name + zone_name（可选）映射；zone 缺失则跳过
        let mut skipped_rule: Vec<String> = Vec::new();
        for r in &rules {
            let cname = r.get("camera_name").and_then(|v| v.as_str()).unwrap_or("");
            let behavior = r.get("behavior").and_then(|v| v.as_str()).unwrap_or("");
            let Some(cid) = cam_id_by_name.get(cname).copied() else {
                skipped_rule.push(format!("{cname}/{behavior}"));
                continue;
            };
            let zone_id = match r.get("zone_name").and_then(|v| v.as_str()) {
                Some(zname) if !zname.is_empty() => {
                    match zone_id_by_name.get(&(cid, zname.to_string())) {
                        Some(zid) => Some(*zid),
                        None => {
                            skipped_rule.push(format!("{cname}/{behavior}（区域缺失）"));
                            continue;
                        }
                    }
                }
                _ => None,
            };
            s.add_rule(
                cid,
                zone_id,
                behavior,
                r.get("level").and_then(|v| v.as_i64()).unwrap_or(2),
                r.get("cooldown_s").and_then(|v| v.as_i64()).unwrap_or(0),
                r.get("time_start").and_then(|v| v.as_str()),
                r.get("time_end").and_then(|v| v.as_str()),
                r.get("threshold").and_then(|v| v.as_i64()).unwrap_or(0),
                r.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
            )?;
            created.2 += 1;
        }
        Ok(Json(json!({"ok": true, "mode": mode,
                       "created": {"cameras": created.0, "zones": created.1, "rules": created.2},
                       "skipped": {"cameras": skipped_cam, "zones": skipped_zone,
                                    "rules": skipped_rule}}))
        .into_response())
    })
}

// ---- 插件占位（rust 独立模式无插件管理器：列表恒空 → 任何 pid 不存在，
// 与 Python 对未知 pid 的 404 行为一致，不是假桩而是真实结果） ----

