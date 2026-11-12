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

/// 静态资源缓存策略（兑现文件头「缓存策略」注释，2026-11-12）：
/// `/assets/*`（Vite hash 文件名，内容不可变）→ `immutable, max-age=31536000`；
/// index.html 与 SPA 回退 → `no-cache`（每次协商校验，保证发版即生效）。
/// 只对 2xx 加头（404/回退失败不缓存）。
pub(crate) async fn static_cache_policy(req: axum::extract::Request, next: Next) -> Response {
    let is_asset = req.uri().path().starts_with("/assets/");
    let mut resp = next.run(req).await;
    if resp.status().is_success() {
        let v = if is_asset {
            HeaderValue::from_static("public, max-age=31536000, immutable")
        } else {
            HeaderValue::from_static("no-cache")
        };
        resp.headers_mut().insert(header::CACHE_CONTROL, v);
    }
    resp
}

// ---- 中间件：Host/CSRF/鉴权（读保护+设备令牌）/安全头/request-id/审计 ----

pub(crate) async fn guard_middleware(
    State(st): State<Shared>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let headers = req.headers().clone();
    // Host 白名单（防 DNS Rebinding）
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().unwrap_or("").split(':').next())
        .unwrap_or("")
        .to_string();
    if !path.starts_with("/ws")
        && !st
            .trusted_hosts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(&host))
        && !(st.testing && host == "testserver")
    {
        return detail_err(StatusCode::BAD_REQUEST, "host not allowed").into_response();
    }
    let unsafe_m = method == axum::http::Method::POST
        || method == axum::http::Method::PUT
        || method == axum::http::Method::PATCH
        || method == axum::http::Method::DELETE;
    let mut actor = String::new();
    if unsafe_m && path.starts_with("/api/") {
        // CSRF/Fetch-Metadata（写请求跨站拒绝）
        let sfs = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok());
        let same = match sfs {
            Some("same-origin") | Some("none") => true,
            Some(_) => false,
            None => {
                let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
                match origin {
                    None => true,
                    Some(o) => {
                        o.split_once("://").map(|(_, h)| h == host).unwrap_or(false)
                            || st.allowed_origins.iter().any(|a| a == o)
                    }
                }
            }
        };
        if !same {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"detail": "跨站请求被拒绝", "code": "csrf_blocked"})),
            )
                .into_response();
        }
        // 管理令牌（写操作；设备令牌可处置事件但不可改配置——T-620）。
        // 豁免：/api/pairing/claim 在 Python 侧无鉴权守卫（公开配对入口，
        // 由配对码本身与限流保护）——契约 1:1 对齐。
        let provided = provided_token(&headers);
        if path == "/api/pairing/claim" {
            actor = "anonymous".into();
        } else {
            match check_write_token(&st, provided.as_deref()) {
                Ok(a) => actor = a,
                Err(resp) => return *resp,
            }
        }
    } else if (method == axum::http::Method::GET || method == axum::http::Method::HEAD)
        && path.starts_with("/api/")
        && st.read_protected
        && !path.starts_with("/api/health/")
        && path != "/api/v1/meta"
    {
        // R2-103 对齐：读保护时 admin/read/设备令牌均可读
        let provided = provided_token(&headers);
        if !check_read_token(&st, provided.as_deref()) {
            return detail_err(
                StatusCode::UNAUTHORIZED,
                "需要管理令牌（X-Admin-Token 或 Authorization: Bearer）",
            )
            .into_response();
        }
        actor = "read".into();
    }
    let rid = uuid_like();
    let mut resp = next.run(req).await;
    // 审计（任务87/R2-503 对齐：写 /api/* 记录；失败不影响响应；不含 body/headers）。
    // await 而非 detach：tokio 的 spawn_blocking 异步等待不阻塞执行器，且保证
    // 响应返回前审计行已落库（测试/观察确定性；SQLite 插入亚毫秒）。
    if unsafe_m && path.starts_with("/api/") {
        let status = resp.status().as_u16() as i64;
        let action = format!("{method} {path}");
        let target = extract_target(&path);
        let st2 = Arc::clone(&st);
        let rid2 = rid.clone();
        let actor2 = actor.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(s) = ls_store::Store::open(&st2.db_path) {
                let _ = s.audit_add(
                    &iso_now(),
                    &rid2,
                    &actor2,
                    &action,
                    target.as_deref(),
                    status,
                );
            }
        })
        .await;
    }
    let hs = resp.headers_mut();
    hs.insert("x-request-id", HeaderValue::from_str(&rid).unwrap());
    hs.entry("x-content-type-options")
        .or_insert(HeaderValue::from_static("nosniff"));
    hs.entry("x-frame-options")
        .or_insert(HeaderValue::from_static("DENY"));
    hs.entry("referrer-policy")
        .or_insert(HeaderValue::from_static("no-referrer"));
    resp
}

