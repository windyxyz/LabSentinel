// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Json, Response};
use serde_json::json;
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

#[derive(Clone)]
pub(crate) struct RequestActor(pub String);

pub(crate) async fn guard_middleware(
    State(st): State<Shared>,
    mut req: axum::extract::Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let headers = req.headers().clone();
    let authority = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<axum::http::uri::Authority>().ok());
    let host = authority
        .as_ref()
        .map(|a| a.host().trim_matches(['[', ']']))
        .unwrap_or("");
    // LS-044：/ws/events 同样校验 Host 白名单（防 DNS Rebinding / CSWSH）
    if !st
        .trusted_hosts
        .iter()
        .any(|h| h.eq_ignore_ascii_case(host))
        && !(st.testing && host == "testserver")
    {
        return detail_err(StatusCode::BAD_REQUEST, "host not allowed").into_response();
    }
    let unsafe_m = method == axum::http::Method::POST
        || method == axum::http::Method::PUT
        || method == axum::http::Method::PATCH
        || method == axum::http::Method::DELETE;
    let is_api = path.starts_with("/api/");
    let is_socket = path.starts_with("/ws/");
    let provided = if is_socket {
        ws_token(&headers)
    } else {
        provided_token(&headers)
    };
    let mut browser = st.access.request(&req, host);
    let password_required = st.access.required(browser.client);
    let share = provided
        .as_deref()
        .is_some_and(|token| share_device(&st, token));
    browser.share = share;
    if share && (unsafe_m || !share_read_path(&path)) {
        return detail_err(StatusCode::FORBIDDEN, "分享令牌仅可访问值守大屏").into_response();
    }
    if share
        && path != "/api/share-links/verify"
        && !provided
            .as_deref()
            .is_some_and(|token| valid_read_credential(&st, token))
    {
        return detail_err(StatusCode::UNAUTHORIZED, "分享链接已失效").into_response();
    }
    let public_read = !unsafe_m
        && (path.starts_with("/api/health/")
            || matches!(
                path.as_str(),
                "/api/access/status" | "/api/v1/meta" | "/api/share-links/verify"
            ));
    let public_write = method == axum::http::Method::POST
        && matches!(
            path.as_str(),
            "/api/access/login"
                | "/api/access/logout"
                | "/api/access/recover"
                | "/api/pairing/claim"
                | "/api/pairing/logout"
        );
    if (is_api || is_socket) && !share {
        if let Some(digest) = crate::access::cookie_digest(&headers, crate::pairing::DEVICE_COOKIE)
        {
            browser.session = None;
            if browser.client.can_use_password() {
                let shared = st.clone();
                browser.device = tokio::task::spawn_blocking(move || {
                    crate::pairing::paired_device(&shared, &digest)
                })
                .await
                .unwrap_or_default();
            }
            browser.device_expired = browser.device.is_none();
        }
    }
    let device_session = browser.device.is_some();
    if browser.device_expired
        && !public_read
        && !(method == axum::http::Method::POST
            && matches!(path.as_str(), "/api/pairing/claim" | "/api/pairing/logout"))
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "detail": "设备配对已过期或被撤销", "code": "device_expired",
            })),
        )
            .into_response();
    }
    if let Some(device) = &browser.device {
        if !crate::pairing::permit_device(&st, device, &method, &path) {
            return detail_err(StatusCode::FORBIDDEN, "该设备无权访问此资源").into_response();
        }
    }
    let session_admin = provided.is_none() && browser.session.is_some();
    let api_admin = !device_session && is_api_admin(&st, provided.as_deref());
    if password_required
        && (is_api || is_socket)
        && !public_read
        && !public_write
        && !device_session
        && !session_admin
        && !provided
            .as_deref()
            .is_some_and(|token| valid_read_credential(&st, token))
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "detail": "请登录后继续", "code": "password_required",
            })),
        )
            .into_response();
    }
    if (is_socket || (unsafe_m && is_api))
        && !same_origin(
            &headers,
            authority.as_ref(),
            &st.allowed_origins,
            (session_admin || device_session || path.starts_with("/api/access/"))
                .then_some(browser.client.secure),
        )
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "detail": "跨站请求被拒绝", "code": "csrf_blocked",
            })),
        )
            .into_response();
    }
    if is_socket
        && (st.read_protected || share)
        && !session_admin
        && !device_session
        && !check_read_token(&st, provided.as_deref())
    {
        return detail_err(StatusCode::UNAUTHORIZED, "实时数据需要读取凭据").into_response();
    }
    const ADMIN_GETS: &[&str] = &[
        "/api/diagnostics/bundle.zip",
        "/api/dataset",
        "/api/system/network",
        "/api/system/autostart",
        "/api/reports/schedule",
        "/api/share-links",
        "/api/access/settings",
        "/api/devices",
        "/api/alert-deliveries",
        "/api/face/config",
        "/api/face/persons",
        "/api/face/library",
        "/api/face/events",
        "/api/review/suppressions",
    ];
    const PROTECTED_ADMIN_GETS: &[&str] = &[
        "/api/audit",
        "/api/config/summary",
        "/api/config/export",
        "/api/alerts/config",
        "/api/detect/config",
        "/api/detect/profiles",
        "/api/logs",
        "/api/backups/status",
        "/api/maintenance",
        "/api/plugins",
    ];
    let matches_path = |p: &&str| path == *p || path.starts_with(&format!("{p}/"));
    let event_notes = path
        .strip_prefix("/api/events/")
        .and_then(|tail| tail.strip_suffix("/notes"))
        .is_some_and(|id| id.parse::<i64>().is_ok());
    let admin_get = ADMIN_GETS.iter().any(matches_path)
        || (path.starts_with("/api/cameras/") && path.ends_with("/spatial"))
        || (event_notes && !device_session)
        || (st.read_protected && PROTECTED_ADMIN_GETS.iter().any(matches_path));
    if !unsafe_m
        && admin_get
        && !public_read
        && (password_required || !st.admin_token.is_empty() || provided.is_some())
        && !session_admin
        && !api_admin
    {
        return detail_err(StatusCode::UNAUTHORIZED, "该端点需要管理令牌").into_response();
    }
    let mut actor = String::new();
    if unsafe_m && is_api {
        static RATE: std::sync::Mutex<
            Option<std::collections::HashMap<String, Vec<std::time::Instant>>>,
        > = std::sync::Mutex::new(None);
        let ip = browser
            .client
            .ip
            .map_or_else(|| host.to_owned(), |ip| ip.to_string());
        let (limit, window) = if path == "/api/pairing/claim" {
            (10usize, 60u64)
        } else {
            (120, 60)
        };
        let bucket = if path == "/api/pairing/claim" {
            format!("{ip}|claim")
        } else {
            ip
        };
        let now = std::time::Instant::now();
        let mut allowed = true;
        if !st.testing {
            if let Ok(mut m) = RATE.lock() {
                let m = m.get_or_insert_with(std::collections::HashMap::new);
                // LS-044：桶数上限清理，防止内存无界增长
                if m.len() > 2048 {
                    m.retain(|_, v| {
                        v.retain(|t| now.duration_since(*t).as_secs() < 60);
                        !v.is_empty()
                    });
                }
                let e = m.entry(bucket).or_default();
                e.retain(|t| now.duration_since(*t).as_secs() < window);
                if e.len() >= limit {
                    allowed = false;
                } else {
                    e.push(now);
                }
                if allowed && path == "/api/pairing/claim" {
                    let all_claims = m.entry("pairing-claims".into()).or_default();
                    all_claims.retain(|t| now.saturating_duration_since(*t).as_secs() < 60);
                    allowed = all_claims.len() < 50;
                    if allowed {
                        all_claims.push(now);
                    }
                }
            }
        }
        if !allowed {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"detail": "请求过于频繁，稍后再试", "code": "rate_limited"})),
            )
                .into_response();
        }
        if let Some(device) = &browser.device {
            actor = format!("device:{}", device.name);
        } else if public_write && !session_admin {
            actor = "anonymous".into();
        } else if session_admin {
            actor = "admin".into();
        } else if password_required && !api_admin {
            match check_write_token(&st, provided.as_deref(), &path) {
                Ok(a) if is_triage_path(&path) && a.starts_with("device:") => actor = a,
                _ => {
                    return detail_err(StatusCode::FORBIDDEN, "此操作需要管理员权限")
                        .into_response()
                }
            }
        } else {
            match check_write_token(&st, provided.as_deref(), &path) {
                Ok(a) => actor = a,
                Err(resp) => return *resp,
            }
        }
    } else if (method == axum::http::Method::GET || method == axum::http::Method::HEAD)
        && is_api
        && st.read_protected
        && !public_read
        && !session_admin
        && !device_session
    {
        // R2-103 对齐：读保护时 admin/read/设备令牌均可读
        if !check_read_token(&st, provided.as_deref())
            && (password_required
                || !crate::handlers::misc_status::valid_media_signature(&st, req.uri()))
        {
            return detail_err(
                StatusCode::UNAUTHORIZED,
                "需要管理令牌（X-Admin-Token 或 Authorization: Bearer）",
            )
            .into_response();
        }
        // P2-4 / LS-044：管理级 GET 端点（含配置导出/密钥配置/日志/备份/维护）——read 令牌不够
        actor = "read".into();
    }
    let rid = uuid_like();
    req.extensions_mut().insert(browser);
    if unsafe_m && is_api {
        req.extensions_mut().insert(RequestActor(actor.clone()));
    }
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
    if !public_read && is_api && (password_required || session_admin || device_session) {
        hs.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    hs.insert("x-request-id", HeaderValue::from_str(&rid).unwrap());
    hs.entry("x-content-type-options")
        .or_insert(HeaderValue::from_static("nosniff"));
    hs.entry("x-frame-options")
        .or_insert(HeaderValue::from_static("DENY"));
    hs.entry("referrer-policy")
        .or_insert(HeaderValue::from_static("no-referrer"));
    resp
}

