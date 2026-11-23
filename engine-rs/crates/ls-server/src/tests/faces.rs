use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{Request, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::helpers::{body_json, req, test_state_cfg_dir};

#[derive(Default)]
struct Remote {
    subject: String,
    enrolls: u32,
    allow_delete: bool,
    recognitions: u32,
}

type SharedRemote = Arc<Mutex<Remote>>;

fn photo_input(path: &str, subject: &str, consent: bool) -> Request<Body> {
    let jpeg = ls_analyzer::evidence::encode_jpeg_bgr(&[125; 64 * 64 * 3], 64, 64).unwrap();
    let boundary = "test-photos";
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"subject\"\r\n\r\n{subject}\r\n\
        --{boundary}\r\nContent-Disposition: form-data; name=\"consent\"\r\n\r\n{consent}\r\n\
        --{boundary}\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"fixture.jpg\"\r\n\
        Content-Type: image/jpeg\r\n\r\n").into_bytes();
    body.extend(jpeg);
    body.extend(format!("\r\n--{boundary}--\r\n").bytes());
    Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "localhost")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn api_faces_obey_consent_selection_and_pending_removal() {
    let remote = SharedRemote::default();
    let upstream = Router::new()
        .route("/api/v1/recognition/subjects", get(|| async { Json(json!({"subjects":[]})) }))
        .route("/api/v1/recognition/subjects/:subject", delete(
            |State(remote):State<SharedRemote>| async move {
                if remote.lock().unwrap().allow_delete { StatusCode::OK } else { StatusCode::BAD_GATEWAY }
            },
        ))
        .route("/api/v1/recognition/faces", post(
            |State(remote): State<SharedRemote>, Query(q): Query<HashMap<String,String>>| async move {
                let mut remote = remote.lock().unwrap();
                remote.subject = q["subject"].clone();
                remote.enrolls += 1;
                Json(json!({"image_id":format!("fixture-{}", remote.enrolls)}))
            },
        ))
        .route("/api/v1/recognition/recognize", post(
            |State(remote):State<SharedRemote>| async move {
                let mut remote = remote.lock().unwrap();
                remote.recognitions += 1;
                Json(json!({"result":[{"subjects":[{"subject":remote.subject,"similarity":0.98}]}]}))
            },
        )).with_state(remote.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_str().unwrap();
    let db = directory.path().join("faces.db");
    let store = ls_store::Store::open(db.to_str().unwrap()).unwrap();
    store.ensure_schema().unwrap();
    let st = test_state_cfg_dir(db.to_str().unwrap(), "", false, root);
    let app = crate::build_router(st.clone());
    let config = body_json(
        req(
            &app,
            "PUT",
            "/api/face/config",
            Some(json!({
                "mode":"api", "retention_days":30, "profile":{
                    "name":"fixture", "url":format!("http://127.0.0.1:{port}"),
                    "api_key":"synthetic-test-key", "enabled":true,"timeout_s":3,"threshold":0.75
                }
            })),
        )
        .await,
    )
    .await;
    assert_eq!(config["status"]["configured"], true);
    assert!(!config.to_string().contains("synthetic-test-key"));
    let stored = store.setting_get("face.settings").unwrap().unwrap();
    assert!(stored.contains("enc:v1:"));
    assert!(!stored.contains("synthetic-test-key"));

    let invalid = app
        .clone()
        .oneshot(photo_input("/api/face/persons", "Fixture A", false))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(remote.lock().unwrap().enrolls, 0);
    let save = app
        .clone()
        .oneshot(photo_input("/api/face/persons", "Fixture A", true))
        .await
        .unwrap();
    assert_eq!(save.status(), StatusCode::OK);
    let person = body_json(save).await;
    let id = person["id"].as_i64().unwrap();
    assert_ne!(remote.lock().unwrap().subject, "Fixture A");
    assert_eq!(store.face_people().unwrap().len(), 1);
    let recognize = || {
        app.clone()
            .oneshot(photo_input("/api/face/recognize", "", true))
    };
    let matched: Value = body_json(recognize().await.unwrap()).await;
    assert_eq!(matched["matched"]["subject"], "Fixture A");

    let camera = store
        .conn_add_camera("Fixture", "test.mp4", "file")
        .unwrap();
    let mut events = st.events_tx.subscribe();
    let frame = ls_analyzer::evidence::encode_jpeg_bgr(&[125; 64 * 64 * 3], 64, 64).unwrap();
    let camera_path = format!("/api/face/cameras/{camera}");
    for permission in ["local", "local_or_api"] {
        let camera_config = body_json(
            req(
                &app,
                "PUT",
                &camera_path,
                Some(json!({"permission":permission, "alert_on_match":true})),
            )
            .await,
        )
        .await;
        assert_eq!(camera_config["alert_on_match"], true);
        crate::local_face::identify_event(
            &st,
            &store,
            crate::local_face::MatchTarget::Alert(1),
            camera,
            [0., 0., 64., 64.],
            &frame,
        );
        if permission == "local" {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), events.recv())
                    .await
                    .is_err()
            );
            assert_eq!(remote.lock().unwrap().recognitions, 1);
        }
    }
    let event: Value = serde_json::from_str(
        &tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(event["type"], "face_match");
    assert!(!event.to_string().contains("Fixture A"));
    let event_id = event["id"].as_i64().unwrap();
    assert_eq!(
        store
            .event_face_match(event_id, crate::local_face::unix_now())
            .unwrap()
            .unwrap()
            .0,
        "Fixture A"
    );
    crate::local_face::identify_event(
        &st,
        &store,
        crate::local_face::MatchTarget::Alert(2),
        camera,
        [0., 0., 64., 64.],
        &frame,
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), events.recv())
            .await
            .is_err()
    );

    let deleted = req(&app, "DELETE", &format!("/api/face/library/{id}"), None).await;
    assert_eq!(deleted.status(), StatusCode::BAD_GATEWAY);
    assert!(store.face_person_by_id(id).unwrap().unwrap().removing);
    assert!(body_json(recognize().await.unwrap()).await["matched"].is_null());
    remote.lock().unwrap().allow_delete = true;
    assert_eq!(
        req(&app, "DELETE", &format!("/api/face/library/{id}"), None)
            .await
            .status(),
        StatusCode::OK
    );
    assert!(store.face_person_by_id(id).unwrap().is_none());

    let off = req(&app, "PUT", "/api/face/config", Some(json!({"mode":"off"}))).await;
    assert_eq!(off.status(), StatusCode::OK);
    assert_eq!(recognize().await.unwrap().status(), StatusCode::FORBIDDEN);
    let restored = body_json(
        req(
            &app,
            "PUT",
            "/api/face/config",
            Some(json!({"restore_previous":true})),
        )
        .await,
    )
    .await;
    assert_eq!(restored["active"]["mode"], "api");
    server.abort();
}
