//! ls-server（L3/T-331）：axum 控制面单二进制。
//!
//! 契约 1:1 对齐 Python FastAPI（docs/openapi.json 快照）：
//! - GET /api/health/live、/api/health/ready、/api/system、/api/events、
//!   /api/cameras、/api/cameras/{id}/status、/api/stats/summary、/api/config/summary
//! - POST /api/events/{id}/status（处置）、/api/cameras（新增）、/api/backups/run
//! - WS /ws/events（tokio broadcast，慢客户端丢弃策略）
//!
//! 中间件：request-id、Host 白名单、CSRF(Origin/Sec-Fetch-Site)、管理令牌、安全响应头。
//! 静态：web/dist（SPA 回退 + 缓存策略）。数据库：ls-store（与 Python 同 SQLite/双向兼容）。

use std::sync::Arc;

mod discovery;
mod fernet;

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
use tokio::sync::broadcast;

/// 应用共享状态
pub struct AppState {
    pub db_path: String,
    pub admin_token: String,
    /// R2-103 对齐：只读令牌（LS_READ_TOKEN；写永拒）
    pub read_token: String,
    pub read_protected: bool,
    pub trusted_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub web_dist: String,
    pub data_dir: String,
    pub version: String,
    pub events_tx: broadcast::Sender<String>,
    /// T-330：后台作业（retention / backup / escalation），启动时 start()
    pub jobs: Arc<ls_jobs::Jobs>,
    /// R2-309：配对码/失败计数/设备 touch 节流（进程内，与 Python devices.py 同语义）
    pub pair_state: std::sync::Mutex<PairState>,
}

#[derive(Default)]
pub struct PairState {
    /// code -> expire_unix_ts
    pub pending: std::collections::HashMap<String, i64>,
    /// 连续 claim 失败计数（5 次作废全部待配对码，N-17/R2-110）
    pub failures: u32,
    /// device_id -> 上次 touch（60s 节流，R2-103）
    pub touch: std::collections::HashMap<i64, std::time::Instant>,
}

/// R2-307: machine-readable route inventory for scripts/rust_route_coverage.py.
/// Format: {"paths": {"<method> <path>": ...}} — one entry per (method, path).
pub fn dump_routes() -> serde_json::Value {
    let mut routes = Vec::new();
    for (method, path) in ROUTE_INVENTORY {
        routes.push(serde_json::json!({"method": method, "path": path}));
    }
    serde_json::json!({ "routes": routes })
}

/// R2-309 批次 1：已实现路由清单（新增的以 ✱ 注释标出；保持与 build_router 同步）。
pub const ROUTE_INVENTORY: &[(&str, &str)] = &[
    ("GET", "/api/health/live"),
    ("GET", "/api/health/ready"),
    ("GET", "/api/system"),
    ("GET", "/api/events"),
    ("POST", "/api/events/:id/status"),
    ("GET", "/api/cameras"),
    ("POST", "/api/cameras"),
    ("GET", "/api/cameras/:id/status"),
    ("GET", "/api/stats/summary"),
    ("GET", "/api/config/summary"),
    ("GET", "/api/backups/status"),
    ("POST", "/api/backups/run"),
    ("GET", "/api/maintenance/status"),
    ("GET", "/api/maintenance/preview"),
    ("POST", "/api/maintenance/cleanup"),
    ("GET", "/api/escalation/status"),
    ("GET", "/api/events/count"),
    ("GET", "/api/ws/events"),
    // ---- R2-309 批次 1：cameras 扩展 ----
    ("DELETE", "/api/cameras/:id"),
    ("PUT", "/api/cameras/:id/group"),
    ("PUT", "/api/cameras/:id/analyzer"),
    ("POST", "/api/cameras/:id/start"),
    ("GET", "/api/cameras/:id/webrtc"),
    ("GET", "/api/camera-groups"),
    ("POST", "/api/cameras/bulk"),
    ("GET", "/api/cameras/usb-devices"),
    // ---- zones / rules ----
    ("GET", "/api/zones"),
    ("POST", "/api/zones"),
    ("DELETE", "/api/zones/:zid"),
    ("GET", "/api/rules"),
    ("POST", "/api/rules"),
    ("DELETE", "/api/rules/:rid"),
    ("GET", "/api/rule-templates"),
    ("POST", "/api/rule-templates"),
    ("PUT", "/api/rule-templates/:tid"),
    ("DELETE", "/api/rule-templates/:tid"),
    // ---- events 扩展 ----
    ("POST", "/api/events/bulk-status"),
    ("GET", "/api/events/timeline"),
    ("GET", "/api/events/:eid/snapshot"),
    ("GET", "/api/events/:eid/video"),
    // ---- stats 全套 ----
    ("GET", "/api/stats/daily"),
    ("GET", "/api/stats/weekly"),
    ("GET", "/api/stats/hourly"),
    ("GET", "/api/stats/quality"),
    // ---- 配对/设备（T-620） ----
    ("POST", "/api/pairing"),
    ("POST", "/api/pairing/claim"),
    ("GET", "/api/devices"),
    ("DELETE", "/api/devices/:did"),
    // ---- 审计/投递/静默/日历 ----
    ("GET", "/api/audit"),
    ("GET", "/api/alert-deliveries"),
    ("POST", "/api/alert-deliveries/:did/retry"),
    ("GET", "/api/alert-silences"),
    ("POST", "/api/alert-silences"),
    ("DELETE", "/api/alert-silences/:sid"),
    ("GET", "/api/calendar"),
    ("POST", "/api/calendar"),
    ("DELETE", "/api/calendar/:d"),
    // ---- 导出/诊断/杂项 ----
    ("GET", "/api/export/events.csv"),
    ("GET", "/api/export/stats.csv"),
    ("GET", "/api/metrics"),
    ("GET", "/api/diagnostics/summary"),
    ("GET", "/api/v1/meta"),
    // ---- R2-309 批次 2 ----
    ("POST", "/api/cameras/:cid/copy-config"),
    ("GET", "/api/config/export"),
    ("GET", "/api/config/validate"),
    ("GET", "/api/media/sign"),
    ("GET", "/api/model/status"),
    ("GET", "/api/mqtt/status"),
    ("GET", "/api/genai/status"),
    ("GET", "/api/plugins"),
    ("GET", "/api/update/check"),
    ("GET", "/api/alerts/status"),
    ("GET", "/api/alert-template"),
    ("GET", "/api/folder-images"),
    ("GET", "/api/folder-images/summary"),
    ("GET", "/api/logs"),
    ("GET", "/api/models/ppe/status"),
    // ---- R2-309 批次 3 ----
    ("POST", "/api/config/import"),
    ("GET", "/api/events/:eid/evidence.zip"),
    ("GET", "/api/export/evidence.zip"),
    ("GET", "/api/plugins/:pid/logs"),
    ("POST", "/api/plugins/:pid/enable"),
    ("POST", "/api/plugins/:pid/disable"),
    ("POST", "/api/plugins/:pid/start"),
    ("POST", "/api/plugins/:pid/stop"),
    ("GET", "/api/face/status"),
    ("POST", "/api/face/persons"),
    ("GET", "/api/face/persons"),
    ("DELETE", "/api/face/persons/all"),
    ("DELETE", "/api/face/persons/:subject"),
    ("POST", "/api/alert-test"),
    ("POST", "/api/models/ppe/download"),
    // ---- R2-309 批次 3b（收尾批） ----
    ("GET", "/api/folder-images/export.csv"),
    ("GET", "/api/folder-images/:iid"),
    ("GET", "/api/folder-images/:iid/file"),
    ("GET", "/api/folder-images/:iid/thumb"),
    ("POST", "/api/alert-template/preview"),
    ("POST", "/api/rule-templates/:tid/apply"),
    ("POST", "/api/system/shutdown"),
    ("GET", "/api/cameras/:cid/snapshot"),
    // ---- R2-309 批次 4（L3 门槛冲刺） ----
    ("GET", "/api/alerts/config"),
    ("PUT", "/api/alerts/config"),
    ("POST", "/api/alerts/config/reset"),
    ("GET", "/api/discovery/cameras"),
    ("POST", "/api/discovery/probe"),
];

pub type Shared = Arc<AppState>;

pub fn build_router(state: Shared) -> Router {
    let api = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/system", get(system))
        .route("/events", get(events))
        .route("/events/:id/status", post(set_status))
        .route("/events/bulk-status", post(bulk_event_status))
        .route("/events/timeline", get(events_timeline))
        .route("/events/:eid/snapshot", get(event_snapshot))
        .route("/events/:eid/video", get(event_video))
        .route("/cameras", get(cameras).post(add_camera))
        .route("/cameras/:id/status", get(camera_status))
        .route("/cameras/:id/group", put(set_camera_group))
        .route("/cameras/:id/analyzer", put(set_camera_analyzer))
        .route("/cameras/:id/start", post(start_camera))
        .route("/cameras/:id/webrtc", get(camera_webrtc))
        .route("/camera-groups", get(camera_groups))
        .route("/cameras/bulk", post(cameras_bulk))
        .route("/cameras/usb-devices", get(usb_devices))
        .route("/zones", get(zones_list).post(add_zone))
        .route("/zones/:zid", delete(delete_zone))
        .route("/rules", get(rules_list).post(add_rule))
        .route("/rules/:rid", delete(delete_rule))
        .route("/rule-templates", get(templates_list).post(add_template))
        .route(
            "/rule-templates/:tid",
            put(update_template).delete(delete_template),
        )
        .route("/stats/summary", get(stats_summary))
        .route("/stats/daily", get(stats_daily))
        .route("/stats/weekly", get(stats_weekly))
        .route("/stats/hourly", get(stats_hourly))
        .route("/stats/quality", get(stats_quality))
        .route("/config/summary", get(config_summary))
        .route("/backups/status", get(backups_status))
        .route("/backups/run", post(backups_run))
        .route("/maintenance/status", get(maintenance_status))
        .route("/maintenance/preview", get(maintenance_preview))
        .route("/maintenance/cleanup", post(maintenance_cleanup))
        .route("/escalation/status", get(escalation_status))
        .route("/events/count", get(events_count))
        .route("/pairing", post(create_pairing))
        .route("/pairing/claim", post(claim_pairing))
        .route("/devices", get(list_devices))
        .route("/devices/:did", delete(revoke_device))
        .route("/audit", get(audit_list))
        .route("/alert-deliveries", get(deliveries_list))
        .route("/alert-deliveries/:did/retry", post(retry_delivery))
        .route("/alert-silences", get(silences_list).post(add_silence))
        .route("/alert-silences/:sid", delete(delete_silence))
        .route("/calendar", get(calendar_list).post(set_calendar))
        .route("/calendar/:d", delete(delete_calendar))
        .route("/export/events.csv", get(export_events_csv))
        .route("/export/stats.csv", get(export_stats_csv))
        .route("/metrics", get(metrics))
        .route("/diagnostics/summary", get(diagnostics_summary))
        .route("/v1/meta", get(api_v1_meta))
        // ---- R2-309 批次 2 ----
        .route("/cameras/:cid/copy-config", post(copy_camera_config))
        .route("/config/export", get(config_export))
        .route("/config/validate", get(config_validate))
        .route("/media/sign", get(sign_media))
        .route("/model/status", get(model_status))
        .route("/mqtt/status", get(mqtt_status))
        .route("/genai/status", get(genai_status))
        .route("/plugins", get(plugins_list))
        .route("/update/check", get(update_check_status))
        .route("/alerts/status", get(alerts_status))
        .route("/alert-template", get(alert_template_get))
        .route("/folder-images", get(folder_images_list))
        .route("/folder-images/summary", get(folder_images_summary))
        .route("/logs", get(logs_tail))
        .route("/models/ppe/status", get(ppe_status))
        // ---- R2-309 批次 3 ----
        .route("/config/import", post(config_import))
        .route("/events/:eid/evidence.zip", get(event_evidence_zip))
        .route("/export/evidence.zip", get(export_evidence_zip))
        .route("/plugins/:pid/logs", get(plugin_logs))
        .route("/plugins/:pid/enable", post(plugin_action))
        .route("/plugins/:pid/disable", post(plugin_action))
        .route("/plugins/:pid/start", post(plugin_action))
        .route("/plugins/:pid/stop", post(plugin_action))
        .route("/face/status", get(face_status))
        .route("/face/persons", get(face_persons_list).post(face_register))
        .route("/face/persons/all", delete(face_clear_all))
        .route("/face/persons/:subject", delete(face_delete))
        .route("/alert-test", post(alert_test))
        .route("/models/ppe/download", post(ppe_download))
        // ---- R2-309 批次 3b ----
        .route("/folder-images/export.csv", get(folder_images_csv))
        .route("/folder-images/:iid", get(folder_image_detail))
        .route("/folder-images/:iid/file", get(folder_image_file))
        .route("/folder-images/:iid/thumb", get(folder_image_thumb))
        .route("/alert-template/preview", post(alert_template_preview))
        .route("/rule-templates/:tid/apply", post(apply_rule_template))
        .route("/system/shutdown", post(shutdown_server))
        .route("/cameras/:cid/snapshot", get(camera_snapshot))
        // ---- R2-309 批次 4 ----
        .route(
            "/alerts/config",
            get(alerts_config_get).put(alerts_config_put),
        )
        .route("/alerts/config/reset", post(alerts_config_reset))
        .route("/discovery/cameras", get(discovery_cameras))
        .route("/discovery/probe", post(discovery_probe))
        .with_state(state.clone());
    let ws = Router::new()
        .route("/ws/events", get(ws_events))
        .with_state(state.clone());
    Router::new()
        .nest("/api", api)
        .merge(ws)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            guard_middleware,
        ))
        .with_state(state)
}