fn same_origin(
    headers: &axum::http::HeaderMap,
    authority: Option<&axum::http::uri::Authority>,
    allowed: &[String],
    secure: Option<bool>,
) -> bool {
    if headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|site| !matches!(site, "same-origin" | "none"))
    {
        return false;
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if secure.is_none() && allowed.iter().any(|value| origin == value) {
        return true;
    }
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    let Some(scheme) = uri.scheme_str().filter(|s| matches!(*s, "http" | "https")) else {
        return false;
    };
    if secure.is_some_and(|s| s != (scheme == "https")) {
        return false;
    }
    let Some(from) = uri.authority() else {
        return false;
    };
    let Some(to) = authority else {
        return false;
    };
    let default_port = if scheme == "https" { 443 } else { 80 };
    from.host().eq_ignore_ascii_case(to.host())
        && from.port_u16().unwrap_or(default_port) == to.port_u16().unwrap_or(default_port)
}

fn share_read_path(path: &str) -> bool {
    matches!(
        path,
        "/api/share-links/verify"
            | "/api/cameras"
            | "/api/stats/daily"
            | "/api/stats/compliance"
            | "/api/events"
            | "/ws/events"
    ) || path
        .strip_prefix("/api/cameras/")
        .and_then(|tail| tail.split_once('/'))
        .is_some_and(|(id, op)| id.parse::<i64>().is_ok() && matches!(op, "snapshot" | "webrtc"))
        || path
            .strip_prefix("/ws/webrtc/")
            .is_some_and(|id| id.parse::<i64>().is_ok())
        || path
            .strip_prefix("/ws/detections/")
            .is_some_and(|id| id.parse::<i64>().is_ok())
}

