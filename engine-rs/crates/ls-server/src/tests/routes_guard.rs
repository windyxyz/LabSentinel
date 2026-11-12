// A4（2026-11-12）：从 main.rs 的 #[cfg(test)] mod tests 迁出。
// 根门面 re-export（handlers/state/util/router）+ 显式 axum/serde 类型导入。
use crate::tests::helpers::*;
use crate::*;
use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;
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