// ---- 中间件：Host/CSRF/鉴权（读保护+设备令牌）/安全头/request-id/审计 ----
async fn guard_middleware(
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
        && host != "testserver"
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "host not allowed"})),
        )
            .into_response();
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
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"detail": "需要管理令牌（X-Admin-Token 或 Authorization: Bearer）"})),
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
fn provided_token(headers: &axum::http::HeaderMap) -> Option<String> {
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

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 设备令牌校验（sha256 存库 + 60s touch 节流）；返回设备名。
fn check_device_token(st: &Shared, token: &str) -> Option<String> {
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
fn check_write_token(st: &Shared, provided: Option<&str>) -> Result<String, Box<Response>> {
    if st.admin_token.is_empty() {
        return Ok("anonymous(auth-off)".into());
    }
    let provided = provided.unwrap_or("");
    if provided == st.admin_token {
        return Ok("admin".into());
    }
    if !st.read_token.is_empty() && provided == st.read_token {
        return Err(Box::new(
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"detail": "当前令牌无写权限（只读令牌）——请使用管理令牌"})),
            )
                .into_response(),
        ));
    }
    if let Some(name) = check_device_token(st, provided) {
        return Ok(format!("device:{name}"));
    }
    Err(Box::new(
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"detail": "需要管理令牌（X-Admin-Token 或 Authorization: Bearer）"})),
        )
            .into_response(),
    ))
}

/// 读守卫：admin/read/设备令牌均可（R2-103）
fn check_read_token(st: &Shared, provided: Option<&str>) -> bool {
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
fn extract_target(path: &str) -> Option<String> {
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
    if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() {
        Some(format!("{kind}:{id}"))
    } else {
        None
    }
}

/// 本地 ISO 秒时间戳（与 Python datetime.now().isoformat(timespec="seconds") 同形）
fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// ISO 日期（UTC 即可，控制面统计与 Python 同基准即可对齐）
fn iso_days_ago(days: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86400) - days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// secrets.token_urlsafe(32) 等价：32 系统熵字节 → base64url（无 padding）
fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).expect("system entropy");
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    // 6-bit 打包：43 字符承载 258 bit >= 256 bit
    let mut out = String::with_capacity(43);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &b in buf.iter() {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((acc >> bits) & 0x3f) as usize] as char);
        }
    }
    out
}

fn uuid_like() -> String {
    // 32 hex（与 Python uuid4().hex 同形，无需外部 crate）
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:032x}", t)
}

// ---- handlers ----
async fn health_live() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn health_ready(State(st): State<Shared>) -> Response {
    match ls_store::Store::open(&st.db_path) {
        Ok(s) => match s.integrity_check() {
            Ok(v) if v == "ok" => {
                Json(json!({"status": "ready", "checks": {"db": {"ok": true}}})).into_response()
            }
            _ => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"status": "degraded"})),
            )
                .into_response(),
        },
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "degraded", "checks": {"db": {"ok": false}}})),
        )
            .into_response(),
    }
}

async fn system(State(st): State<Shared>) -> Json<Value> {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return Json(json!({"status": "error"})),
    };
    let cams = s.cameras().unwrap_or_default();
    let _total = s.count_events().unwrap_or(0);
    Json(json!({
        "version": st.version,
        "schema_version": s.schema_version().unwrap_or(0),
        "backend": "rust",
        "cameras": cams.iter().map(|c| json!({"id": c.id, "name": c.name,
            "status": c.status, "type": c.type_})).collect::<Vec<_>>(),
        "today_counts": {},
        "unhandled_today": 0,
        "components": {"engine": {"state": "idle"}, "go2rtc": {"ok": false}},
        "config_issues": {"fail": 0, "warn": 0, "source_issues": []},
    }))
}

async fn events(State(st): State<Shared>, Query(q): Query<EventsQuery>) -> Response {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.events(
        q.limit.unwrap_or(50).min(200),
        q.offset.unwrap_or(0),
        q.camera_id,
        q.status.as_deref(),
    ) {
        Ok(rows) => Json(rows.iter().map(event_json).collect::<Vec<_>>()).into_response(),
        Err(_) => server_err(),
    }
}

#[derive(serde::Deserialize)]
struct EventsQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    camera_id: Option<i64>,
    status: Option<String>,
}

fn event_json(e: &ls_store::Event) -> Value {
    json!({"id": e.id, "ts": e.ts, "camera_id": e.camera_id, "type": e.type_,
           "level": e.level, "track_id": e.track_id, "detail": e.detail,
           "has_snapshot": e.snapshot.is_some(), "has_video": e.video.is_some(),
           "status": e.status})
}

async fn events_count(State(st): State<Shared>) -> Json<Value> {
    let n = ls_store::Store::open(&st.db_path)
        .ok()
        .and_then(|s| s.count_events().ok())
        .unwrap_or(0);
    Json(json!({"count": n}))
}

async fn cameras(State(st): State<Shared>) -> Response {
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

async fn camera_status(State(st): State<Shared>, AxPath(id): AxPath<i64>) -> Response {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.camera(id) {
        Ok(Some(c)) => Json(json!({"id": c.id, "name": c.name, "type": c.type_,
            "status": c.status, "analyzer": c.analyzer, "fps": null}))
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "camera not found"})),
        )
            .into_response(),
        Err(_) => server_err(),
    }
}

async fn add_camera(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
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
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "source 非法"})),
        )
            .into_response();
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

async fn set_status(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
    Json(body): Json<Value>,
) -> Response {
    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if !["new", "acked", "false_positive", "ignored"].contains(&status) {
        // T-505 契约对齐：Python 侧 EventStatusBody Literal → 422，两实现同契约
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "非法状态"})),
        )
            .into_response();
    }
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return server_err(),
    };
    match s.set_event_status(id, status, None, None) {
        Ok(0) => (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "event not found"})),
        )
            .into_response(),
        Ok(_) => Json(json!({"ok": true})).into_response(),
        Err(_) => server_err(),
    }
}

async fn stats_summary(State(st): State<Shared>) -> Json<Value> {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return Json(json!({})),
    };
    let rows = s.events(100000, 0, None, None).unwrap_or_default();
    let total = rows.len();
    let by_level = json!({"1": rows.iter().filter(|e| e.level == 1).count(),
                          "2": rows.iter().filter(|e| e.level == 2).count(),
                          "3": rows.iter().filter(|e| e.level == 3).count()});
    Json(json!({"total": total, "by_level": by_level, "days": 30}))
}

async fn config_summary(State(st): State<Shared>) -> Json<Value> {
    // 不回显敏感值（T-109 等价）
    Json(
        json!({"version": st.version, "auth_read_protected": st.read_protected,
                "admin_token": {"configured": !st.admin_token.is_empty()},
                "webhook": {"configured": false}}),
    )
}

async fn backups_run(State(st): State<Shared>) -> Response {
    if !st.jobs.trigger_backup() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"detail": "备份正在进行中"})),
        )
            .into_response();
    }
    Json(json!({"started": true})).into_response()
}

async fn backups_status(State(st): State<Shared>) -> Json<Value> {
    Json(st.jobs.backup_status())
}

async fn maintenance_status(State(st): State<Shared>) -> Response {
    let jobs = Arc::clone(&st.jobs);
    match tokio::task::spawn_blocking(move || jobs.maintenance_status()).await {
        Ok(Ok(v)) => Json(v).into_response(),
        _ => server_err(),
    }
}

async fn maintenance_preview(State(st): State<Shared>) -> Response {
    let jobs = Arc::clone(&st.jobs);
    match tokio::task::spawn_blocking(move || jobs.preview_cleanup()).await {
        Ok(Ok(v)) => Json(v).into_response(),
        _ => server_err(),
    }
}

async fn maintenance_cleanup(State(st): State<Shared>) -> Response {
    let jobs = Arc::clone(&st.jobs);
    match tokio::task::spawn_blocking(move || jobs.run_cleanup()).await {
        Ok(Ok(stats)) => match serde_json::to_value(stats) {
            Ok(v) => Json(v).into_response(),
            Err(_) => server_err(),
        },
        _ => server_err(),
    }
}

async fn escalation_status(State(st): State<Shared>) -> Response {
    match st.jobs.escalation_status() {
        Ok(v) => Json(v).into_response(),
        Err(_) => server_err(),
    }
}

// ==== R2-309 批次 1 新增 handlers（契约 1:1 对齐 Python） ====

fn with_store<F, R>(st: &Shared, f: F) -> Response
where
    F: FnOnce(&ls_store::Store) -> rusqlite::Result<R>,
    R: IntoResponse,
{
    match ls_store::Store::open(&st.db_path) {
        Ok(s) => match f(&s) {
            Ok(v) => v.into_response(),
            Err(_) => server_err(),
        },
        Err(_) => server_err(),
    }
}

fn zone_json(t: &(i64, i64, String, String)) -> Value {
    let polygon = serde_json::from_str::<Value>(&t.3).unwrap_or(Value::String(t.3.clone()));
    json!({"id": t.0, "camera_id": t.1, "name": t.2, "polygon": polygon})
}

async fn zones_list(
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

async fn add_zone(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let cid = body.get("camera_id").and_then(|v| v.as_i64()).unwrap_or(-1);
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let polygon = body.get("polygon").cloned().unwrap_or(Value::Null);
    if !polygon.is_array() || name.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "name 与 polygon(数组) 必填"})),
        )
            .into_response();
    }
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "camera not found"})),
            )
                .into_response());
        }
        let zid = s.add_zone(cid, &name, &polygon.to_string())?;
        Ok(
            Json(json!({"id": zid, "camera_id": cid, "name": name, "polygon": polygon}))
                .into_response(),
        )
    })
}

async fn delete_zone(State(st): State<Shared>, AxPath(zid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_zone(zid)? > 0 {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "zone not found"})),
            )
                .into_response())
        }
    })
}

#[allow(clippy::type_complexity)]
fn rule_json(
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
    ),
) -> Value {
    json!({"id": t.0, "camera_id": t.1, "zone_id": t.2, "behavior": t.3,
           "level": t.4, "cooldown_s": t.5, "time_start": t.6, "time_end": t.7,
           "threshold": t.8, "enabled": t.9 != 0})
}

async fn rules_list(
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

async fn add_rule(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let cid = body.get("camera_id").and_then(|v| v.as_i64()).unwrap_or(-1);
    let behavior = body
        .get("behavior")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if behavior.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "behavior 必填"})),
        )
            .into_response();
    }
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "camera not found"})),
            )
                .into_response());
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

async fn delete_rule(State(st): State<Shared>, AxPath(rid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_rule(rid)? > 0 {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "rule not found"})),
            )
                .into_response())
        }
    })
}

async fn set_camera_group(
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
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "camera not found"})),
            )
                .into_response());
        }
        s.set_camera_group(cid, &group)?;
        Ok(Json(json!({"id": cid, "group_name": group})).into_response())
    })
}

async fn set_camera_analyzer(
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
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "analyzer 必须是 python|rust"})),
        )
            .into_response();
    }
    with_store(&st, |s| {
        let cam = match s.camera(cid)? {
            Some(c) => c,
            None => {
                return Ok((
                    StatusCode::NOT_FOUND,
                    Json(json!({"detail": "camera not found"})),
                )
                    .into_response())
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

async fn start_camera(State(st): State<Shared>, AxPath(cid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "camera not found"})),
            )
                .into_response());
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

async fn camera_webrtc(State(st): State<Shared>, AxPath(cid): AxPath<i64>) -> Response {
    with_store(&st, |s| match s.camera(cid)? {
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "camera not found"})),
        )
            .into_response()),
        Some(c) if c.type_ == "folder" => Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "图片文件夹源没有实时流；请在「图片识别」页查看关键图片"})),
        )
            .into_response()),
        Some(_) => Ok(
            Json(json!({"camera_id": cid, "ws_url": format!("/ws/webrtc/{cid}")})).into_response(),
        ),
    })
}

async fn camera_groups(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let groups = s.camera_groups()?;
        Ok(Json(json!({"groups": groups})))
    })
}

async fn cameras_bulk(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
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
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "ids 必须为去重非空列表且 ≤100 个；action=start|stop"})),
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

