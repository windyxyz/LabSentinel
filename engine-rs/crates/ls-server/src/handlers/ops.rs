// backup/maintenance/escalation (ls-jobs delegation).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) async fn backups_run(State(st): State<Shared>) -> Response {
    if !st.jobs.trigger_backup() {
        return detail_err(StatusCode::CONFLICT, "备份正在进行中").into_response();
    }
    Json(json!({"started": true})).into_response()
}

pub(crate) async fn backups_status(State(st): State<Shared>) -> Json<Value> {
    Json(st.jobs.backup_status())
}

pub(crate) async fn maintenance_status(State(st): State<Shared>) -> Response {
    let jobs = Arc::clone(&st.jobs);
    match tokio::task::spawn_blocking(move || jobs.maintenance_status()).await {
        Ok(Ok(v)) => Json(v).into_response(),
        _ => server_err(),
    }
}

pub(crate) async fn maintenance_preview(State(st): State<Shared>) -> Response {
    let jobs = Arc::clone(&st.jobs);
    match tokio::task::spawn_blocking(move || jobs.preview_cleanup()).await {
        Ok(Ok(v)) => Json(v).into_response(),
        _ => server_err(),
    }
}

pub(crate) async fn maintenance_cleanup(State(st): State<Shared>) -> Response {
    let jobs = Arc::clone(&st.jobs);
    match tokio::task::spawn_blocking(move || jobs.run_cleanup()).await {
        Ok(Ok(stats)) => match serde_json::to_value(stats) {
            Ok(v) => Json(v).into_response(),
            Err(_) => server_err(),
        },
        _ => server_err(),
    }
}

pub(crate) async fn escalation_status(State(st): State<Shared>) -> Response {
    match st.jobs.escalation_status() {
        Ok(v) => Json(v).into_response(),
        Err(_) => server_err(),
    }
}

// ==== R2-309 批次 1 新增 handlers（契约 1:1 对齐 Python） ====

pub(crate) async fn audit_list(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let offset: i64 = q.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let action = q.get("action").map(|s| s.as_str());
    let actor = q.get("actor").map(|s| s.as_str());
    with_store(&st, |s| {
        let (rows, total) = s.audit_list(limit, offset, action, actor)?;
        Ok(Json(json!({"items": rows.iter().map(|r| json!({
                "id": r.id, "ts": r.ts, "request_id": r.request_id, "actor": r.actor,
                "action": r.action, "target": r.target, "status_code": r.status_code}))
            .collect::<Vec<_>>(), "total": total})))
    })
}

pub(crate) async fn deliveries_list(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let status = q.get("status").map(|s| s.as_str());
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    with_store(&st, |s| {
        let rows = s.deliveries(status, limit)?;
        Ok(Json(
            rows.iter()
                .map(|d| {
                    json!({"id": d.id, "event_id": d.event_id, "channel": d.channel,
                            "status": d.status, "attempts": d.attempts,
                            "last_error": d.last_error, "updated_ts": d.updated_ts})
                })
                .collect::<Vec<_>>(),
        ))
    })
}

pub(crate) async fn retry_delivery(State(st): State<Shared>, AxPath(did): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.retry_delivery(did)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "delivery not found").into_response())
        }
    })
}

pub(crate) async fn silences_list(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.silences()?;
        Ok(Json(
            rows.iter()
                .map(|x| {
                    json!({"id": x.id, "scope": x.scope, "target": x.target,
                            "reason": x.reason, "created_ts": x.created_ts,
                            "until_ts": x.until_ts})
                })
                .collect::<Vec<_>>(),
        ))
    })
}

pub(crate) async fn add_silence(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let scope = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if scope.is_empty() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "scope 必填").into_response();
    }
    let target = body.get("target").and_then(|v| v.as_str());
    let reason = body.get("reason").and_then(|v| v.as_str());
    let until = body
        .get("until_ts")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if until.is_empty() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "until_ts 必填").into_response();
    }
    with_store(&st, |s| {
        let sid = s.add_silence(&scope, target, reason, &until)?;
        Ok(Json(json!({"id": sid})).into_response())
    })
}

pub(crate) async fn delete_silence(State(st): State<Shared>, AxPath(sid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_silence(sid)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "silence not found").into_response())
        }
    })
}

pub(crate) async fn calendar_list(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.calendar_exceptions()?;
        Ok(Json(
            rows.iter()
                .map(|c| json!({"date": c.date, "mode": c.mode, "note": c.note}))
                .collect::<Vec<_>>(),
        ))
    })
}

pub(crate) async fn set_calendar(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let date = body
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mode = body
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let note = body
        .get("note")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if date.is_empty() || (mode != "workday" && mode != "holiday") {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "date 必填；mode 必须是 workday|holiday",
        )
        .into_response();
    }
    with_store(&st, |s| {
        s.set_calendar_exception(&date, &mode, &note)?;
        Ok(Json(json!({"date": date, "mode": mode, "note": note})).into_response())
    })
}

pub(crate) async fn delete_calendar(
    State(st): State<Shared>,
    AxPath(d): AxPath<String>,
) -> Response {
    with_store(&st, |s| {
        if s.delete_calendar_exception(&d)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "calendar exception not found").into_response())
        }
    })
}

// ---- 规则模板（任务116） ----
