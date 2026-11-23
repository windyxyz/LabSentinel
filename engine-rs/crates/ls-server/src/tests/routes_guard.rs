// A4（2026-11-12）：从 main.rs 的 #[cfg(test)] mod tests 迁出。
// 根门面 re-export（handlers/state/util/router）+ 显式 axum/serde 类型导入。
use crate::tests::helpers::*;
use crate::*;
use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn analysis_source_settings_persist_without_exposing_stream_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("analysis.db");
    let path = db.to_str().unwrap();
    let store = ls_store::Store::open(path).unwrap();
    store.ensure_schema().unwrap();
    let cid = store
        .conn_add_camera("cam", "rtsp://localhost/main", "rtsp")
        .unwrap();
    let app = build_router(test_state(path));
    let endpoint = format!("/api/cameras/{cid}/analysis-source");
    let substream = "rtsp://operator:fixture-secret@localhost/sub";
    assert_eq!(
        req(
            &app,
            "PUT",
            &endpoint,
            Some(json!({"mode":"substream","source":"file:///video.mp4"}))
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let saved = body_json(
        req(
            &app,
            "PUT",
            &endpoint,
            Some(json!({"mode":"substream","source":substream})),
        )
        .await,
    )
    .await;
    assert_eq!(saved["analysis_mode"], "substream");
    assert_eq!(
        store.camera(cid).unwrap().unwrap().analysis_source,
        substream
    );
    let visible = body_json(req(&app, "GET", "/api/cameras", None).await).await;
    assert_eq!(visible[0]["analysis_mode"], "substream");
    assert!(!visible.to_string().contains("fixture-secret"));
    let relay = body_json(req(&app, "PUT", &endpoint, Some(json!({"mode":"relay"}))).await).await;
    assert_eq!(relay["analysis_mode"], "relay");
    let camera = store.camera(cid).unwrap().unwrap();
    assert!(camera.analysis_shared);
    assert!(camera.analysis_source.is_empty());
}

#[tokio::test]
async fn dahua_create_persists_main_and_substream_without_echoing_password() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("dahua.db");
    let path = database.to_str().unwrap();
    let store = ls_store::Store::open(path).unwrap();
    store.ensure_schema().unwrap();
    let app = build_router(test_state(path));
    let result = req(
        &app,
        "POST",
        "/api/cameras",
        Some(json!({
            "name": "NVR", "type": "rtsp", "group_name": "Lab",
            "dahua": {"host":"192.168.1.10","port":554,"channel":3,
                      "username":"operator","password":"fixture-secret","substream":true}
        })),
    )
    .await;
    assert_eq!(result.status(), StatusCode::OK);
    let created = body_json(result).await;
    assert!(!created.to_string().contains("fixture-secret"));
    let camera = store
        .camera(created["id"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    assert!(camera.source.ends_with("channel=3&subtype=0"));
    assert!(camera.analysis_source.ends_with("channel=3&subtype=1"));
    assert_eq!(camera.group_name, "Lab");
}

#[tokio::test]
async fn dataset_export_contains_only_verified_samples_with_normalized_labels() {
    use std::io::Read;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("dataset.db");
    let store = ls_store::Store::open(db.to_str().unwrap()).unwrap();
    store.ensure_schema().unwrap();
    let pool = dir.path().join("dataset_pool");
    std::fs::create_dir(&pool).unwrap();
    let jpeg = ls_analyzer::evidence::encode_jpeg_bgr(&vec![100; 100 * 100 * 3], 100, 100).unwrap();
    let boxes = r#"[{"class":"person","x1":25,"y1":25,"x2":75,"y2":75}]"#;
    for n in 0..6 {
        let name = format!("sample{n}.jpg");
        std::fs::write(pool.join(&name), &jpeg).unwrap();
        let id = store
            .add_dataset_sample(
                "event",
                &n.to_string(),
                None,
                &format!("dataset_pool/{name}"),
                100,
                100,
                boxes,
            )
            .unwrap();
        if n < 5 {
            store.update_dataset_sample(id, boxes, "verified").unwrap();
        }
    }
    let app = build_router(test_state_cfg_dir(
        db.to_str().unwrap(),
        "",
        false,
        dir.path().to_str().unwrap(),
    ));
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/dataset/export.zip")
                .header("host", "localhost")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<_> = zip
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    assert_eq!(
        names
            .iter()
            .filter(|name| name.starts_with("images/train/"))
            .count(),
        4
    );
    assert_eq!(
        names
            .iter()
            .filter(|name| name.starts_with("images/val/"))
            .count(),
        1
    );
    assert!(!names.iter().any(|name| name.ends_with("event_6.jpg")));
    let label_name = names
        .iter()
        .find(|name| name.starts_with("labels/val/"))
        .unwrap();
    let mut label = String::new();
    zip.by_name(label_name)
        .unwrap()
        .read_to_string(&mut label)
        .unwrap();
    assert_eq!(label, "7 0.500000 0.500000 0.500000 0.500000");
    assert!(zip.by_name("data.yaml").is_ok());
}

#[tokio::test]
async fn share_links_enforce_read_only_expiry_and_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("share.db");
    let store = ls_store::Store::open(db.to_str().unwrap()).unwrap();
    store.ensure_schema().unwrap();
    let token = "unit-test-share-token";
    let hash = crate::util::sha256_hex(token.as_bytes());
    let id = store
        .add_share_link(
            "test",
            &hash,
            24,
            "2026-01-01T00:00:00",
            "9999-01-01T00:00:00",
        )
        .unwrap();
    store.add_device("test", "share", &hash).unwrap();
    let app = build_router(test_state_cfg(db.to_str().unwrap(), "test-admin", true));
    let request = |method: &str, uri: &str| {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1")
            .header("x-admin-token", token)
            .body(axum::body::Body::empty())
            .unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(request("GET", "/api/cameras"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.clone()
            .oneshot(request("POST", "/api/events/1/status"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(request("GET", "/api/diagnostics/bundle.zip"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    store.revoke_share_link(id).unwrap();
    assert_eq!(
        app.oneshot(request("GET", "/api/cameras"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn detections_socket_enforces_host_and_read_token_and_delivers_frames() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("detections.db");
    let st = test_state_cfg(db.to_str().unwrap(), "test-admin", true);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!("ws://{}/ws/detections/42", listener.local_addr().unwrap());
    let app = build_router(st.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut invalid = url.clone().into_client_request().unwrap();
    invalid
        .headers_mut()
        .insert("host", "invalid.example".parse().unwrap());
    for (request, status) in [
        (invalid, 400),
        (url.clone().into_client_request().unwrap(), 401),
    ] {
        match tokio_tungstenite::connect_async(request).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                assert_eq!(response.status().as_u16(), status)
            }
            other => panic!("expected HTTP rejection, got {other:?}"),
        }
    }
    let mut authorized = url.into_client_request().unwrap();
    authorized.headers_mut().insert(
        "sec-websocket-protocol",
        "labsentinel, bearer.test-admin".parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(authorized).await.unwrap();
    assert!(st.detections.subscribed(99).is_none());
    let other = st.detections.subscribe(99);
    let _other_receiver = other.subscribe();
    other.send(json!({"camera_id": 99}).to_string()).unwrap();
    let frame = json!({"camera_id":42,"ts":"2026-11-22T08:00:00","img_w":1280,"img_h":720,"tracks":[],"zones":[]});
    st.detections
        .subscribed(42)
        .unwrap()
        .send(frame.to_string())
        .unwrap();
    let received = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(received.to_text().unwrap()).unwrap(),
        frame
    );
    server.abort();
}

#[tokio::test]
async fn sensitivity_preset_updates_person_and_ppe_thresholds() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("preset.db");
    seed_schema(&db);
    let state = test_state_cfg(db.to_str().unwrap(), "", false);
    let app = build_router(state);
    let response = req(
        &app,
        "PUT",
        "/api/detect/config",
        Some(json!({"sensitivity": "sensitive"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let store = ls_store::Store::open(db.to_str().unwrap()).unwrap();
    let camera = store.conn_add_camera("preset", "fixture", "file").unwrap();
    let config = crate::worker::WorkerConfig::from_db(&store, camera)
        .unwrap()
        .unwrap();
    assert_eq!(config.detector.conf, 0.32);
    assert_eq!(crate::worker::ppe_thresholds(&store, true), (0.42, 0.40));
    let response = req(&app, "GET", "/api/detect/config", None).await;
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let prefs: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(prefs["sensitivity"], "sensitive");

    let bad = req(
        &app,
        "PUT",
        "/api/detect/config",
        Some(json!({"sensitivity": 4})),
    )
    .await;
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        store.setting_get("detector.conf").unwrap().as_deref(),
        Some("0.32")
    );
}

#[tokio::test]
async fn events_socket_replays_committed_ids_in_order_after_reconnect() {
    use futures_util::StreamExt;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("replay.db");
    let st = test_state_cfg(db.to_str().unwrap(), "", false);
    let store = ls_store::Store::open(&st.db_path).unwrap();
    store.ensure_schema().unwrap();
    let add = |camera_id| {
        store
            .add_event(&ls_store::NewEvent {
                camera_id,
                type_: "enter",
                level: 2,
                track_id: 0,
                detail: "replay fixture",
                snapshot: None,
                ts: "2026-11-23T12:00:00",
            })
            .unwrap()
    };
    let first = add(1);
    let since = add(2);
    let second = add(1);
    let third = add(2);
    let scoped = store.events_after_id(first, Some(&[1]), 64).unwrap();
    assert_eq!(
        scoped.iter().map(|event| event.id).collect::<Vec<_>>(),
        vec![second]
    );

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!(
        "ws://{}/ws/events?since={since}",
        listener.local_addr().unwrap()
    );
    let app = build_router(st.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let next_id = async |socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
                         key: &str| {
        let message = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str::<serde_json::Value>(message.to_text().unwrap()).unwrap()[key]
            .as_i64()
            .unwrap()
    };
    assert_eq!(next_id(&mut socket, "stream_cursor").await, since);
    assert_eq!(next_id(&mut socket, "id").await, second);
    assert_eq!(next_id(&mut socket, "id").await, third);
    let fourth = add(1);
    let fifth = add(2);
    st.events_tx.send(json!({"id": fifth}).to_string()).unwrap();
    assert_eq!(next_id(&mut socket, "id").await, fourth);
    assert_eq!(next_id(&mut socket, "id").await, fifth);
    server.abort();
}

#[tokio::test]
async fn revoking_a_share_ends_an_open_events_socket() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("share-ws.db");
    let st = test_state_cfg(db.to_str().unwrap(), "test-admin", true);
    let store = ls_store::Store::open(&st.db_path).unwrap();
    store.ensure_schema().unwrap();
    let token = "revocable-share";
    let hash = crate::util::sha256_hex(token.as_bytes());
    let id = store
        .add_share_link(
            "wall",
            &hash,
            24,
            "2026-01-01T00:00:00",
            "2099-01-01T00:00:00",
        )
        .unwrap();
    store.add_device("wall", "share", &hash).unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let mut request = format!("ws://{}/ws/events", listener.local_addr().unwrap())
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "labsentinel, bearer.revocable-share".parse().unwrap(),
    );
    let app = build_router(st.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while st.events_tx.receiver_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    st.events_tx.send("before revocation".into()).unwrap();
    let normal = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    assert!(matches!(normal, Some(Ok(Message::Text(_)))));

    store.revoke_share_link(id).unwrap();
    let closed = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap();
    assert!(matches!(
        closed,
        None | Some(Ok(Message::Close(_))) | Some(Err(_))
    ));
    server.abort();
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
    assert_eq!(j["schema_version"], 14);
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
                ts: &format!("{}T10:00:00", crate::util::iso_days_ago(1)),
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
    // 人员名单要求管理权限，默认关闭时管理员仍收到配置提示。
    let r = req(&app, "GET", "/api/face/persons", None).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = req_hdr(
        &app,
        "GET",
        "/api/face/persons",
        None,
        &[("x-admin-token", "adm")],
    )
    .await;
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
    assert_eq!(rows[0]["leave_grace_frames"], 12);
    let r = req(
        &app,
        "PUT",
        &format!("/api/zones/{zid}"),
        Some(json!({
            "anchor_mode": "upper_body", "margin_px": 8, "leave_grace_frames": 5
        })),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let r = req(&app, "GET", &format!("/api/zones?camera_id={cid}"), None).await;
    let rows = body_json(r).await;
    assert_eq!(rows[0]["anchor_mode"], "upper_body");
    assert_eq!(rows[0]["margin_px"], 8);
    assert_eq!(rows[0]["leave_grace_frames"], 5);
    assert_eq!(rows[0]["polygon"], json!([[0, 0], [100, 0], [100, 100]]));
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
async fn spatial_masks_and_false_positive_spots_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("spatial.db");
    seed_schema(&db);
    let store = ls_store::Store::open(db.to_str().unwrap()).unwrap();
    let cid = store.conn_add_camera("camera", "x.mp4", "file").unwrap();
    let second_camera = store.conn_add_camera("other", "y.mp4", "file").unwrap();
    let app = build_router(test_state(db.to_str().unwrap()));
    let bounds = [30.0, 35.0, 100.0, 110.0];
    let eid = store
        .add_event_with_evidence(
            &ls_store::NewEvent {
                camera_id: cid,
                type_: "no_mask",
                level: 2,
                track_id: 9,
                detail: "fixture",
                snapshot: None,
                ts: "2026-11-24T12:00:00",
            },
            ls_store::EventEvidence {
                bbox: Some(bounds),
                ..Default::default()
            },
        )
        .unwrap();
    let remember = format!("/api/events/{eid}/remember-spot");
    assert_eq!(
        req(&app, "POST", &remember, None).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    store
        .set_event_status(eid, "false_positive", None, None)
        .unwrap();
    let first = body_json(req(&app, "POST", &remember, None).await).await;
    let second = body_json(req(&app, "POST", &remember, None).await).await;
    let spot_id = first["id"].as_i64().unwrap();
    assert_eq!(first["id"], second["id"]);
    let mask = body_json(
        req(
            &app,
            "POST",
            &format!("/api/cameras/{cid}/masks"),
            Some(json!({
                "name": "屏幕", "kind": "motion", "polygon": [[0,0],[150,0],[150,100],[0,100]]
            })),
        )
        .await,
    )
    .await;
    let mask_id = mask["id"].as_i64().unwrap();
    let rows =
        body_json(req(&app, "GET", &format!("/api/cameras/{cid}/spatial"), None).await).await;
    assert_eq!(rows["masks"][0]["kind"], "motion");
    assert_eq!(rows["spots"][0]["bbox"], json!(bounds));
    let policy = crate::spatial::SpatialPolicy::read(&store, cid).unwrap();
    assert!(policy.suppress(&store, cid, "no_mask", bounds, "2026-11-24T12:00:00"));
    assert!(!policy.suppress(&store, cid, "person_fall", bounds, "2026-11-24T12:00:00"));
    let audit = body_json(
        req(
            &app,
            "GET",
            &format!("/api/review/suppressions?camera_id={cid}"),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(audit.as_array().unwrap().len(), 1);
    assert_eq!(audit[0]["reason"], "remembered_spot");
    assert_eq!(audit[0]["source_id"], spot_id);
    assert_eq!(
        req(
            &app,
            "DELETE",
            &format!("/api/cameras/{second_camera}/masks/{mask_id}"),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        req(
            &app,
            "DELETE",
            &format!("/api/cameras/{cid}/masks/{mask_id}"),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        req(
            &app,
            "DELETE",
            &format!("/api/cameras/{cid}/fp-spots/{spot_id}"),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(!crate::spatial::SpatialPolicy::read(&store, cid)
        .unwrap()
        .suppress(&store, cid, "no_mask", bounds, "2026-11-24T12:00:00"));
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
                // 相对日期（回归修复：硬编码 2026-11-09 在 7 天窗外后 stats_quality 必炸）
                ts: &format!("{}T10:00:00", crate::util::iso_days_ago(1)),
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
    let r = req(
        &app,
        "GET",
        &format!("/api/stats/hourly?date={}", crate::util::iso_days_ago(1)),
        None,
    )
    .await;
    assert_eq!(body_json(r).await["hours"][10]["total"], 2);
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

// ==== R2-309 收尾：月报 PDF（export/monthly.pdf，105/106）====

#[tokio::test]

async fn testserver_host_gated_by_testing() {
    // R2-108 对齐（深审发现 Rust 侧 testserver 曾无条件豁免，Python 已按测试态门控）
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    // 生产态（testing=false）：testserver → 400
    let st = test_state_custom(
        db.to_str().unwrap(),
        "",
        false,
        dir.path().to_str().unwrap(),
        false,
        "",
    );
    let app = build_router(st);
    let r = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/health/live")
                .header("host", "testserver")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        StatusCode::BAD_REQUEST,
        "生产态 testserver 必须被拒"
    );
    // 对照：白名单内主机放行
    let r2 = axum::http::Request::builder()
        .uri("/api/health/live")
        .header("host", "127.0.0.1")
        .body(axum::body::Body::empty())
        .unwrap();
    let r2 = app.oneshot(r2).await.unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    // 测试态（testing=true）：testserver 放行
    let dir2 = tempfile::tempdir().unwrap();
    let st2 = test_state(dir2.path().join("t2.db").to_str().unwrap());
    let app2 = build_router(st2);
    let r3 = app2
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/health/live")
                .header("host", "testserver")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::OK, "测试态 testserver 放行");
}