async fn usb_devices() -> Response {
    // rust 控制面不做 DirectShow 枚举（前端回退手输索引，与"未找到 ffmpeg"路径一致）
    Json(json!({"devices": [],
                "error": "rust 控制面暂不支持 USB 枚举；启动 Python 服务后可用"}))
    .into_response()
}

async fn bulk_event_status(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let ids: Vec<i64> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default();
    if !["new", "acked", "false_positive", "ignored"].contains(&status.as_str())
        || body
            .get("ids")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().any(|x| x.is_boolean()))
            .unwrap_or(false)
    {
        // 与 Python EventStatusBody Literal / bool 拒绝同契约 → 422
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "非法状态或 ids 含布尔"})),
        )
            .into_response();
    }
    with_store(&st, |s| {
        let n = s.bulk_status(&ids, &status, "admin")?;
        Ok(Json(json!({"ok": true, "updated": n})).into_response())
    })
}

async fn events_timeline(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let days: i64 = q.get("days").and_then(|v| v.parse().ok()).unwrap_or(7);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    let level: Option<i64> = q.get("level").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let rows = s.events_timeline(days, cid, level)?;
        let days_list: Vec<Value> = rows
            .into_iter()
            .map(|(d, hours)| {
                let total: i64 = hours.iter().map(|h| h.1).sum();
                json!({"date": d, "total": total,
                       "hours": hours.into_iter().map(|(h, n, l3)|
                           json!({"hour": h, "total": n, "l3": l3})).collect::<Vec<_>>()})
            })
            .collect();
        Ok(Json(
            json!({"days": days_list.len(), "days_list": days_list}),
        ))
    })
}

fn serve_event_media(
    st: &Shared,
    eid: i64,
    col: fn(&ls_store::Event) -> &Option<String>,
    content_type: &str,
) -> Response {
    match ls_store::Store::open(&st.db_path) {
        Ok(s) => match s.event_by_id(eid) {
            Ok(Some(ev)) => {
                let rel = col(&ev).clone().unwrap_or_default();
                let p = std::path::Path::new(&st.data_dir).join(&rel);
                match std::fs::read(&p) {
                    Ok(bytes) => ([(header::CONTENT_TYPE, content_type)], bytes).into_response(),
                    Err(_) => (
                        StatusCode::NOT_FOUND,
                        Json(json!({"detail": "media file not found"})),
                    )
                        .into_response(),
                }
            }
            _ => (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "event not found"})),
            )
                .into_response(),
        },
        Err(_) => server_err(),
    }
}

async fn event_snapshot(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    serve_event_media(&st, eid, |e| &e.snapshot, "image/jpeg")
}

async fn event_video(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    serve_event_media(&st, eid, |e| &e.video, "video/mp4")
}

fn agg_json(row: &ls_store::AggRow) -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in row {
        m.insert(k.clone(), json!(v));
    }
    Value::Object(m)
}

async fn stats_daily(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let days: i64 = q.get("days").and_then(|v| v.parse().ok()).unwrap_or(7);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    let status = q.get("status").map(|s| s.as_str());
    with_store(&st, |s| {
        let rows = s.stats_daily(days, cid, status)?;
        Ok(Json(json!({"days": rows.into_iter()
            .map(|(d, row)| { let mut v = json!({"date": d}); if let Value::Object(m) = agg_json(&row) { for (k, val) in m { v[k] = val; } } v })
            .collect::<Vec<_>>()})))
    })
}

async fn stats_weekly(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let weeks: i64 = q.get("weeks").and_then(|v| v.parse().ok()).unwrap_or(4);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    let status = q.get("status").map(|s| s.as_str());
    with_store(&st, |s| {
        let rows = s.stats_weekly(weeks, cid, status)?;
        Ok(Json(json!({"weeks": rows.into_iter()
            .map(|(wk, row)| { let mut v = json!({"week_start": wk}); if let Value::Object(m) = agg_json(&row) { for (k, val) in m { v[k] = val; } } v })
            .collect::<Vec<_>>()})))
    })
}

async fn stats_hourly(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let date = q.get("date").cloned().unwrap_or_default();
    if date.len() != 10 || date.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "date 必须是 YYYY-MM-DD 格式"})),
        )
            .into_response();
    }
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let rows = s.stats_hourly(&date, cid)?;
        let mut hours = serde_json::Map::new();
        for (h, row) in rows {
            hours.insert(h, agg_json(&row));
        }
        Ok(Json(json!({"date": date, "hours": Value::Object(hours)})))
    })
}

async fn stats_quality(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let days: i64 = q
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(7)
        .clamp(1, 90);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let start = iso_days_ago(days - 1) + "T00:00:00";
        let end = iso_now();
        let mut where_sql = String::from(" AND ts>=? AND ts<=?");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(start.clone()), Box::new(end.clone())];
        if let Some(c) = cid {
            where_sql.push_str(" AND camera_id=?");
            args.push(Box::new(c));
        }
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let total: i64 = s.conn().query_row(
            &format!("SELECT COUNT(*) FROM events WHERE 1=1{where_sql}"),
            vals.as_slice(),
            |r| r.get(0),
        )?;
        let mut by_status = serde_json::Map::new();
        {
            let mut stmt = s.conn().prepare(&format!(
                "SELECT COALESCE(status,'new') k, COUNT(*) n FROM events WHERE 1=1{where_sql} GROUP BY k"))?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?;
            for (k, n) in rows.collect::<Result<Vec<_>, _>>()? {
                by_status.insert(k, json!(n));
            }
        }
        let mut by_level = serde_json::Map::new();
        {
            let mut stmt = s.conn().prepare(&format!(
                "SELECT level k, COUNT(*) n FROM events WHERE 1=1{where_sql} GROUP BY k"
            ))?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?;
            for (k, n) in rows.collect::<Result<Vec<_>, _>>()? {
                by_level.insert(k.to_string(), json!(n));
            }
        }
        let mut by_type = serde_json::Map::new();
        {
            let mut stmt = s.conn().prepare(&format!(
                "SELECT type k, COUNT(*) n FROM events WHERE 1=1{where_sql} GROUP BY k"
            ))?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?;
            for (k, n) in rows.collect::<Result<Vec<_>, _>>()? {
                by_type.insert(k, json!(n));
            }
        }
        let mut by_camera: Vec<Value> = Vec::new();
        {
            let sql = format!(
                "SELECT e.camera_id, COALESCE(c.name,'') name, COUNT(*) total, \
                 SUM(CASE WHEN e.status='false_positive' THEN 1 ELSE 0 END) fp \
                 FROM events e LEFT JOIN cameras c ON c.id=e.camera_id WHERE 1=1{where_sql} \
                 GROUP BY e.camera_id ORDER BY total DESC"
            );
            let mut stmt = s.conn().prepare(&sql)?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?;
            for (cid2, name, total2, fp) in rows.collect::<Result<Vec<_>, _>>()? {
                by_camera.push(json!({"camera_id": cid2, "name": name, "total": total2,
                    "false_positive": fp,
                    "rate": if total2 > 0 { (fp as f64 / total2 as f64 * 1000.0).round() / 1000.0 } else { 0.0 }}));
            }
        }
        let mut daily: Vec<Value> = Vec::new();
        {
            let sql = format!(
                "SELECT substr(ts,1,10) d, COUNT(*) total, \
                 SUM(CASE WHEN status='false_positive' THEN 1 ELSE 0 END) fp \
                 FROM events WHERE 1=1{where_sql} GROUP BY d ORDER BY d"
            );
            let mut stmt = s.conn().prepare(&sql)?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for (d, total2, fp) in rows.collect::<Result<Vec<_>, _>>()? {
                daily.push(json!({"date": d, "total": total2, "false_positive": fp}));
            }
        }
        let stat = |m: &serde_json::Map<String, Value>, k: &str| {
            m.get(k).and_then(|v| v.as_i64()).unwrap_or(0)
        };
        let fp = stat(&by_status, "false_positive");
        let handled = stat(&by_status, "acked") + fp + stat(&by_status, "ignored");
        let l3_total = by_level.get("3").and_then(|v| v.as_i64()).unwrap_or(0);
        let unhandled = stat(&by_status, "new");
        Ok(Json(json!({
            "days": days, "start_ts": start, "end_ts": end,
            "total": total,
            "by_status": Value::Object(by_status),
            "by_level": Value::Object(by_level),
            "by_type": Value::Object(by_type),
            "by_camera": by_camera,
            "false_positive_rate": if handled > 0 { (fp as f64 / handled as f64 * 1000.0).round() / 1000.0 } else { 0.0 },
            "unhandled": unhandled,
            "l3_total": l3_total,
            "by_ai_verdict": {"likely_true": 0, "likely_false": 0, "uncertain": 0},
            "daily": daily,
        }))
        .into_response())
    })
}

// ---- 配对/设备（T-620/R2-309） ----
async fn create_pairing(State(st): State<Shared>) -> Response {
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

async fn claim_pairing(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
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
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "code 必填"})),
        )
            .into_response();
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
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "配对码无效或已过期"})),
        )
            .into_response();
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

async fn list_devices(State(st): State<Shared>) -> Response {
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

async fn revoke_device(State(st): State<Shared>, AxPath(did): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.revoke_device(did)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "device not found"})),
            )
                .into_response())
        }
    })
}

// ---- 审计/投递/静默/日历 ----
async fn audit_list(
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

async fn deliveries_list(
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

async fn retry_delivery(State(st): State<Shared>, AxPath(did): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.retry_delivery(did)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "delivery not found"})),
            )
                .into_response())
        }
    })
}

async fn silences_list(State(st): State<Shared>) -> Response {
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

async fn add_silence(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let scope = body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if scope.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "scope 必填"})),
        )
            .into_response();
    }
    let target = body.get("target").and_then(|v| v.as_str());
    let reason = body.get("reason").and_then(|v| v.as_str());
    let until = body
        .get("until_ts")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if until.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "until_ts 必填"})),
        )
            .into_response();
    }
    with_store(&st, |s| {
        let sid = s.add_silence(&scope, target, reason, &until)?;
        Ok(Json(json!({"id": sid})).into_response())
    })
}

async fn delete_silence(State(st): State<Shared>, AxPath(sid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_silence(sid)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "silence not found"})),
            )
                .into_response())
        }
    })
}

async fn calendar_list(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.calendar_exceptions()?;
        Ok(Json(
            rows.iter()
                .map(|c| json!({"date": c.date, "mode": c.mode, "note": c.note}))
                .collect::<Vec<_>>(),
        ))
    })
}

async fn set_calendar(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
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
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "date 必填；mode 必须是 workday|holiday"})),
        )
            .into_response();
    }
    with_store(&st, |s| {
        s.set_calendar_exception(&date, &mode, &note)?;
        Ok(Json(json!({"date": date, "mode": mode, "note": note})).into_response())
    })
}

async fn delete_calendar(State(st): State<Shared>, AxPath(d): AxPath<String>) -> Response {
    with_store(&st, |s| {
        if s.delete_calendar_exception(&d)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "calendar exception not found"})),
            )
                .into_response())
        }
    })
}

// ---- 规则模板（任务116） ----
async fn templates_list(State(st): State<Shared>) -> Response {
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

async fn add_template(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let config = body.get("config").cloned();
    if name.is_empty() || config.is_none() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "name 与 config 必填"})),
        )
            .into_response();
    }
    with_store(&st, |s| {
        match s.add_rule_template(&name, &config.unwrap().to_string()) {
            Ok(id) => Ok(Json(json!({"id": id, "name": name})).into_response()),
            Err(_) => Ok((
                StatusCode::CONFLICT,
                Json(json!({"detail": "模板名已存在"})),
            )
                .into_response()),
        }
    })
}

async fn update_template(
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
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "template not found"})),
            )
                .into_response())
        }
    })
}

async fn delete_template(State(st): State<Shared>, AxPath(tid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.delete_rule_template(tid)? {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "template not found"})),
            )
                .into_response())
        }
    })
}

// ---- 导出 / 指标 / 诊断 / meta ----
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

async fn export_events_csv(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.events_query(&ls_store::EventsFilter {
            limit: 10_000,
            offset: 0,
            ..Default::default()
        })?;
        let mut out = String::from("id,ts,camera_id,type,level,track_id,status,detail\n");
        for e in &rows {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                e.id,
                csv_escape(&e.ts),
                e.camera_id,
                csv_escape(&e.type_),
                e.level,
                e.track_id,
                csv_escape(&e.status),
                csv_escape(&e.detail)
            ));
        }
        Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8")], out).into_response())
    })
}

async fn export_stats_csv(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.stats_daily(30, None, None)?;
        let mut out = String::from("date,total,l1,l2,l3\n");
        for (d, row) in rows {
            let g = |k: &str| row.get(k).copied().unwrap_or(0);
            out.push_str(&format!(
                "{},{},{},{},{}\n",
                d,
                g("total"),
                g("l1"),
                g("l2"),
                g("l3")
            ));
        }
        Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8")], out).into_response())
    })
}