pub(crate) fn share_device(st: &Shared, token: &str) -> bool {
    if !st.admin_token.is_empty() && ct_eq(token, &st.admin_token) {
        return false;
    }
    ls_store::Store::open(&st.db_path)
        .ok()
        .and_then(|s| {
            s.device_by_hash(&sha256_hex(token.as_bytes()))
                .ok()
                .flatten()
        })
        .is_some_and(|d| d.platform == "share")
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

pub(crate) fn ws_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .and_then(|protocols| {
            protocols.split(',').find_map(|protocol| {
                protocol
                    .trim()
                    .strip_prefix("bearer.")
                    .filter(|value| !value.is_empty())
            })
        })
        .map(str::to_owned)
        .or_else(|| provided_token(headers))
}

pub(crate) fn is_api_admin(st: &Shared, provided: Option<&str>) -> bool {
    !st.admin_token.is_empty() && provided.is_some_and(|value| ct_eq(value, &st.admin_token))
}

pub(crate) fn is_api_reader(st: &Shared, provided: Option<&str>) -> bool {
    is_api_admin(st, provided)
        || (!st.read_token.is_empty() && provided.is_some_and(|value| ct_eq(value, &st.read_token)))
}

/// 设备令牌校验（sha256 存库 + 60s touch 节流）；返回设备名。
pub(crate) fn check_device_token(st: &Shared, token: &str) -> Option<String> {
    if token.is_empty() {
        return None;
    }
    let hash = sha256_hex(token.as_bytes());
    let s = ls_store::Store::open(&st.db_path).ok()?;
    let dev = s.device_by_hash(&hash).ok()??;
    if dev.revoked || dev.policy_json.is_some() {
        return None;
    }
    // LS-054：分享链接令牌额外校验 TTL（过期/吊销即失效）
    if dev.platform == "share"
        && !s
            .share_link_valid(&hash, &ls_core::time::local_now_iso())
            .unwrap_or(false)
    {
        return None;
    }
    touch_device(st, &s, dev.id);
    Some(dev.name)
}

