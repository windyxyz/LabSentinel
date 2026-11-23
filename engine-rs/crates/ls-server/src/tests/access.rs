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

#[tokio::test]
async fn browser_pairing_scopes_http_and_closes_a_socket_when_authorization_changes() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices.sqlite");
    let db_path = path.to_str().unwrap();
    let store = ls_store::Store::open(db_path).unwrap();
    store.ensure_schema().unwrap();
    let first = store
        .conn_add_camera("Room A", "example-a.mp4", "file")
        .unwrap();
    let second = store
        .conn_add_camera("Room B", "example-b.mp4", "file")
        .unwrap();
    store.set_camera_group(first, "Room A").unwrap();
    let add_event = |camera_id| {
        store
            .add_event(&ls_store::NewEvent {
                camera_id,
                type_: "enter",
                level: 2,
                track_id: 1,
                detail: "test event",
                snapshot: None,
                ts: &crate::util::iso_now(),
            })
            .unwrap()
    };
    let first_event = add_event(first);
    let second_event = add_event(second);
    let st = test_state_cfg(db_path, "fixture-admin", true);
    let app = build_router(st.clone());
    let admin = [("x-admin-token", "fixture-admin")];
    let created = local(
        &app,
        "POST",
        "/api/pairing",
        &admin,
        Some(json!({
            "role": "read_only", "all_cameras": false, "groups": ["Room A"], "expires_days": 30,
        })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::OK);
    let code = body_json(created).await["code"].clone();
    let payload = json!({"code": code, "device_name": "Test tablet", "platform": "web"});
    let claim = local(
        &app,
        "POST",
        "/api/pairing/claim",
        &[],
        Some(payload.clone()),
    )
    .await;
    assert_eq!(claim.status(), StatusCode::OK);
    let set_cookie = claim.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly; SameSite=Strict"));
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    assert!(cookie.starts_with("ls_device="));
    assert!(body_json(claim).await.get("token").is_none());
    let replay = local(&app, "POST", "/api/pairing/claim", &[], Some(payload)).await;
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
    let paired = [("cookie", cookie.as_str())];

    let cameras = body_json(local(&app, "GET", "/api/cameras", &paired, None).await).await;
    let cameras = cameras.as_array().unwrap();
    assert_eq!(cameras.len(), 1);
    assert_eq!(cameras[0]["id"], first);
    assert!(cameras[0].get("source").is_none());
    let events = body_json(local(&app, "GET", "/api/events", &paired, None).await).await;
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["id"], first_event);
    let daily = body_json(local(&app, "GET", "/api/stats/daily?days=1", &paired, None).await).await;
    assert_eq!(daily["days"][0]["l2"], 1);
    for denied in [
        "/api/system".to_owned(),
        format!("/api/cameras/{second}/snapshot"),
        format!("/ws/detections/{second}"),
    ] {
        assert_eq!(
            local(&app, "GET", &denied, &paired, None).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    let status_path = format!("/api/events/{first_event}/status");
    let ack = json!({"status": "acked"});
    assert_eq!(
        local(&app, "POST", &status_path, &paired, Some(ack.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let now = body_json(local(&app, "GET", "/api/access/status", &paired, None).await).await;
    let device_id = now["device"]["id"].as_i64().unwrap();

    let listener = tokio::net::TcpListener::bind((LOCAL, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let service = app
        .clone()
        .into_make_service_with_connect_info::<SocketAddr>();
    let host = tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });
    let socket_request = || {
        let mut request = format!("ws://{address}/ws/events")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        request
    };
    let (mut socket, _) = tokio_tungstenite::connect_async(socket_request())
        .await
        .unwrap();
    st.events_tx
        .send(json!({"id": second_event, "camera_id": second}).to_string())
        .unwrap();
    st.events_tx
        .send(json!({"id": first_event, "camera_id": first}).to_string())
        .unwrap();
    let visible = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap();
    let Some(Ok(Message::Text(visible))) = visible else {
        panic!("expected one authorized event")
    };
    assert_eq!(
        serde_json::from_str::<Value>(&visible).unwrap()["camera_id"],
        first
    );

    let device_path = format!("/api/devices/{device_id}");
    let edited = local(&app, "PUT", &device_path, &admin, Some(json!({
        "name": "Shift tablet", "role": "triage", "all_cameras": false, "camera_ids": [second], "expires_days": 30,
    }))).await;
    assert_eq!(edited.status(), StatusCode::OK);
    let ended = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap();
    assert!(!matches!(ended, Some(Ok(Message::Text(_)))));
    assert_eq!(
        local(&app, "POST", &status_path, &paired, Some(ack.clone()))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        local(
            &app,
            "POST",
            &format!("/api/events/{second_event}/status"),
            &paired,
            Some(ack)
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        local(&app, "DELETE", &device_path, &admin, None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        local(&app, "GET", "/api/cameras", &paired, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(tokio_tungstenite::connect_async(socket_request())
        .await
        .is_err());
    host.abort();
}