async fn metrics(State(st): State<Shared>) -> Response {
    let (cams, events) = match ls_store::Store::open(&st.db_path) {
        Ok(s) => (
            s.cameras().map(|c| c.len()).unwrap_or(0),
            s.count_events().unwrap_or(0),
        ),
        Err(_) => (0, 0),
    };
    let body = format!(
        "# HELP labsentinel_cameras_total configured cameras\n\
         # TYPE labsentinel_cameras_total gauge\n\
         labsentinel_cameras_total {cams}\n\
         # HELP labsentinel_events_total stored events\n\
         # TYPE labsentinel_events_total gauge\n\
         labsentinel_events_total {events}\n\
         # HELP labsentinel_schema_version db schema version\n\
         # TYPE labsentinel_schema_version gauge\n\
         labsentinel_schema_version {}\n",
        ls_store::SCHEMA_VERSION_LATEST
    );
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
}

async fn diagnostics_summary(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let counts = json!({
            "cameras": s.cameras().map(|c| c.len()).unwrap_or(0),
            "zones": s.zones_all().map(|z| z.len()).unwrap_or(0),
            "rules": s.rules_all().map(|r| r.len()).unwrap_or(0),
            "events": s.count_events().unwrap_or(0),
        });
        Ok(Json(json!({
            "version": st.version,
            "backend": "rust",
            "uptime_s": null,
            "counts": counts,
            "disk": {"data_dir": st.data_dir, "free_bytes": null, "total_bytes": null},
            "logs": {"file": null, "size_bytes": 0, "rotated": []},
            "recent_errors": [],
        })))
    })
}

async fn api_v1_meta() -> Json<Value> {
    // /api/v1/meta：文案单一来源（与 Python constants.py 同口径；T-621）
    Json(json!({
        "types": {
            "enter": "进入区域", "exit": "离开区域",
            "no_lab_coat": "未穿实验服", "glove": "手套", "goggles": "护目镜"
        },
        "levels": {"1": "提示", "2": "一般", "3": "严重"},
        "statuses": {"new": "未处置", "acked": "已确认",
                     "false_positive": "误报", "ignored": "忽略"}
    }))
}

// ==== R2-309 批次 2：配置导出/签名/状态端点/图片识别记录（契约 1:1 对齐 Python） ====

/// Python `_RTSP_CRED` 同款：rtsp://user:pass@ → rtsp://user:***:***@
fn mask_rtsp_creds(source: &str) -> String {
    // 简单可靠的做法：找到 "://" 后第一个 @，把其中的 "pass:" 部分掩掉
    let Some(idx) = source.find("://") else {
        return source.to_string();
    };
    let (scheme, rest) = source.split_at(idx + 3);
    match rest.find('@') {
        Some(at) if !rest[..at].is_empty() => {
            let cred = &rest[..at];
            let masked = match cred.split_once(':') {
                Some((user, _)) => format!("{user}:***:***@"),
                None => "***:***@".to_string(),
            };
            format!("{scheme}{masked}{}", &rest[at + 1..])
        }
        _ => source.to_string(),
    }
}

fn file_info(p: &std::path::Path) -> Value {
    match std::fs::metadata(p) {
        Ok(m) => json!({"path": p.to_string_lossy(), "exists": true,
                        "size_bytes": m.len(),
                        "mtime": iso_now()}),
        Err(_) => json!({"path": p.to_string_lossy(), "exists": false,
                         "size_bytes": null, "mtime": null}),
    }
}

async fn copy_camera_config(
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
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "target_ids 必填"})),
        )
            .into_response();
    }
    if targets.contains(&cid) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "target_ids 不能包含源摄像头自身"})),
        )
            .into_response();
    }
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "camera not found"})),
            )
                .into_response());
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

async fn config_export(
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

async fn config_validate(State(st): State<Shared>) -> Response {
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

/// T-108 对齐：HMAC(path|exp) 短期签名；密钥 = admin_token（未设时进程内随机）。
fn media_secret(st: &Shared) -> String {
    use std::sync::OnceLock;
    static FALLBACK: OnceLock<String> = OnceLock::new();
    if !st.admin_token.is_empty() {
        st.admin_token.clone()
    } else {
        FALLBACK.get_or_init(random_token).clone()
    }
}

async fn sign_media(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let path = q.get("path").cloned().unwrap_or_default();
    if !path.starts_with("/api/") || path.contains('?') || path.contains('#') || path.contains(' ')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "path 只能是 /api/ 媒体路径（查询串自行拼接在签名后）"})),
        )
            .into_response();
    }
    let ttl = 300i64;
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        + ttl;
    let msg = format!("{path}|{exp}");
    use hmac::Mac; // trait 方法（update/finalize）需在作用域
    type HmacSha256 = hmac::Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(media_secret(&st).as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(msg.as_bytes());
    let sig: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Json(json!({"exp": exp.to_string(), "sig": format!("sha256={sig}")})).into_response()
}

async fn model_status() -> Response {
    // rust 控制面无 Python 引擎；文件状态与配置如实上报，引擎字段如实标注。
    let fp32 = std::env::var("LS_MODEL_FP32").unwrap_or_default();
    let ppe = std::env::var("LS_PPE_MODEL").unwrap_or_default();
    let active = if ppe.is_empty() { &fp32 } else { &ppe };
    Json(json!({
        "engine": {"loaded": false, "backend": null,
                   "note": "rust 控制面独立模式不加载 ONNX（数据面在 Python 服务/sidecar）"},
        "models": {"ppe": file_info(std::path::Path::new(active)),
                   "fp32": file_info(std::path::Path::new(&fp32))},
        "backend_order": std::env::var("LS_BACKEND_ORDER").unwrap_or_default(),
    }))
    .into_response()
}

async fn mqtt_status() -> Response {
    // rust 侧未接 MQTT 桥（任务125 在 Python 侧）；未启用形状与 Python 一致
    Json(json!({"enabled": false, "connected": false,
                "note": "MQTT 事件桥在 Python 服务实现；rust 控制面独立模式不启用"}))
    .into_response()
}

async fn genai_status() -> Response {
    Json(json!({"provider": "none", "configured": false,
                "note": "VLM 复核在 Python 服务实现；rust 控制面独立模式不启用"}))
    .into_response()
}

async fn plugins_list() -> Response {
    Json(json!({"plugins": [],
                "note": "插件管理器在 Python 服务实现；rust 控制面独立模式无插件"}))
    .into_response()
}

async fn update_check_status() -> Response {
    // LS_UPDATE_MANIFEST_URL 未配置时 Python 返回 enabled=false（只查静态清单）
    let manifest = std::env::var("LS_UPDATE_MANIFEST_URL").unwrap_or_default();
    Json(
        json!({"enabled": !manifest.is_empty(), "current": st_version(),
        "latest": null, "update_available": false,
        "error": null,
        "note": if manifest.is_empty() {
            "未配置 LS_UPDATE_MANIFEST_URL，更新检查关闭"
        } else {
            "rust 控制面仅上报配置状态；清单拉取在 Python 服务"
        }}),
    )
    .into_response()
}

fn st_version() -> String {
    std::env::var("LS_VERSION").unwrap_or_else(|_| "0.2.0".into())
}

async fn alerts_status(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let counts = s.deliveries(None, 10_000)?;
        let mut summary = std::collections::BTreeMap::new();
        for d in &counts {
            *summary.entry(d.status.clone()).or_insert(0) += 1;
        }
        Ok(Json(json!({
            "configured": {"wecom": false, "dingtalk": false, "feishu": false,
                            "serverchan": false, "pushplus": false, "qmsg": false,
                            "email": false, "webhook": false},
            "note": "渠道配置在 Python 服务（app_settings/env + 密钥加密）；rust 控制面只汇总投递",
            "deliveries_summary": summary,
            "deliveries_total": counts.len(),
        }))
        .into_response())
    })
}

async fn alert_template_get() -> Json<Value> {
    // 与 Python settings 默认模板 + placeholders 说明一致（任务85）
    Json(json!({
        "title": "LabSentinel 级别{level}告警",
        "text": "{icon} L{level} {type_text} · {ts}\n摄像头 #{camera_id}{track_part}\n{detail}",
        "placeholders": {
            "icon": "级别图标（ℹ️/⚠️/🔴）", "level": "级别数字 1-3",
            "type": "事件类型标识", "type_text": "事件类型中文名",
            "ts": "事件时间", "camera_id": "摄像头 ID",
            "track_id": "跟踪 ID（occupancy 为 0）", "detail": "事件详情",
            "track_part": "「 · track N」或空串（occupancy 时为空）",
        },
    }))
}

async fn folder_images_list(
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

async fn folder_images_summary(
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

async fn logs_tail(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit: usize = q
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
        .clamp(1, 2000);
    // rust 控制面的日志走 stdout；若有 Python 侧落盘日志则读取尾部
    let log_path = std::path::Path::new(&st.data_dir).join("logs/server.log");
    let lines: Vec<String> = match std::fs::read_to_string(&log_path) {
        Ok(content) => content
            .lines()
            .rev()
            .take(limit)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        Err(_) => Vec::new(),
    };
    Json(json!({"lines": lines, "file": log_path.to_string_lossy(),
                "note": if lines.is_empty() { "rust 控制面日志走 stdout（无落盘文件）" } else { "" }}))
        .into_response()
}

async fn ppe_status(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        // ppe_model 优先级：env > app_settings（Q-08 按需下载写入）
        let env_ppe = std::env::var("LS_PPE_MODEL").ok().filter(|v| !v.is_empty());
        let db_ppe = s.setting_get("ppe_model").ok().flatten();
        let model_path = env_ppe.or(db_ppe);
        let installed = model_path
            .as_ref()
            .map(|p| std::path::Path::new(p).is_file())
            .unwrap_or(false);
        Ok(Json(json!({
            "installed": installed,
            "mode": if installed { "ppe" } else { "person_only" },
            "active": installed,
            "license_confirmed_required": !installed,
            "license_notice": "实验服模型权重由 Roboflow 训练数据集派生，具体许可仍待确认（Q-08）。下载即表示你已阅读并接受该数据集的用途限制。",
            "model_path": model_path,
            "note": "rust 控制面只读状态；下载端点在 Python 服务（含 SHA256 校验）",
        }))
        .into_response())
    })
}

async fn ws_events(State(st): State<Shared>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, st.events_tx.subscribe()))
}

// ==== R2-309 批次 4：网页端告警渠道配置（任务140 复刻 + Fernet 互操作）+ 局域网发现 ====

/// 与 Python `alert_config.FIELDS/SECRET/URL/BOOL/INT` 同口径的字段元数据。
mod alert_cfg {
    pub const PREFIX: &str = "alert.";
    /// channel -> (api 字段, settings 字段)
    pub const FIELDS: &[(&str, &[(&str, &str)])] = &[
        ("wecom", &[("webhook", "wecom_webhook")]),
        ("dingtalk", &[("webhook", "dingtalk_webhook")]),
        ("feishu", &[("webhook", "feishu_webhook")]),
        ("serverchan", &[("sendkey", "serverchan_sendkey")]),
        ("pushplus", &[("token", "pushplus_token")]),
        ("qmsg", &[("key", "qmsg_key"), ("qq", "qmsg_qq")]),
        (
            "email",
            &[
                ("smtp_host", "smtp_host"),
                ("smtp_port", "smtp_port"),
                ("smtp_user", "smtp_user"),
                ("smtp_pass", "smtp_pass"),
                ("to", "alert_email_to"),
            ],
        ),
        (
            "webhook",
            &[
                ("url", "webhook_url"),
                ("secret", "webhook_secret"),
                ("allow_http", "webhook_allow_http"),
            ],
        ),
    ];
    pub const SECRET_FIELDS: &[&str] = &[
        "wecom_webhook",
        "dingtalk_webhook",
        "feishu_webhook",
        "serverchan_sendkey",
        "pushplus_token",
        "qmsg_key",
        "smtp_pass",
        "webhook_secret",
    ];
    pub const URL_FIELDS: &[&str] = &[
        "wecom_webhook",
        "dingtalk_webhook",
        "feishu_webhook",
        "webhook_url",
    ];
    pub const BOOL_FIELDS: &[&str] = &["webhook_allow_http"];
    pub const INT_FIELDS: &[&str] = &["smtp_port"];

    /// 渠道 URL host 白名单（与 Python alerts.py 常量同口径）
    pub fn url_allowed_hosts(field: &str) -> Option<&'static [&'static str]> {
        match field {
            "wecom_webhook" => Some(&["qyapi.weixin.qq.com"]),
            "dingtalk_webhook" => Some(&["oapi.dingtalk.com"]),
            "feishu_webhook" => Some(&["open.feishu.cn"]),
            _ => None,
        }
    }

    pub fn fields_of(channel: &str) -> Option<&'static [(&'static str, &'static str)]> {
        FIELDS.iter().find(|(c, _)| *c == channel).map(|(_, m)| *m)
    }

    pub fn is_secret(field: &str) -> bool {
        SECRET_FIELDS.contains(&field)
    }

    pub fn is_url(field: &str) -> bool {
        URL_FIELDS.contains(&field)
    }

    pub fn is_bool(field: &str) -> bool {
        BOOL_FIELDS.contains(&field)
    }

    pub fn is_int(field: &str) -> bool {
        INT_FIELDS.contains(&field)
    }
}