pub(crate) fn touch_device(st: &Shared, store: &ls_store::Store, id: i64) {
    if let Ok(mut ps) = st.pair_state.lock() {
        let now = std::time::Instant::now();
        let need = ps
            .touch
            .get(&id)
            .map(|t| now.duration_since(*t).as_secs() >= 60)
            .unwrap_or(true);
        if need {
            let _ = store.touch_device(id);
            ps.touch.insert(id, now);
        }
    }
}

/// P2-2（安全审计）：常数时间令牌比较（双 SHA256 后定长比较——长度恒 32，
/// 无早退分支；对齐 Python hmac.compare_digest）。
pub(crate) fn ct_eq(a: &str, b: &str) -> bool {
    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;
    bool::from(Sha256::digest(a.as_bytes()).ct_eq(&Sha256::digest(b.as_bytes())))
}

/// P1-1（安全审计）：triage 端点白名单——设备令牌仅允许处置单事件
/// （POST /api/events/{id}/status）；bulk-status/config/plugins/shutdown 等
/// 写操作一律要求管理令牌（对齐 Python require_triage 的端点声明）。
fn is_triage_path(path: &str) -> bool {
    match path.strip_prefix("/api/events/") {
        Some(tail) => match tail.strip_suffix("/status") {
            Some(id) => id.parse::<i64>().is_ok(),
            None => false,
        },
        None => false,
    }
}

/// 写守卫：返回 (actor) 或 401/403 响应（与 require_admin/require_triage 同语义）。
/// Err 用 Box 装箱（result_large_err：axum Response 体较大）。
pub(crate) fn check_write_token(
    st: &Shared,
    provided: Option<&str>,
    path: &str,
) -> Result<String, Box<Response>> {
    let provided = provided.unwrap_or("");
    if is_api_admin(st, Some(provided)) {
        return Ok("admin".into());
    }
    if !st.read_token.is_empty() && ct_eq(provided, &st.read_token) {
        return Err(Box::new(
            detail_err(
                StatusCode::UNAUTHORIZED,
                "当前令牌无写权限（只读令牌）——请使用管理令牌",
            )
            .into_response(),
        ));
    }
    if let Some(name) = check_device_token(st, provided) {
        // P1-1：设备令牌仅限 triage（处置单事件）——否则权限放大
        // （shutdown/config import/plugins start 均可被配对设备触发）
        if is_triage_path(path) {
            return Ok(format!("device:{name}"));
        }
        return Err(Box::new(
            detail_err(
                StatusCode::FORBIDDEN,
                "设备令牌仅可处置事件（triage）——管理操作请使用管理令牌",
            )
            .into_response(),
        ));
    }
    if st.admin_token.is_empty() {
        return Ok("anonymous(auth-off)".into());
    }
    Err(Box::new(
        detail_err(
            StatusCode::UNAUTHORIZED,
            "需要管理令牌（X-Admin-Token 或 Authorization: Bearer）",
        )
        .into_response(),
    ))
}

/// 读守卫：admin/read/设备令牌均可（R2-103）；P2-2 常数时间比较
pub(crate) fn check_read_token(st: &Shared, provided: Option<&str>) -> bool {
    st.admin_token.is_empty() || provided.is_some_and(|token| valid_read_credential(st, token))
}

fn valid_read_credential(st: &Shared, provided: &str) -> bool {
    is_api_reader(st, Some(provided)) || check_device_token(st, provided).is_some()
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
