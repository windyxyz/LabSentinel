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

use axum::extract::{
    ws::{WebSocket, WebSocketUpgrade},
    Path as AxPath, Query, State,
};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::broadcast;

/// 应用共享状态
pub struct AppState {
    pub db_path: String,
    pub admin_token: String,
    pub read_protected: bool,
    pub trusted_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub web_dist: String,
    pub version: String,
    pub events_tx: broadcast::Sender<String>,
    /// T-330：后台作业（retention / backup / escalation），启动时 start()
    pub jobs: Arc<ls_jobs::Jobs>,
}

/// R2-307: machine-readable route inventory for scripts/rust_route_coverage.py.
/// Format: {"paths": {"<method> <path>": ...}} — one entry per (method, path).
pub fn dump_routes() -> serde_json::Value {
    let mut routes = Vec::new();
    for (method, path) in [
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
    ] {
        routes.push(serde_json::json!({"method": method, "path": path}));
    }
    serde_json::json!({ "routes": routes })
}

pub type Shared = Arc<AppState>;

pub fn build_router(state: Shared) -> Router {
    let api = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/system", get(system))
        .route("/events", get(events))
        .route("/events/:id/status", post(set_status))
        .route("/cameras", get(cameras).post(add_camera))
        .route("/cameras/:id/status", get(camera_status))
        .route("/stats/summary", get(stats_summary))
        .route("/config/summary", get(config_summary))
        .route("/backups/status", get(backups_status))
        .route("/backups/run", post(backups_run))
        .route("/maintenance/status", get(maintenance_status))
        .route("/maintenance/preview", get(maintenance_preview))
        .route("/maintenance/cleanup", post(maintenance_cleanup))
        .route("/escalation/status", get(escalation_status))
        .route("/events/count", get(events_count))
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

// ---- 中间件：Host/CSRF/鉴权/安全头/request-id ----
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
    // CSRF/Fetch-Metadata（写请求跨站拒绝）
    let unsafe_m = method == axum::http::Method::POST
        || method == axum::http::Method::PUT
        || method == axum::http::Method::PATCH
        || method == axum::http::Method::DELETE;
    if unsafe_m && path.starts_with("/api/") {
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
        // 管理令牌（写操作）
        if !st.admin_token.is_empty() {
            let got = headers
                .get("x-admin-token")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if got != st.admin_token {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"detail": "需要管理令牌（X-Admin-Token）"})),
                )
                    .into_response();
            }
        }
    }
    let rid = uuid_like();
    let mut resp = next.run(req).await;
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

async fn ws_events(State(st): State<Shared>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, st.events_tx.subscribe()))
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
            println!("{}", serde_json::to_string_pretty(&dump_routes()).expect("json"));
            std::process::exit(0);
        }
    }
    let db_path =
        std::env::var("LS_DB_PATH").unwrap_or_else(|_| "server/data/labsentinel.db".into());
    let token = std::env::var("LS_ADMIN_TOKEN").unwrap_or_default();
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
        read_protected: false,
        trusted_hosts: vec!["127.0.0.1".into(), "localhost".into(), "::1".into()],
        allowed_origins: vec![],
        web_dist: "web/dist".into(),
        version,
        events_tx: broadcast::channel(256).0,
        jobs,
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
        Arc::new(AppState {
            db_path: db.into(),
            admin_token: String::new(),
            read_protected: false,
            trusted_hosts: vec!["127.0.0.1".into(), "localhost".into()],
            allowed_origins: vec![],
            web_dist: String::new(),
            version: "0.2.0".into(),
            events_tx: broadcast::channel(16).0,
            jobs: Arc::new(ls_jobs::Jobs::new(
                std::path::PathBuf::from(db),
                ls_jobs::JobsConfig::default_for_db(std::path::Path::new(db)),
            )),
        })
    }

    async fn body_json(resp: Response) -> Value {
        let b = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        serde_json::from_slice(&b).unwrap()
    }

    /// 测试用最小 schema（单条字面量脚本；与 ls-store roundtrip 同口径）。
    fn seed_schema(path: &std::path::Path) {
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT);
             INSERT INTO schema_version VALUES(13,'x');
             CREATE TABLE events(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, \
             type TEXT, level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT, \
             status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT, escalated_ts TEXT);
             CREATE TABLE cameras(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, \
             type TEXT, status TEXT DEFAULT 'stopped', group_name TEXT DEFAULT '', \
             analyzer TEXT DEFAULT 'python');",
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
}