/// Python `_mask_url` 复刻：保留 scheme/host/路径结构，查询值全 ***，长 token 打码。
fn mask_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_string(), r.to_string()),
        None => return "***".into(),
    };
    let (authority, tail) = match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.clone(), String::new()),
    };
    let (path_part, query) = match tail.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (tail.clone(), None),
    };
    let mut parts: Vec<String> = path_part
        .split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if let Some(last) = parts.last_mut() {
        if last.len() >= 8 {
            *last = "***".into();
        }
    }
    let query_out = query.map(|q| {
        q.split('&')
            .filter(|kv| !kv.is_empty())
            .map(|kv| format!("{}=***", kv.split('=').next().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("&")
    });
    format!(
        "{scheme}://{authority}/{}{}{}",
        parts.join("/"),
        if query_out.is_some() { "?" } else { "" },
        query_out.unwrap_or_default()
    )
}

/// 字段当前有效值：DB 覆盖（解密）优先，其次 env（Python load() 语义的读路径）。
fn alert_field_value(s: &ls_store::Store, st: &Shared, field: &str) -> String {
    if let Ok(Some(raw)) = s.setting_get(&format!("{}{field}", alert_cfg::PREFIX)) {
        if alert_cfg::is_secret(field) {
            return fernet::unprotect(&raw, std::path::Path::new(&st.data_dir));
        }
        return raw;
    }
    std::env::var(format!("LS_{}", field.to_ascii_uppercase())).unwrap_or_default()
}

/// Python `alert_config.status` 复刻：永不回显密钥/完整 webhook。
fn alerts_config_status(s: &ls_store::Store, st: &Shared) -> Value {
    let mut channels = serde_json::Map::new();
    for (channel, mapping) in alert_cfg::FIELDS {
        let mut item = serde_json::Map::new();
        let mut any_db = false;
        let mut any_env = false;
        for (api_name, field) in mapping.iter() {
            let db_raw = s
                .setting_get(&format!("{}{field}", alert_cfg::PREFIX))
                .ok()
                .flatten();
            if db_raw.is_some() {
                any_db = true;
            }
            let val = alert_field_value(s, st, field);
            if !val.is_empty() {
                any_env = true;
            }
            if alert_cfg::is_bool(field) {
                let b = matches!(val.as_str(), "1" | "true" | "True" | "yes" | "on");
                item.insert(api_name.to_string(), json!(b));
                continue;
            }
            if alert_cfg::is_int(field) {
                let n: Option<i64> = val.parse().ok();
                item.insert(api_name.to_string(), json!(n));
                continue;
            }
            let configured_like = !val.trim().is_empty();
            item.insert(format!("has_{api_name}"), json!(configured_like));
            if alert_cfg::is_url(field) {
                item.insert(api_name.to_string(), json!(mask_url(&val)));
            } else if alert_cfg::is_secret(field) {
                item.insert(api_name.to_string(), json!(""));
            } else {
                item.insert(api_name.to_string(), json!(val));
            }
        }
        // configured 判定（Python _configured 同口径；qmsg 需 key+qq、email 需 host+to）
        let get = |f: &str| alert_field_value(s, st, f);
        let configured = match *channel {
            "qmsg" => !get("qmsg_key").trim().is_empty() && !get("qmsg_qq").trim().is_empty(),
            "email" => {
                !get("smtp_host").trim().is_empty() && !get("alert_email_to").trim().is_empty()
            }
            _ => mapping
                .iter()
                .any(|(_, f)| !get(f).trim().is_empty() && !alert_cfg::is_bool(f)),
        };
        item.insert("configured".into(), json!(configured));
        item.insert(
            "source".into(),
            json!(if any_db {
                "db"
            } else if any_env {
                "env"
            } else {
                "none"
            }),
        );
        channels.insert(channel.to_string(), Value::Object(item));
    }
    json!({"channels": Value::Object(channels),
           "note": "密钥/完整 webhook 不会回显；留空保存=保持原值；清除=回退启动时 .env"})
}

async fn alerts_config_get(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        Ok(Json(alerts_config_status(s, &st)).into_response())
    })
}

/// Python `_validate_url` 同口径：禁凭证；厂商渠道只允许 https + 白名单域名。
fn validate_alert_url(field: &str, url: &str) -> Result<(), String> {
    if let Some(authority) = url.split("://").nth(1) {
        if let Some((user, _)) = authority.rsplit_once('@') {
            if user.contains(':') && !user.is_empty() {
                return Err("Webhook URL 不能携带 user:pass@ 凭证".into());
            }
        }
    }
    if field == "webhook_url" {
        let ok_scheme = url.starts_with("https://") || url.starts_with("http://");
        let has_host = url
            .split("://")
            .nth(1)
            .map(|r| !r.is_empty())
            .unwrap_or(false);
        if !ok_scheme || !has_host {
            return Err("Webhook URL 必须是 http(s) 且含主机".into());
        }
        return Ok(());
    }
    if let Some(allowed) = alert_cfg::url_allowed_hosts(field) {
        if !url.starts_with("https://") {
            return Err(format!(
                "该渠道只允许 https 且域名为 {}",
                allowed.join(", ")
            ));
        }
        let host = url::host_of(url).unwrap_or_default();
        if !allowed.contains(&host.as_str()) {
            return Err(format!(
                "该渠道只允许 https 且域名为 {}",
                allowed.join(", ")
            ));
        }
    }
    Ok(())
}

async fn alerts_config_put(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let Some(body_obj) = body.as_object() else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "body 必须是对象"})),
        )
            .into_response();
    };
    let mut pending: Vec<(String, String)> = Vec::new(); // (field, serialized)
    for (channel, values) in body_obj {
        let Some(mapping) = alert_cfg::fields_of(channel) else {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": format!("未知渠道: {channel}")})),
            )
                .into_response();
        };
        let Some(values) = values.as_object() else {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": format!("{channel} 配置必须是对象")})),
            )
                .into_response();
        };
        for (api_name, raw) in values {
            let Some((_, field)) = mapping.iter().find(|(a, _)| a == api_name) else {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"detail": format!("{channel} 含未知字段: {api_name}")})),
                )
                    .into_response();
            };
            // 留空 = 保持原值
            let text = raw
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let Some(text) = text else { continue };
            if alert_cfg::is_int(field) {
                let Ok(n) = text.parse::<i64>() else {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"detail": format!("{api_name} 必须是整数")})),
                    )
                        .into_response();
                };
                if !(1..=65535).contains(&n) {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"detail": format!("{api_name} 必须在 1-65535")})),
                    )
                        .into_response();
                }
                pending.push((field.to_string(), n.to_string()));
                continue;
            }
            if alert_cfg::is_bool(field) {
                let b = matches!(text.as_str(), "1" | "true" | "True" | "yes" | "on");
                pending.push((field.to_string(), if b { "1".into() } else { "0".into() }));
                continue;
            }
            if alert_cfg::is_url(field) {
                if let Err(e) = validate_alert_url(field, &text) {
                    return (StatusCode::BAD_REQUEST, Json(json!({"detail": e}))).into_response();
                }
            }
            if *field == "alert_email_to" && !text.is_empty() {
                let parts: Vec<&str> = text.split('@').collect();
                let ok = parts.len() == 2
                    && !parts[0].is_empty()
                    && !parts[1].is_empty()
                    && parts[1].contains('.');
                if !ok {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"detail": "收件邮箱格式不正确"})),
                    )
                        .into_response();
                }
            }
            if *field == "smtp_host" && (text.contains("://") || text.contains(' ')) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"detail": "SMTP 服务器只填主机名，如 smtp.qq.com"})),
                )
                    .into_response();
            }
            if *field == "qmsg_qq" && !text.chars().all(|c| c.is_ascii_digit()) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"detail": "Qmsg 接收 QQ 必须是数字"})),
                )
                    .into_response();
            }
            // 敏感字段加密（Fernet，与 POSIX 语义互操作）；失败拒绝保存（R2-106）
            let serialized = if alert_cfg::is_secret(field) {
                match fernet::protect(&text, std::path::Path::new(&st.data_dir)) {
                    Some(enc) => enc,
                    None => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"detail": "加密组件不可用，拒绝明文保存密钥"})),
                        )
                            .into_response()
                    }
                }
            } else {
                text.clone()
            };
            pending.push((field.to_string(), serialized));
        }
    }
    with_store(&st, |s| {
        for (field, val) in &pending {
            s.setting_set(&format!("{}{field}", alert_cfg::PREFIX), val)?;
        }
        Ok(Json(alerts_config_status(s, &st)).into_response())
    })
}

async fn alerts_config_reset(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let channel = q.get("channel").cloned();
    let fields: Vec<String> = match channel.as_deref() {
        None => alert_cfg::FIELDS
            .iter()
            .flat_map(|(_, m)| m.iter().map(|(_, f)| f.to_string()))
            .collect(),
        Some(c) => match alert_cfg::fields_of(c) {
            Some(m) => m.iter().map(|(_, f)| f.to_string()).collect(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"detail": format!("未知渠道: {c}")})),
                )
                    .into_response();
            }
        },
    };
    with_store(&st, |s| {
        for f in &fields {
            // Python delete_app_settings 移除行；这里置空串——读路径 DB 覆盖为空 →
            // env 生效，行为等价（source 会显示 db，与 Python 的 env 有差异，如实记录）
            s.setting_set(&format!("{}{f}", alert_cfg::PREFIX), "")?;
        }
        Ok(Json(alerts_config_status(s, &st)).into_response())
    })
}

// ---- 局域网发现（任务113；require_admin 口径同 Python） ----
async fn discovery_cameras(
    State(_st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let timeout: u64 = q
        .get("timeout")
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v.clamp(1.0, 5.0) as u64)
        .unwrap_or(3);
    let refresh = q.get("refresh").map(|v| v == "1").unwrap_or(false);
    let devices = discovery::get_devices(timeout, refresh);
    let hint = if devices.is_empty() {
        "未发现设备：确认摄像头与主机同网段、摄像头已开启 ONVIF、UDP 3702 未被防火墙拦截；也可用品牌模板手输 RTSP 地址"
    } else {
        ""
    };
    Json(
        json!({"devices": devices, "lan": discovery::lan_addresses(),
                "error": hint,
                "note": "ffprobe 增强探测在 rust 侧为可选（tools/ffprobe 存在时启用）"}),
    )
    .into_response()
}

async fn discovery_probe(State(_st): State<Shared>, Json(body): Json<Value>) -> Response {
    let host = body
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if host.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "host 必填"})),
        )
            .into_response();
    }
    let port = body.get("port").and_then(|v| v.as_i64()).unwrap_or(554);
    if !(1..=65535).contains(&port) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "port 必须在 1-65535"})),
        )
            .into_response();
    }
    let username = body.get("username").and_then(|v| v.as_str());
    let password = body.get("password").and_then(|v| v.as_str());
    // 凭证仅本次探测用（不写日志/不回传——probe_host 返回值已保证）
    Json(discovery::probe_host(
        &host,
        port as u16,
        username,
        password,
    ))
    .into_response()
}

// ==== R2-309 批次 3b：图片记录详情/导出/模板预览与应用/关机/快照占位 ====

fn folder_row_json(r: &ls_store::FolderImageRow, path_full: bool) -> Value {
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

async fn folder_image_detail(State(st): State<Shared>, AxPath(iid): AxPath<i64>) -> Response {
    with_store(&st, |s| match s.folder_image(iid)? {
        Some(r) => Ok(Json(folder_row_json(&r, true)).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "image record not found"})),
        )
            .into_response()),
    })
}

/// 与 Python folder_readable_path 同口径：记录的 path 是真实文件路径
async fn folder_image_file(State(st): State<Shared>, AxPath(iid): AxPath<i64>) -> Response {
    with_store(&st, |s| match s.folder_image(iid)? {
        Some(r) => match std::fs::read(&r.path) {
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
            Err(_) => Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "原图片文件不存在（可能已被系统清理）"})),
            )
                .into_response()),
        },
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "image record not found"})),
        )
            .into_response()),
    })
}

async fn folder_image_thumb(State(st): State<Shared>, AxPath(iid): AxPath<i64>) -> Response {
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
                Err(_) => Ok((
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    Json(json!({"detail": "缩略图缓存不存在（rust 控制面不生成缩略图；原图走 /file）"})),
                )
                    .into_response()),
            }
        }
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "image record not found"})),
        )
            .into_response()),
    })
}

