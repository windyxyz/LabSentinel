use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use axum::{response::Response, Router};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tower::ServiceExt;

use super::helpers::{body_json, seed_schema, test_state_cfg};
use crate::build_router;

const LOCAL: &str = "127.0.0.1";
const OLD_PASSWORD: &str = "local-test-password-alpha";
const NEW_PASSWORD: &str = "local-test-password-bravo";

async fn request(
    app: &Router,
    peer: &str,
    host: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Response {
    let peer = (peer.parse::<std::net::IpAddr>().unwrap(), 25000);
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, host)
        .extension(ConnectInfo(SocketAddr::from(peer)));
    for &(key, value) in headers {
        request = request.header(key, value);
    }
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn local(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Response {
    request(app, LOCAL, LOCAL, method, path, headers, body).await
}

fn cookie(response: &Response, is_secure: bool) -> String {
    assert_eq!(response.status(), StatusCode::OK);
    let value = response.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(value.contains("; HttpOnly; SameSite=Strict; Max-Age=43200"));
    assert_eq!(value.contains("; Secure"), is_secure);
    value.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn local_password_works_with_api_credentials_and_rotates_browser_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("access.sqlite");
    seed_schema(&db);
    let db = db.to_str().unwrap();
    let st = test_state_cfg(db, "legacy-test-admin", true);
    let app = build_router(st.clone());
    let admin = [("x-admin-token", "legacy-test-admin")];
    let signing = local(
        &app,
        "GET",
        "/api/media/sign?path=/api/events/1/snapshot",
        &admin,
        None,
    )
    .await;
    assert_eq!(signing.status(), StatusCode::OK);
    let signed = body_json(signing).await;
    let media = format!(
        "/api/events/1/snapshot?exp={}&sig={}",
        signed["exp"].as_str().unwrap(),
        signed["sig"].as_str().unwrap()
    );
    let saved = local(
        &app,
        "PUT",
        "/api/access/settings",
        &admin,
        Some(json!({
            "local": true, "lan": false, "password": OLD_PASSWORD,
        })),
    )
    .await;
    let first = cookie(&saved, false);
    let first_headers = [("cookie", first.as_str())];
    let stored = ls_store::Store::open(db)
        .unwrap()
        .setting_get("access.password")
        .unwrap()
        .unwrap();
    assert!(stored.contains("$argon2id$"));
    assert!(!stored.contains(OLD_PASSWORD));
    for path in [
        "/api/system",
        "/api/access/settings",
        "/ws/events",
        media.as_str(),
    ] {
        let denied = local(&app, "GET", path, &[], None).await;
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
    assert_eq!(
        local(&app, "GET", "/api/system", &admin, None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        local(&app, "GET", "/api/system", &first_headers, None)
            .await
            .status(),
        StatusCode::OK
    );
    let second = local(
        &app,
        "POST",
        "/api/access/login",
        &[],
        Some(json!({"password": OLD_PASSWORD})),
    )
    .await;
    let second_cookie = cookie(&second, false);
    let revoked_by_cross_origin = local(
        &app,
        "POST",
        "/api/access/revoke-others",
        &[
            ("cookie", first.as_str()),
            ("origin", "https://other.example"),
        ],
        None,
    )
    .await;
    assert_eq!(revoked_by_cross_origin.status(), StatusCode::FORBIDDEN);
    let revoke = local(
        &app,
        "POST",
        "/api/access/revoke-others",
        &first_headers,
        None,
    )
    .await;
    assert_eq!(revoke.status(), StatusCode::OK);
    let invalid = local(
        &app,
        "GET",
        "/api/system",
        &[("cookie", second_cookie.as_str())],
        None,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    let rotated = local(
        &app,
        "PUT",
        "/api/access/settings",
        &first_headers,
        Some(json!({
            "local": true, "lan": false, "current_password": OLD_PASSWORD, "password": NEW_PASSWORD,
        })),
    )
    .await;
    let current = cookie(&rotated, false);
    let old_session = local(&app, "GET", "/api/system", &first_headers, None).await;
    assert_eq!(old_session.status(), StatusCode::UNAUTHORIZED);
    let old_credential = local(
        &app,
        "POST",
        "/api/access/login",
        &[],
        Some(json!({"password": OLD_PASSWORD})),
    )
    .await;
    assert_eq!(old_credential.status(), StatusCode::UNAUTHORIZED);
    let logged_out = local(
        &app,
        "POST",
        "/api/access/logout",
        &[("cookie", current.as_str())],
        None,
    )
    .await;
    assert_eq!(logged_out.status(), StatusCode::OK);
    assert!(logged_out.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert_eq!(
        local(
            &app,
            "GET",
            "/api/system",
            &[("cookie", current.as_str())],
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );

    let restarted = test_state_cfg(db, "legacy-test-admin", true);
    restarted.access.load(db);
    let app = build_router(restarted);
    let after_restart = local(&app, "GET", "/api/access/status", &[], None).await;
    assert_eq!(body_json(after_restart).await["required"], true);
}

#[tokio::test]
async fn lan_password_accepts_https_from_the_configured_loopback_proxy() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("proxy.sqlite");
    seed_schema(&db);
    let mut st = test_state_cfg(db.to_str().unwrap(), "", false);
    let proxy_host = "app.example.test";
    std::sync::Arc::get_mut(&mut st)
        .unwrap()
        .trusted_hosts
        .push(proxy_host.into());
    let app = build_router(st);
    let setup = local(
        &app,
        "PUT",
        "/api/access/settings",
        &[],
        Some(json!({
            "local": true, "lan": true, "password": OLD_PASSWORD, "trust_proxy": true,
        })),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);

    let proxy_headers = [
        ("x-forwarded-for", "192.0.2.16"),
        ("x-forwarded-proto", "https"),
    ];
    let payload = json!({"password": OLD_PASSWORD});
    let rejected = request(
        &app,
        "192.0.2.16",
        proxy_host,
        "POST",
        "/api/access/login",
        &proxy_headers,
        Some(payload.clone()),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    let plaintext = request(
        &app,
        LOCAL,
        proxy_host,
        "POST",
        "/api/access/login",
        &[],
        Some(payload.clone()),
    )
    .await;
    assert_eq!(plaintext.status(), StatusCode::FORBIDDEN);
    let logged_in = request(
        &app,
        LOCAL,
        proxy_host,
        "POST",
        "/api/access/login",
        &proxy_headers,
        Some(payload),
    )
    .await;
    let session = cookie(&logged_in, true);
    let authenticated_proxy = [
        proxy_headers[0],
        proxy_headers[1],
        ("cookie", session.as_str()),
    ];
    let ok = request(
        &app,
        LOCAL,
        proxy_host,
        "GET",
        "/api/access/settings",
        &authenticated_proxy,
        None,
    )
    .await;
    assert_eq!(ok.status(), StatusCode::OK);
    let direct = local(
        &app,
        "GET",
        "/api/system",
        &[("cookie", session.as_str())],
        None,
    )
    .await;
    assert_eq!(direct.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn recovery_is_local_one_time_and_keeps_the_lan_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("recovery.sqlite");
    seed_schema(&db);
    let db = db.to_str().unwrap();
    let st = test_state_cfg(db, "", false);
    let app = build_router(st);
    let saved = local(
        &app,
        "PUT",
        "/api/access/settings",
        &[],
        Some(json!({
            "local": false, "lan": true, "password": OLD_PASSWORD,
        })),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::OK);
    let code = crate::access::recovery_code(db).unwrap();
    let payload = json!({"code": code, "password": NEW_PASSWORD});
    let rejected = request(
        &app,
        "192.0.2.20",
        LOCAL,
        "POST",
        "/api/access/recover",
        &[],
        Some(payload.clone()),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    let recovered = local(
        &app,
        "POST",
        "/api/access/recover",
        &[],
        Some(payload.clone()),
    )
    .await;
    let session = cookie(&recovered, false);
    let settings = local(
        &app,
        "GET",
        "/api/access/settings",
        &[("cookie", session.as_str())],
        None,
    )
    .await;
    assert_eq!(body_json(settings).await["lan"], true);
    let reused = local(&app, "POST", "/api/access/recover", &[], Some(payload)).await;
    assert_eq!(reused.status(), StatusCode::UNAUTHORIZED);
}
