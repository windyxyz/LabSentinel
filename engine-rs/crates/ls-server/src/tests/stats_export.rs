// A4（2026-11-12）：从 main.rs 的 #[cfg(test)] mod tests 迁出。
// 根门面 re-export（handlers/state/util/router）+ 显式 axum/serde 类型导入。
use crate::tests::helpers::*;
use crate::*;
use axum::http::StatusCode;
use tower::ServiceExt;
#[tokio::test]
async fn export_monthly_pdf_route() {
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
    // 播种：2026-11 三条（2×L2 new、1×L3 acked）+ 越界月份一条（不得计入）
    {
        let s = ls_store::Store::open(db.to_str().unwrap()).unwrap();
        for (i, (ts, level, detail)) in [
            ("2026-11-05T10:00:00", 2i64, "a"),
            ("2026-11-05T11:00:00", 2, "b"),
            ("2026-11-20T09:30:00", 3, "c"),
            ("2026-12-01T00:00:00", 1, "out-of-month"),
        ]
        .into_iter()
        .enumerate()
        {
            s.add_event(&ls_store::NewEvent {
                camera_id: 1,
                type_: "enter",
                level,
                track_id: i as i64,
                detail,
                snapshot: None,
                ts,
            })
            .unwrap();
        }
        s.set_event_status(3, "acked", None, None).unwrap(); // L3 那条已处置
    }
    // 缺 month → 422（对齐 FastAPI 必填参数语义）
    let r = req(&app, "GET", "/api/export/monthly.pdf", None).await;
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY, "缺 month");
    // 格式/范围非法 → 400（对齐 Python「month 必须是 YYYY-MM 格式」；年收紧 4 位）
    let r = req(&app, "GET", "/api/export/monthly.pdf?month=2026-13", None).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = req(&app, "GET", "/api/export/monthly.pdf?month=26-2", None).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    // 200：application/pdf + Content-Disposition + 合法 PDF 结构（页眉/EOF）
    let r = req(&app, "GET", "/api/export/monthly.pdf?month=2026-11", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "application/pdf");
    let cd = r.headers()["content-disposition"].to_str().unwrap();
    assert!(cd.contains("monthly_2026-11.pdf"), "{cd}");
    let b = http_body_util::BodyExt::collect(r.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert!(b.starts_with(b"%PDF"), "PDF 魔数");
    assert!(b.windows(5).any(|w| w == b"%%EOF"), "EOF 标记");
    assert!(b.windows(9).any(|w| w == b"endstream"), "内容流闭合");
    // 闰年 2 月（29 天）同样可用
    let r = req(&app, "GET", "/api/export/monthly.pdf?month=2024-02", None).await;
    assert_eq!(r.status(), StatusCode::OK);
}

// ==== 2026-11-11 深审简化批：R2-108 对齐 + web_dist 接线 ====

#[tokio::test]
async fn web_dist_static_serving_and_spa_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>lab-ui</html>").unwrap();
    std::fs::write(web.join("app.js"), b"console.log(1)").unwrap();
    let db = dir.path().join("t.db");
    let st = test_state_custom(
        db.to_str().unwrap(),
        "",
        false,
        dir.path().to_str().unwrap(),
        true,
        web.to_str().unwrap(),
    );
    let app = build_router(st);
    // 首页直接命中
    let r = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/")
                .header("host", "127.0.0.1")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let b = http_body_util::BodyExt::collect(r.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(b.as_ref(), b"<html>lab-ui</html>");
    // 未命中路径 → SPA 回退 index.html（vue-router history 模式）
    let app2 = build_router(test_state_custom(
        db.to_str().unwrap(),
        "",
        false,
        dir.path().to_str().unwrap(),
        true,
        web.to_str().unwrap(),
    ));
    let r2 = app2
        .oneshot(
            axum::http::Request::builder()
                .uri("/events")
                .header("host", "127.0.0.1")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    let b2 = http_body_util::BodyExt::collect(r2.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(b2.as_ref(), b"<html>lab-ui</html>");
    // web_dist 为空 → 不挂载静态服务（/ 与未命中路径都 404）
    let dir3 = tempfile::tempdir().unwrap();
    let st3 = test_state_custom(
        dir3.path().join("t3.db").to_str().unwrap(),
        "",
        false,
        dir3.path().to_str().unwrap(),
        true,
        "",
    );
    let app3 = build_router(st3);
    let r3 = app3
        .oneshot(
            axum::http::Request::builder()
                .uri("/")
                .header("host", "127.0.0.1")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_cache_policy_headers() {
    // 2026-11-12：兑现文件头「缓存策略」——/assets/* hash 文件名 immutable，
    // index/SPA 回退 no-cache（协商校验，发版即生效）。
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("index.html"), "<html>lab-ui</html>").unwrap();
    std::fs::write(
        web.join("assets").join("index-abc123.js"),
        b"console.log(1)",
    )
    .unwrap();
    let db = dir.path().join("t.db");
    let st = test_state_custom(
        db.to_str().unwrap(),
        "",
        false,
        dir.path().to_str().unwrap(),
        true,
        web.to_str().unwrap(),
    );
    let app = build_router(st);

    // /assets/* → immutable 一年
    let r = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/assets/index-abc123.js")
                .header("host", "127.0.0.1")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        r.headers()[axum::http::header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );

    // 首页 → no-cache
    let r2 = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/")
                .header("host", "127.0.0.1")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(r2.headers()[axum::http::header::CACHE_CONTROL], "no-cache");

    // SPA 回退 → 同样 no-cache
    let r3 = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/events")
                .header("host", "127.0.0.1")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::OK);
    assert_eq!(r3.headers()[axum::http::header::CACHE_CONTROL], "no-cache");
}