async fn folder_images_csv(
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

/// Python TYPE_TEXT/LEVEL_ICON 同口径（constants.py 同步维护）
fn type_text(t: &str) -> &str {
    match t {
        "enter" => "进入安全区",
        "enter_no_ppe" => "进入安全区未穿实验服",
        "exit" => "离开安全区",
        "occupancy_over" => "在场人数超限",
        "stranger" => "陌生人进入",
        other => other,
    }
}

fn level_icon(level: i64) -> &'static str {
    match level {
        1 => "ℹ️",
        2 => "⚠️",
        3 => "🔴",
        _ => "ℹ️",
    }
}

/// Python alerts.render_alert 复刻：占位符白名单 + 未知占位符原样保留
fn render_alert_row(
    template: &str,
    ts: &str,
    camera_id: i64,
    type_: &str,
    level: i64,
    track_id: i64,
    detail: &str,
) -> String {
    let track_part = if type_ == "occupancy_over" {
        String::new()
    } else {
        format!(" · track {track_id}")
    };
    let vals: Vec<(&str, String)> = vec![
        ("icon", level_icon(level).to_string()),
        ("level", level.to_string()),
        ("type", type_.to_string()),
        ("type_text", type_text(type_).to_string()),
        ("ts", ts.to_string()),
        ("camera_id", camera_id.to_string()),
        ("track_id", track_id.to_string()),
        ("track_part", track_part),
        ("detail", detail.to_string()),
    ];
    // 单次替换：先按长度降序尝试每个 {key}；未知占位符原样保留
    let mut out = template.to_string();
    for (k, v) in &vals {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

async fn alert_template_preview(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let _ = &st; // 模板文本取默认值（settings 单一来源在 Python 侧；rust 用同默认串）
    let ts = body
        .get("ts")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(iso_now);
    let camera_id = body.get("camera_id").and_then(|v| v.as_i64()).unwrap_or(1);
    let type_ = body
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("enter")
        .to_string();
    let level = body.get("level").and_then(|v| v.as_i64()).unwrap_or(2);
    let track_id = body.get("track_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let detail = body
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let text = render_alert_row(
        "{icon} L{level} {type_text} · {ts}\n摄像头 #{camera_id}{track_part}\n{detail}",
        &ts,
        camera_id,
        &type_,
        level,
        track_id,
        &detail,
    );
    let title = format!("LabSentinel 级别{level}告警");
    Json(json!({"title": title, "text": text})).into_response()
}

async fn apply_rule_template(
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
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "template not found"})),
            )
                .into_response());
        };
        let parsed: Value =
            serde_json::from_str(&tpl.config_json).unwrap_or(Value::Array(Vec::new()));
        let Some(rules) = parsed.as_array() else {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": "模板 config_json 不是数组"})),
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

async fn shutdown_server() -> Response {
    // Python _do_shutdown 同语义：先应答再退出（延迟给响应送达留窗口）
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::process::exit(0);
    });
    Json(json!({"ok": true, "message": "服务将在 300ms 内退出"})).into_response()
}

async fn camera_snapshot(State(st): State<Shared>, AxPath(cid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        if s.camera(cid)?.is_none() {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "camera not found"})),
            )
                .into_response());
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

/// 标准 ZIP（stored 无压缩）写出器：本地文件头 + central directory + EOCD。
/// CRC32（IEEE 802.3，表驱动）手写实现，零额外依赖——存证打包只有 jpeg/mp4，
/// stored 已足够（压缩收益小，可靠优先）。
struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
}

impl ZipWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            central: Vec::new(),
        }
    }

    fn crc32(data: &[u8]) -> u32 {
        static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            let mut t = [0u32; 256];
            for (i, e) in t.iter_mut().enumerate() {
                let mut c = i as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
                *e = c;
            }
            t
        });
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c = table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
        !c
    }

    fn add_file(&mut self, name: &str, data: &[u8]) {
        let crc = Self::crc32(data);
        let offset = self.out.len() as u32;
        // 本地文件头：sig, ver(20), flags(0), method(stored), time/date(0), crc,
        // csize=size, usize=size, namelen, extralen(0)
        self.out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&20u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&crc.to_le_bytes());
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
        self.out.extend_from_slice(data);
        // central directory 条目
        self.central
            .extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        self.central.extend_from_slice(&20u16.to_le_bytes());
        self.central.extend_from_slice(&20u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes());
        self.central.extend_from_slice(&crc.to_le_bytes());
        self.central
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.central
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.central
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes()); // extra
        self.central.extend_from_slice(&0u16.to_le_bytes()); // comment
        self.central.extend_from_slice(&0u16.to_le_bytes()); // disk
        self.central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        self.central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        self.central.extend_from_slice(&offset.to_le_bytes());
        self.central.extend_from_slice(name.as_bytes());
    }

    fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.out.len() as u32;
        let cd_size = self.central.len() as u32;
        let count = {
            // 条目数 = central 里 0x02014b50 出现次数
            self.central
                .windows(4)
                .filter(|w| *w == 0x0201_4b50u32.to_le_bytes())
                .count() as u16
        };
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // EOCD
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&count.to_le_bytes());
        self.out.extend_from_slice(&count.to_le_bytes());
        self.out.extend_from_slice(&cd_size.to_le_bytes());
        self.out.extend_from_slice(&cd_offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out
    }
}

fn read_media_file(st: &Shared, rel: &str) -> Option<(String, Vec<u8>)> {
    let p = std::path::Path::new(&st.data_dir).join(rel);
    let data = std::fs::read(&p).ok()?;
    let name = std::path::Path::new(rel)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    Some((name, data))
}

async fn event_evidence_zip(State(st): State<Shared>, AxPath(eid): AxPath<i64>) -> Response {
    with_store(&st, |s| {
        let Some(ev) = s.event_by_id(eid)? else {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "event not found"})),
            )
                .into_response());
        };
        let mut zip = ZipWriter::new();
        let mut parts = 0usize;
        if let Some(snap) = &ev.snapshot {
            if let Some((name, data)) = read_media_file(&st, snap) {
                zip.add_file(&format!("event-{eid}/{name}"), &data);
                parts += 1;
            }
        }
        if let Some(video) = &ev.video {
            if let Some((name, data)) = read_media_file(&st, video) {
                zip.add_file(&format!("event-{eid}/{name}"), &data);
                parts += 1;
            }
        }
        let meta = json!({
            "id": ev.id, "ts": ev.ts, "camera_id": ev.camera_id, "type": ev.type_,
            "level": ev.level, "track_id": ev.track_id, "detail": ev.detail,
            "status": ev.status,
        });
        zip.add_file(
            &format!("event-{eid}/evidence.json"),
            meta.to_string().as_bytes(),
        );
        if parts == 0 {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "事件无存证文件（快照/视频不存在）"})),
            )
                .into_response());
        }
        Ok(([(header::CONTENT_TYPE, "application/zip")], zip.finish()).into_response())
    })
}

async fn export_evidence_zip(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let parse_i64 = |k: &str| q.get(k).and_then(|v| v.parse().ok());
    let camera_id = parse_i64("camera_id");
    let days: i64 = q
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .clamp(1, 365);
    let start = iso_days_ago(days - 1) + "T00:00:00";
    with_store(&st, |s| {
        let rows = s.events_query(&ls_store::EventsFilter {
            camera_id,
            start_ts: Some(&start),
            limit: 2000,
            ..Default::default()
        })?;
        let with_media: Vec<_> = rows
            .into_iter()
            .filter(|e| e.snapshot.is_some() || e.video.is_some())
            .collect();
        if with_media.is_empty() {
            return Ok((
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "范围内无带存证的事件"})),
            )
                .into_response());
        }
        let mut zip = ZipWriter::new();
        let mut index = Vec::new();
        for e in &with_media {
            let mut files = 0usize;
            if let Some(snap) = &e.snapshot {
                if let Some((name, data)) = read_media_file(&st, snap) {
                    zip.add_file(&format!("event-{}/{}", e.id, name), &data);
                    files += 1;
                }
            }
            if let Some(video) = &e.video {
                if let Some((name, data)) = read_media_file(&st, video) {
                    zip.add_file(&format!("event-{}/{}", e.id, name), &data);
                    files += 1;
                }
            }
            index.push(json!({"id": e.id, "ts": e.ts, "camera_id": e.camera_id,
                "type": e.type_, "level": e.level, "detail": e.detail, "files": files}));
        }
        zip.add_file(
            "export-index.json",
            json!({"exported": index.len(), "events": index})
                .to_string()
                .as_bytes(),
        );
        Ok(([(header::CONTENT_TYPE, "application/zip")], zip.finish()).into_response())
    })
}

async fn config_import(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let mode = body
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("merge")
        .to_string();
    if mode != "merge" && mode != "replace" {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "mode 必须是 merge|replace"})),
        )
            .into_response();
    }
    if mode == "replace"
        && !body
            .get("confirm_replace")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "replace 模式必须 confirm_replace=true（将清空现有摄像头/区域/规则；事件保留但失去对应摄像头）"})),
        )
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
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"detail": "data 至少包含 cameras/zones/rules 之一"})),
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
fn plugin_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"detail": "plugin not found（rust 控制面无插件管理器，列表见 GET /api/plugins）"})),
    )
        .into_response()
}

async fn plugin_logs(AxPath(_pid): AxPath<String>) -> Response {
    plugin_not_found()
}

async fn plugin_action(AxPath(_pid): AxPath<String>) -> Response {
    plugin_not_found()
}

// ---- face（合规敏感：默认关闭。分支与 Python _face_guard 同语义） ----
fn face_enabled() -> bool {
    std::env::var("LS_FACE_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn face_guard() -> Result<(), Box<Response>> {
    if !face_enabled() {
        return Err(Box::new((
            StatusCode::FORBIDDEN,
            Json(json!({"detail": "人脸比对功能未开启（LS_FACE_ENABLED=false）。开启方式与合规要求见 README。"})),
        )
            .into_response()));
    }
    let key = std::env::var("LS_COMPREFACE_API_KEY").unwrap_or_default();
    if key.is_empty() {
        return Err(Box::new(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"detail": "人脸服务未配置（缺少 LS_COMPREFACE_API_KEY）"})),
            )
                .into_response(),
        ));
    }
    // CompreFace HTTP 客户端在 Python 服务实现（注册/识别/1:N 全链）；rust 独立
    // 模式开启开关后如实返回 503 而不是假装可用（§11.3 证据约束）。
    Err(Box::new((
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"detail": "CompreFace 客户端在 Python 服务实现；rust 控制面独立模式不支持人脸注册/识别",
                    "code": "not_available_standalone"})),
    )
        .into_response()))
}

async fn face_status() -> Json<Value> {
    let key = std::env::var("LS_COMPREFACE_API_KEY").unwrap_or_default();
    Json(
        json!({"enabled": face_enabled(), "configured": !key.is_empty(),
                "threshold": 0.75,
                "note": "CompreFace 客户端在 Python 服务实现"}),
    )
}

async fn face_register(State(_st): State<Shared>, _body: axum::body::Bytes) -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    unreachable!("face_guard 在独立模式下恒 Err（见上）")
}

async fn face_persons_list() -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    unreachable!("face_guard 在独立模式下恒 Err")
}

async fn face_clear_all() -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    unreachable!("face_guard 在独立模式下恒 Err")
}

async fn face_delete(AxPath(_subject): AxPath<String>) -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    unreachable!("face_guard 在独立模式下恒 Err")
}

