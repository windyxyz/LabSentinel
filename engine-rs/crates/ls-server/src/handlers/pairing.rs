// pairing & device tokens (T-620).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{Path as AxPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

pub(crate) async fn create_pairing(State(st): State<Shared>) -> Response {
    // 6 位数字码（5 分钟有效；写守卫已在中间件完成）
    let mut buf = [0u8; 4];
    let _ = getrandom::getrandom(&mut buf);
    let n = u32::from_le_bytes(buf) % 1_000_000;
    let code = format!("{n:06}");
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        + 300;
    if let Ok(mut ps) = st.pair_state.lock() {
        ps.pending.retain(|_, e| {
            *e > std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });
        ps.pending.insert(code.clone(), exp);
    }
    Json(json!({"code": code, "expires_in_s": 300,
                "claim": "POST /api/pairing/claim {code, device_name, platform}"}))
    .into_response()
}

pub(crate) async fn claim_pairing(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let code = body
        .get("code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let name = body
        .get("device_name")
        .and_then(|v| v.as_str())
        .unwrap_or("device")
        .chars()
        .take(64)
        .collect::<String>();
    let platform = body
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .take(32)
        .collect::<String>();
    if code.is_empty() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "code 必填").into_response();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let valid = {
        let mut ps = match st.pair_state.lock() {
            Ok(p) => p,
            Err(_) => {
                return server_err();
            }
        };
        let exp = ps.pending.remove(&code);
        match exp {
            Some(e) if e >= now => {
                ps.failures = 0;
                true
            }
            _ => {
                ps.failures += 1;
                if ps.failures >= 5 {
                    // R2-110：5 次失败作废全部待配对码
                    ps.pending.clear();
                    ps.failures = 0;
                }
                false
            }
        }
    };
    if !valid {
        return detail_err(StatusCode::BAD_REQUEST, "配对码无效或已过期").into_response();
    }
    let token = random_token();
    let hash = sha256_hex(token.as_bytes());
    with_store(&st, |s| {
        s.add_device(&name, &platform, &hash)?;
        Ok(
            Json(json!({"token": token, "note": "令牌仅显示一次，请保存到安全存储"}))
                .into_response(),
        )
    })
}

pub(crate) async fn list_devices(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.devices()?;
        Ok(Json(
            rows.iter()
                .map(|d| {
                    json!({"id": d.id, "name": d.name, "platform": d.platform,
                            "created_ts": d.created_ts, "last_seen_ts": d.last_seen_ts,
                            "revoked": d.revoked})
                })
                .collect::<Vec<_>>(),
        ))
    })
}

pub(crate) async fn revoke_device(State(st): State<Shared>, AxPath(did): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.revoke_device(did)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "device not found").into_response())
        }
    })
}

// ---- 审计/投递/静默/日历 ----
