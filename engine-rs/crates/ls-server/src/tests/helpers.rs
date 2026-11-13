// A4（2026-11-12）：从 main.rs 的 #[cfg(test)] mod tests 迁出。
// 根门面 re-export（handlers/state/util/router）+ 显式 axum/serde 类型导入。
use crate::*;
use axum::response::Response;
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;
pub(crate) fn test_state(db: &str) -> Shared {
    test_state_cfg(db, "", false)
}

pub(crate) fn test_state_cfg(db: &str, admin_token: &str, read_protected: bool) -> Shared {
    test_state_cfg_dir(db, admin_token, read_protected, "D:/tmp")
}

pub(crate) fn test_state_cfg_dir(
    db: &str,
    admin_token: &str,
    read_protected: bool,
    data_dir: &str,
) -> Shared {
    test_state_custom(db, admin_token, read_protected, data_dir, true, "")
}

/// 变体：可指定 testing（R2-108 回归用——生产态 testserver 必须被拒）与
/// web_dist（静态服务回归用）；空串 = 不挂载静态服务。
pub(crate) fn test_state_custom(
    db: &str,
    admin_token: &str,
    read_protected: bool,
    data_dir: &str,
    testing: bool,
    web_dist: &str,
) -> Shared {
    Arc::new(AppState {
        db_path: db.into(),
        admin_token: admin_token.into(),
        read_token: String::new(),
        read_protected,
        trusted_hosts: vec!["127.0.0.1".into(), "localhost".into()],
        allowed_origins: vec![],
        web_dist: web_dist.into(),
        data_dir: data_dir.into(),
        version: "0.2.0".into(),
        events_tx: broadcast::channel(16).0,
        jobs: Arc::new(ls_jobs::Jobs::new(
            std::path::PathBuf::from(db),
            ls_jobs::JobsConfig::default_for_db(std::path::Path::new(db)),
        )),
        pair_state: std::sync::Mutex::new(PairState::default()),
        testing,
        workers: crate::worker::Workers::default(),
        mqtt: None,
    })
}

pub(crate) async fn body_json(resp: Response) -> Value {
    let b = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&b).unwrap()
}

/// 测试用最小 schema（单条字面量脚本；与 ls-store roundtrip 同口径，
/// v13 全量列与 Python 权威 schema 一致）。
pub(crate) fn seed_schema(path: &std::path::Path) {
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

/// 构造请求并直接经 router 发送（oneshot），返回响应。
pub(crate) async fn req(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Response {
    req_hdr(app, method, uri, body, &[]).await
}

/// 同 req，但可附带额外请求头（令牌测试用）。
pub(crate) async fn req_hdr(
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