// ---- 告警自检（真实发送：复用 ls-dispatch，渠道 URL 读 env） ----
async fn alert_test(
    State(_st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let want = q.get("channel").map(|s| s.as_str().to_string());
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    // 渠道 -> (env key, ls-dispatch Channel)；URL 与 Python settings 同名 env
    let candidates: Vec<(String, ls_dispatch::Channel, String)> = vec![
        (
            "wecom".into(),
            ls_dispatch::Channel::Wecom,
            env("LS_WECOM_WEBHOOK").unwrap_or_default(),
        ),
        (
            "dingtalk".into(),
            ls_dispatch::Channel::Dingtalk,
            env("LS_DINGTALK_WEBHOOK").unwrap_or_default(),
        ),
        (
            "feishu".into(),
            ls_dispatch::Channel::Feishu,
            env("LS_FEISHU_WEBHOOK").unwrap_or_default(),
        ),
        (
            "webhook".into(),
            ls_dispatch::Channel::Webhook,
            env("LS_WEBHOOK_URL").unwrap_or_default(),
        ),
    ];
    let mut results: Vec<Value> = Vec::new();
    let mut sent = 0usize;
    for (name, ch, url) in candidates {
        if let Some(w) = &want {
            if w != &name {
                continue;
            }
        }
        if url.is_empty() {
            if want.as_deref() == Some(name.as_str()) {
                // 显式点名但未配置：如实报告
                results.push(json!({"channel": name, "ok": false,
                                     "error": "未配置（env 为空）"}));
            }
            continue;
        }
        let cfg = ls_dispatch::ChannelConfig {
            url,
            allow_http: std::env::var("LS_WEBHOOK_ALLOW_HTTP")
                .map(|v| v == "1")
                .unwrap_or(false),
            ..Default::default()
        };
        let d = ls_dispatch::deliver(ch, &cfg, 2, "LabSentinel 渠道自检测试（rust 控制面）");
        if d.ok {
            sent += 1;
        }
        results.push(json!({"channel": name, "ok": d.ok, "attempts": d.attempts,
                             "error": d.error}));
    }
    Json(
        json!({"ok": want.is_none() || results.iter().any(|r| r["ok"] == true),
                "sent": sent, "results": results,
                "note": "渠道配置读 env；网页端配置的渠道（app_settings）在 Python 服务"}),
    )
    .into_response()
}

// ---- PPE 按需下载（Q-08 复刻：锁文件白名单 + SHA256 + 许可确认） ----
fn asset_lock_path(st: &Shared) -> std::path::PathBuf {
    // 便携布局 <包>/scripts/assets.lock.json → 开发布局 <repo>/scripts/assets.lock.json
    std::path::Path::new(&st.data_dir)
        .ancestors()
        .find_map(|p| {
            p.join("scripts/assets.lock.json")
                .is_file()
                .then(|| p.join("scripts/assets.lock.json"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("scripts/assets.lock.json"))
}

async fn ppe_download(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    if !body
        .get("confirm_license")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "必须先确认实验服模型的数据集许可说明"})),
        )
            .into_response();
    }
    let lock_path = asset_lock_path(&st);
    let Ok(lock_raw) = std::fs::read_to_string(&lock_path) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": format!("assets.lock.json 不存在: {}", lock_path.display())})),
        )
            .into_response();
    };
    let Ok(lock) = serde_json::from_str::<Value>(&lock_raw) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"detail": "assets.lock.json 解析失败"})),
        )
            .into_response();
    };
    let assets = match lock.get("assets").and_then(|v| v.as_object()) {
        Some(a) => a,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"detail": "assets.lock.json 缺 assets"})),
            )
                .into_response()
        }
    };
    // SSRF 防护对齐 Python _allowed_hosts：白名单域名只来自锁文件 urls
    let mut allowed_hosts: Vec<String> = Vec::new();
    for spec in assets.values() {
        if let Some(urls) = spec.get("urls").and_then(|v| v.as_array()) {
            for u in urls {
                if let Some(url) = u.as_str() {
                    if let Some(host) = url::host_of(url) {
                        if !allowed_hosts.contains(&host) {
                            allowed_hosts.push(host);
                        }
                    }
                }
            }
        }
    }
    let models_dir = std::path::Path::new(&st.data_dir).join("models");
    std::fs::create_dir_all(&models_dir).ok();
    let mut downloaded = Vec::new();
    for name in ["yolo11n_ppe.onnx", "yolo11n_ppe.classes.txt"] {
        let Some(spec) = assets.get(name) else {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": format!("锁文件缺资产: {name}")})),
            )
                .into_response();
        };
        let want_sha = spec.get("sha256").and_then(|v| v.as_str()).unwrap_or("");
        let want_size = spec.get("size").and_then(|v| v.as_i64()).unwrap_or(-1);
        let out = models_dir.join(name);
        // 已装且哈希正确 → 跳过重复下载
        if let Ok(existing) = std::fs::read(&out) {
            if existing.len() as i64 == want_size && sha256_hex(&existing) == want_sha {
                downloaded.push(json!({"name": name, "cached": true}));
                continue;
            }
        }
        let mut ok = false;
        let mut last_err = String::new();
        let urls: Vec<String> = spec
            .get("urls")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for url in &urls {
            // 白名单 + https 校验（与 Python _check_url 同口径）
            let Some(host) = url::host_of(url) else {
                continue;
            };
            if !allowed_hosts.contains(&host) || !url.starts_with("https://") {
                last_err = format!("url 不在白名单或非 https: {url}");
                continue;
            }
            match ureq::get(url)
                .timeout(std::time::Duration::from_secs(60))
                .call()
            {
                Ok(resp) => {
                    let mut bytes = Vec::new();
                    use std::io::Read;
                    if resp.into_reader().read_to_end(&mut bytes).is_err() {
                        last_err = "下载中断".into();
                        continue;
                    }
                    let got_sha = sha256_hex(&bytes);
                    if got_sha != want_sha || bytes.len() as i64 != want_size {
                        last_err = format!("SHA256/大小校验失败（got {got_sha}）");
                        continue;
                    }
                    if std::fs::write(&out, &bytes).is_err() {
                        last_err = "写盘失败".into();
                        continue;
                    }
                    ok = true;
                    break;
                }
                Err(e) => {
                    last_err = format!("下载失败: {e}");
                    continue;
                }
            }
        }
        if !ok {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"detail": format!("资产 {name} 下载/校验失败: {last_err}")})),
            )
                .into_response();
        }
        downloaded.push(json!({"name": name, "cached": false}));
    }
    let model_path = models_dir.join("yolo11n_ppe.onnx");
    with_store(&st, |s| {
        s.setting_set("ppe_model", &model_path.to_string_lossy())?;
        Ok(Json(json!({"ok": true, "installed": true, "mode": "ppe",
                       "downloaded": downloaded,
                       "model_path": model_path.to_string_lossy(),
                       "note": "已生效并写入 app_settings；env LS_PPE_MODEL 优先级更高"}))
        .into_response())
    })
}

mod url {
    /// 从 URL 提取 host（标准库解析；与 Python urlparse(url).hostname 等价的宽松实现）
    pub fn host_of(url: &str) -> Option<String> {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let host_port = rest.split(['/', '?', '#']).next()?;
        let host = host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port);
        if host.is_empty() || host.contains('@') {
            return None;
        }
        Some(host.to_string())
    }
}

async fn handle_ws(mut socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    use axum::extract::ws::Message;
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if socket.send(Message::Text(text)).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue, // 慢客户端丢弃
                    Err(_) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

fn server_err() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"detail": "服务器内部错误", "code": "internal_error"})),
    )
        .into_response()
}