/// 令牌来源：X-Admin-Token 优先，其次 Authorization: Bearer（R2-103 移动端惯例）
pub(crate) fn provided_token(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-admin-token").and_then(|v| v.to_str().ok()) {
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())?;
    auth.strip_prefix("Bearer ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 设备令牌校验（sha256 存库 + 60s touch 节流）；返回设备名。
pub(crate) fn check_device_token(st: &Shared, token: &str) -> Option<String> {
    if token.is_empty() {
        return None;
    }
    let hash = sha256_hex(token.as_bytes());
    let s = ls_store::Store::open(&st.db_path).ok()?;
    let dev = s.device_by_hash(&hash).ok()??;
    if dev.revoked {
        return None;
    }
    if let Ok(mut ps) = st.pair_state.lock() {
        let now = std::time::Instant::now();
        let need = ps
            .touch
            .get(&dev.id)
            .map(|t| now.duration_since(*t).as_secs() >= 60)
            .unwrap_or(true);
        if need {
            let _ = s.touch_device(dev.id);
            ps.touch.insert(dev.id, now);
        }
    }
    Some(dev.name)
}

/// 写守卫：返回 (actor) 或 401/403 响应（与 require_admin/require_triage 同语义）。
/// Err 用 Box 装箱（result_large_err：axum Response 体较大）。
pub(crate) fn check_write_token(st: &Shared, provided: Option<&str>) -> Result<String, Box<Response>> {
    if st.admin_token.is_empty() {
        return Ok("anonymous(auth-off)".into());
    }
    let provided = provided.unwrap_or("");
    if provided == st.admin_token {
        return Ok("admin".into());
    }
    if !st.read_token.is_empty() && provided == st.read_token {
        return Err(Box::new(
            detail_err(
                StatusCode::UNAUTHORIZED,
                "当前令牌无写权限（只读令牌）——请使用管理令牌",
            )
            .into_response(),
        ));
    }
    if let Some(name) = check_device_token(st, provided) {
        return Ok(format!("device:{name}"));
    }
    Err(Box::new(
        detail_err(
            StatusCode::UNAUTHORIZED,
            "需要管理令牌（X-Admin-Token 或 Authorization: Bearer）",
        )
        .into_response(),
    ))
}

/// 读守卫：admin/read/设备令牌均可（R2-103）
pub(crate) fn check_read_token(st: &Shared, provided: Option<&str>) -> bool {
    if st.admin_token.is_empty() {
        return true;
    }
    let provided = provided.unwrap_or("");
    if provided == st.admin_token {
        return true;
    }
    if !st.read_token.is_empty() && provided == st.read_token {
        return true;
    }
    check_device_token(st, provided).is_some()
}

/// Python `_audit_write` 的 target 提取：/cameras/3 → "camera:3" 等
pub(crate) fn extract_target(path: &str) -> Option<String> {
    let kind = if path.contains("/cameras/") {
        "camera"
    } else if path.contains("/events/") {
        "event"
    } else if path.contains("/zones/") {
        "zone"
    } else if path.contains("/rules/") {
        "rule"
    } else if path.contains("/alert-silences/") {
        "silence"
    } else {
        return None;
    };
    let id = path.rsplit('/').next()?.trim();
    // 安全复核（Mimosa medium「疑似跨文件污点」）：审计 resource 只保留纯数字 id
    // （URL 中的其余部分——含任何注入字符——都会在此被整体丢弃），随后的 format!
    // 只拼 kind（上表硬编码枚举）与该数字串，无日志注入/结构污染面。
    if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() {
        Some(format!("{kind}:{id}"))
    } else {
        None
    }
}