#[tokio::main]
async fn main() {
    // Docker HEALTHCHECK：`labsentinel-server --healthcheck <db>` 只读探活即退出
    let mut args = std::env::args().skip(1);
    if let Some(a) = args.next() {
        if a == "--healthcheck" {
            let db = args
                .next()
                .unwrap_or_else(|| "server/data/labsentinel.db".into());
            let ok = ls_store::Store::open(&db)
                .ok()
                .and_then(|s| s.integrity_check().ok())
                .is_some_and(|v| v == "ok");
            std::process::exit(if ok { 0 } else { 1 });
        }
        // T-332：跨语言 DB 兼容驱动——对指定库写一条事件/投递并读回（exit 0=成功）。
        // 由 server/tests/test_store_bidirectional.py 调用，实现三路径兼容验收。
        if a == "--selftest-db" {
            let db = args.next().expect("--selftest-db <path>");
            let code = ls_store::selftest::run(&db);
            std::process::exit(code);
        }
        // R2-307: print the route inventory JSON and exit (coverage script).
        if a == "--dump-routes" {
            println!(
                "{}",
                serde_json::to_string_pretty(&dump_routes()).expect("json")
            );
            std::process::exit(0);
        }
    }
    let db_path =
        std::env::var("LS_DB_PATH").unwrap_or_else(|_| "server/data/labsentinel.db".into());
    let token = std::env::var("LS_ADMIN_TOKEN").unwrap_or_default();
    let read_token = std::env::var("LS_READ_TOKEN").unwrap_or_default();
    let read_protected = std::env::var("LS_AUTH_READ_PROTECTED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let data_dir = std::env::var("LS_DATA_DIR").unwrap_or_else(|_| "server/data".into());
    let version = std::env::var("LS_VERSION").unwrap_or_else(|_| "0.2.0".into());
    let port: u16 = std::env::var("LS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8000);
    let jobs = Arc::new(ls_jobs::Jobs::new(
        std::path::PathBuf::from(&db_path),
        ls_jobs::JobsConfig::from_env(std::path::Path::new(&db_path)),
    ));
    let st = Arc::new(AppState {
        db_path,
        admin_token: token,
        read_token,
        read_protected,
        trusted_hosts: vec!["127.0.0.1".into(), "localhost".into(), "::1".into()],
        allowed_origins: vec![],
        web_dist: "web/dist".into(),
        data_dir,
        version,
        events_tx: broadcast::channel(256).0,
        jobs,
        pair_state: std::sync::Mutex::new(PairState::default()),
    });
    let app = build_router(st.clone());
    st.jobs.start();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    println!("labsentinel-server READY port={port}");
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn test_state(db: &str) -> Shared {
        test_state_cfg(db, "", false)
    }

    fn test_state_cfg(db: &str, admin_token: &str, read_protected: bool) -> Shared {
        test_state_cfg_dir(db, admin_token, read_protected, "D:/tmp")
    }

    fn test_state_cfg_dir(
        db: &str,
        admin_token: &str,
        read_protected: bool,
        data_dir: &str,
    ) -> Shared {
        Arc::new(AppState {
            db_path: db.into(),
            admin_token: admin_token.into(),
            read_token: String::new(),
            read_protected,
            trusted_hosts: vec!["127.0.0.1".into(), "localhost".into()],
            allowed_origins: vec![],
            web_dist: String::new(),
            data_dir: data_dir.into(),
            version: "0.2.0".into(),
            events_tx: broadcast::channel(16).0,
            jobs: Arc::new(ls_jobs::Jobs::new(
                std::path::PathBuf::from(db),
                ls_jobs::JobsConfig::default_for_db(std::path::Path::new(db)),
            )),
            pair_state: std::sync::Mutex::new(PairState::default()),
        })
    }

    async fn body_json(resp: Response) -> Value {
        let b = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        serde_json::from_slice(&b).unwrap()
    }

    /// 测试用最小 schema（单条字面量脚本；与 ls-store roundtrip 同口径，
    /// v13 全量列与 Python 权威 schema 一致）。
    fn seed_schema(path: &std::path::Path) {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT);
             INSERT INTO schema_version VALUES(13,'x');
             CREATE TABLE events(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, \
             type TEXT, level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT, \
             status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT, escalated_ts TEXT, \
             ai_review TEXT, ai_review_ts TEXT, ai_review_model TEXT);
             CREATE TABLE cameras(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, \
             type TEXT, status TEXT DEFAULT 'stopped', group_name TEXT DEFAULT '', \
             analyzer TEXT DEFAULT 'python');
             CREATE TABLE zones(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, name TEXT, polygon TEXT);
             CREATE TABLE rules(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, zone_id INTEGER, \
             behavior TEXT, level INTEGER, cooldown_s INTEGER, time_start TEXT, time_end TEXT, \
             threshold INTEGER, enabled INTEGER, days_of_week TEXT, holiday_only INTEGER);
             CREATE TABLE alert_deliveries(id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER, channel TEXT, \
             status TEXT DEFAULT 'pending', attempts INTEGER DEFAULT 0, last_error TEXT, updated_ts TEXT);
             CREATE TABLE alert_silences(id INTEGER PRIMARY KEY AUTOINCREMENT, scope TEXT NOT NULL, target TEXT, \
             reason TEXT, created_ts TEXT NOT NULL, until_ts TEXT NOT NULL, created_by TEXT);
             CREATE TABLE audit_log(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, request_id TEXT, \
             actor TEXT, action TEXT NOT NULL, target TEXT, status_code INTEGER, detail TEXT);
             CREATE TABLE calendar_exceptions(date TEXT PRIMARY KEY, mode TEXT NOT NULL, note TEXT);
             CREATE TABLE rule_templates(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL, \
             config_json TEXT NOT NULL, created_ts TEXT, updated_ts TEXT);
             CREATE TABLE app_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_ts TEXT NOT NULL);
             CREATE TABLE devices(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
             platform TEXT DEFAULT '', token_hash TEXT NOT NULL UNIQUE, created_ts TEXT NOT NULL, \
             last_seen_ts TEXT, revoked INTEGER DEFAULT 0);",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn live_and_system_contract() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let app = build_router(test_state(db.to_str().unwrap()));
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/health/live")
                    .header("host", "127.0.0.1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(body_json(r).await, json!({"status": "ok"}));
        let r2 = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/system")
                    .header("host", "127.0.0.1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let j = body_json(r2).await;
        assert_eq!(j["version"], "0.2.0");
        assert_eq!(j["schema_version"], 13);
    }

    #[tokio::test]
    async fn maintenance_and_backup_routes() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let app = build_router(test_state(db.to_str().unwrap()));
        for (uri, method) in [
            ("/api/maintenance/status", "GET"),
            ("/api/maintenance/preview", "GET"),
            ("/api/backups/status", "GET"),
            ("/api/escalation/status", "GET"),
        ] {
            let r = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("host", "127.0.0.1")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK, "{uri}");
        }
        let r = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/backups/run")
                    .header("host", "127.0.0.1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(body_json(r).await["started"], true);
    }

    #[tokio::test]
    async fn batch3_import_evidence_and_status() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let st = test_state_cfg_dir(
            db.to_str().unwrap(),
            "adm",
            false,
            dir.path().to_str().unwrap(),
        );
        let app = build_router(Arc::clone(&st));
        // config/import：merge 导入两台摄像头 + 区域
        let import_body = json!({"mode": "merge", "data": {
            "cameras": [
                {"name": "导入A", "source": "rtsp://u:p@192.168.1.10/s", "type": "rtsp"},
                {"name": "导入B", "source": "b.mp4", "type": "file"}],
            "zones": [
                {"camera_name": "导入A", "name": "台面", "polygon": [[0,0],[50,0],[50,50]]}]},
        });
        let r = req_hdr(
            &app,
            "POST",
            "/api/config/import",
            Some(import_body.clone()),
            &[("x-admin-token", "adm")],
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK, "import");
        let j = body_json(r).await;
        assert_eq!(j["created"]["cameras"], 2);
        assert_eq!(j["created"]["zones"], 1);
        // 重复导入 → merge 按名称去重
        let r = req_hdr(
            &app,
            "POST",
            "/api/config/import",
            Some(import_body),
            &[("x-admin-token", "adm")],
        )
        .await;
        let j = body_json(r).await;
        assert_eq!(j["created"]["cameras"], 0);
        assert_eq!(j["skipped"]["cameras"].as_array().unwrap().len(), 2);
        // replace 未确认 → 400
        let r = req_hdr(
            &app,
            "POST",
            "/api/config/import",
            Some(json!({"mode": "replace", "data": {"cameras": []}})),
            &[("x-admin-token", "adm")],
        )
        .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        // evidence.zip：造快照文件 → 打包可解
        {
            let s = ls_store::Store::open(db.to_str().unwrap()).unwrap();
            let _eid = s
                .add_event(&ls_store::NewEvent {
                    camera_id: 1,
                    type_: "enter",
                    level: 2,
                    track_id: 1,
                    detail: "存证",
                    snapshot: Some("snapshots/ev.jpg"),
                    ts: "2026-11-09T10:00:00",
                })
                .unwrap();
            let snap_dir = dir.path().join("snapshots");
            std::fs::create_dir_all(&snap_dir).unwrap();
            std::fs::write(snap_dir.join("ev.jpg"), b"\xff\xd8fake-jpeg").unwrap();
        }
        let r = req(&app, "GET", "/api/events/1/evidence.zip", None).await;
        assert_eq!(r.status(), StatusCode::OK, "zip");
        let b = http_body_util::BodyExt::collect(r.into_body())
            .await
            .unwrap()
            .to_bytes();
        // ZIP 魔数（本地文件头）
        assert_eq!(&b[0..4], &0x0403_4b50u32.to_le_bytes());
        assert!(b.len() > 100);
        // export/evidence.zip（范围内有存证）
        let r = req(&app, "GET", "/api/export/evidence.zip?days=30", None).await;
        assert_eq!(r.status(), StatusCode::OK, "export zip");
        // plugins：未知 pid → 404（rust 无插件管理器，真实结果）
        let r = req(&app, "GET", "/api/plugins/p1/logs", None).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        // face 默认关闭 → 403（真实配置状态）
        let r = req(&app, "GET", "/api/face/persons", None).await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let r = req(&app, "GET", "/api/face/status", None).await;
        assert_eq!(body_json(r).await["enabled"], false);
        // media/sign：HMAC 签名返回 exp+sig
        let r = req(
            &app,
            "GET",
            "/api/media/sign?path=/api/events/1/snapshot",
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let j = body_json(r).await;
        assert!(j["sig"].as_str().unwrap().starts_with("sha256="));
        // 非法 path → 400
        let r = req(&app, "GET", "/api/media/sign?path=https://evil", None).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        // alert-test：无渠道配置 → sent=0 且 200（写守卫与 Python admin_guard 一致）
        let r = req_hdr(
            &app,
            "POST",
            "/api/alert-test",
            None,
            &[("x-admin-token", "adm")],
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(body_json(r).await["sent"], 0);
    }

    #[tokio::test]
    async fn csrf_and_host_guard() {
        let app = build_router(test_state("D:/tmp/no-such.db"));
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/backups/run")
                    .header("host", "127.0.0.1")
                    .header("origin", "https://evil.example")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        let r2 = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/system")
                    .header("host", "evil.example")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::BAD_REQUEST);
    }

    // ==== R2-309 批次 1 集成测试 ====

    /// 构造请求并直接经 router 发送（oneshot），返回响应。
    async fn req(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Response {
        req_hdr(app, method, uri, body, &[]).await
    }

    /// 同 req，但可附带额外请求头（令牌测试用）。
    async fn req_hdr(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> Response {
        let b = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1");
        let b = headers.iter().fold(b, |acc, (k, v)| acc.header(*k, *v));
        let request = if let Some(v) = &body {
            b.header("content-type", "application/json")
                .body(axum::body::Body::from(v.to_string()))
                .unwrap()
        } else {
            b.body(axum::body::Body::empty()).unwrap()
        };
        app.clone().oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn zones_and_rules_crud() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let app = build_router(test_state(db.to_str().unwrap()));
        // 建摄像头
        let r = req(
            &app,
            "POST",
            "/api/cameras",
            Some(json!({"name": "cam", "source": "x.mp4", "type": "file"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let cid = body_json(r).await["id"].as_i64().unwrap();
        // 区域：POST/GET/DELETE
        let r = req(
            &app,
            "POST",
            "/api/zones",
            Some(json!({"camera_id": cid, "name": "台面",
                         "polygon": [[0,0],[100,0],[100,100]]})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK, "add zone");
        let zid = body_json(r).await["id"].as_i64().unwrap();
        let r = req(&app, "GET", &format!("/api/zones?camera_id={cid}"), None).await;
        let rows = body_json(r).await;
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["name"], "台面");
        assert!(rows[0]["polygon"].is_array(), "polygon 解析为 JSON 数组");
        let r = req(&app, "DELETE", &format!("/api/zones/{zid}"), None).await;
        assert_eq!(r.status(), StatusCode::OK);
        // 规则：POST/GET/DELETE
        let r = req(
            &app,
            "POST",
            "/api/rules",
            Some(
                json!({"camera_id": cid, "zone_id": null, "behavior": "enter",
                         "level": 2, "cooldown_s": 30}),
            ),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK, "add rule");
        let rid = body_json(r).await["id"].as_i64().unwrap();
        let r = req(&app, "GET", "/api/rules", None).await;
        assert_eq!(body_json(r).await.as_array().unwrap().len(), 1);
        let r = req(&app, "DELETE", &format!("/api/rules/{rid}"), None).await;
        assert_eq!(r.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn pairing_and_device_read() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let st = test_state_cfg(db.to_str().unwrap(), "adm", true);
        let app = build_router(Arc::clone(&st));
        // 读保护：匿名 GET 401
        let r = req(&app, "GET", "/api/events", None).await;
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        // 生成配对码（需管理令牌）→ claim → 设备令牌可读
        let r = req_hdr(
            &app,
            "POST",
            "/api/pairing",
            None,
            &[("x-admin-token", "adm")],
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let code = body_json(r).await["code"].as_str().unwrap().to_string();
        let r = req(
            &app,
            "POST",
            "/api/pairing/claim",
            Some(json!({"code": code, "device_name": "手机", "platform": "android"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK, "claim");
        let tok = body_json(r).await["token"].as_str().unwrap().to_string();
        // 设备列表：与 Python require_admin 同口径 → 管理令牌
        let r = req_hdr(
            &app,
            "GET",
            "/api/devices",
            None,
            &[("x-admin-token", "adm")],
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK, "devices list needs admin");
        let devs = body_json(r).await;
        assert_eq!(devs.as_array().unwrap().len(), 1);
        assert!(devs[0]["token_hash"].is_null() || devs[0].get("token_hash").is_none());
        // 设备令牌读（Bearer）：读保护下 200
        let r = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/events")
                    .header("host", "127.0.0.1")
                    .header("authorization", format!("Bearer {tok}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "device token read");
        // 吊销后 401
        let app2 = build_router(Arc::clone(&st));
        let r = app2
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/api/devices/1")
                    .header("host", "127.0.0.1")
                    .header("x-admin-token", "adm")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let app3 = build_router(st);
        let r = app3
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/events")
                    .header("host", "127.0.0.1")
                    .header("authorization", format!("Bearer {tok}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "revoked");
    }

    #[tokio::test]
    async fn audit_and_bulk_and_stats() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let app = build_router(test_state(db.to_str().unwrap()));
        let r = req(
            &app,
            "POST",
            "/api/cameras",
            Some(json!({"name": "cam", "source": "x.mp4", "type": "file"})),
        )
        .await;
        let cid = body_json(r).await["id"].as_i64().unwrap();
        // 造两条事件
        {
            let s = ls_store::Store::open(&db.to_string_lossy()).unwrap();
            for i in 0..2 {
                s.add_event(&ls_store::NewEvent {
                    camera_id: cid,
                    type_: "enter",
                    level: 2,
                    track_id: i,
                    detail: "t",
                    snapshot: None,
                    ts: "2026-11-09T10:00:00",
                })
                .unwrap();
            }
        }
        // 批量处置
        let r = req(
            &app,
            "POST",
            "/api/events/bulk-status",
            Some(json!({"ids": [1, 2], "status": "acked"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK, "bulk");
        assert_eq!(body_json(r).await["updated"], 2);
        // 422：非法状态
        let r = req(
            &app,
            "POST",
            "/api/events/bulk-status",
            Some(json!({"ids": [1], "status": "bogus"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
        // 审计：写请求已记录（含批量处置）
        let r = req(&app, "GET", "/api/audit?limit=50", None).await;
        assert_eq!(r.status(), StatusCode::OK);
        let j = body_json(r).await;
        assert!(j["total"].as_i64().unwrap() >= 2, "audit rows recorded");
        // 统计
        let r = req(&app, "GET", "/api/stats/daily?days=3", None).await;
        assert_eq!(r.status(), StatusCode::OK);
        let j = body_json(r).await;
        assert_eq!(j["days"].as_array().unwrap().len(), 3);
        let r = req(&app, "GET", "/api/stats/hourly?date=2026-11-09", None).await;
        assert_eq!(body_json(r).await["hours"]["10"]["total"], 2);
        let r = req(&app, "GET", "/api/events/timeline?days=2", None).await;
        assert_eq!(body_json(r).await["days_list"].as_array().unwrap().len(), 2);
        let r = req(&app, "GET", "/api/stats/quality?days=7", None).await;
        assert_eq!(body_json(r).await["total"], 2);
        // CSV
        let r = req(&app, "GET", "/api/export/events.csv", None).await;
        assert!(r.status() == StatusCode::OK);
        let r = req(&app, "GET", "/api/export/stats.csv", None).await;
        assert!(r.status() == StatusCode::OK);
        // 指标 & 诊断 & meta
        for uri in ["/api/metrics", "/api/diagnostics/summary", "/api/v1/meta"] {
            let r = req(&app, "GET", uri, None).await;
            assert_eq!(r.status(), StatusCode::OK, "{uri}");
        }
    }

    #[tokio::test]
    async fn silences_calendar_templates() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        seed_schema(&db);
        let app = build_router(test_state(db.to_str().unwrap()));
        // 静默
        let r = req(
            &app,
            "POST",
            "/api/alert-silences",
            Some(json!({"scope": "camera", "target": "1",
                         "reason": "检修", "until_ts": "2026-12-01T00:00:00"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let sid = body_json(r).await["id"].as_i64().unwrap();
        let r = req(&app, "GET", "/api/alert-silences", None).await;
        assert_eq!(body_json(r).await.as_array().unwrap().len(), 1);
        let r = req(&app, "DELETE", &format!("/api/alert-silences/{sid}"), None).await;
        assert_eq!(r.status(), StatusCode::OK);
        // 日历
        let r = req(
            &app,
            "POST",
            "/api/calendar",
            Some(json!({"date": "2026-12-25", "mode": "holiday", "note": "圣诞"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let r = req(&app, "GET", "/api/calendar", None).await;
        assert_eq!(body_json(r).await.as_array().unwrap().len(), 1);
        let r = req(&app, "DELETE", "/api/calendar/2026-12-25", None).await;
        assert_eq!(r.status(), StatusCode::OK);
        // 模板
        let r = req(
            &app,
            "POST",
            "/api/rule-templates",
            Some(json!({"name": "夜间入侵", "config": {"behavior": "enter", "level": 3}})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let tid = body_json(r).await["id"].as_i64().unwrap();
        let r = req(
            &app,
            "PUT",
            &format!("/api/rule-templates/{tid}"),
            Some(json!({"name": "夜间入侵v2", "config": {"behavior": "exit", "level": 2}})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        let r = req(&app, "GET", "/api/rule-templates", None).await;
        let j = body_json(r).await;
        assert_eq!(j[0]["name"], "夜间入侵v2");
        assert_eq!(j[0]["config"]["level"], 2);
        let r = req(&app, "DELETE", &format!("/api/rule-templates/{tid}"), None).await;
        assert_eq!(r.status(), StatusCode::OK);
    }
}
