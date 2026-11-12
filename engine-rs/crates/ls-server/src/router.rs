// 2026-11-12 batch A split: route registration, ROUTE_INVENTORY, dump_routes.
// `use crate::*` glob covers root re-exports (state/handlers fns) and root `use` axum/serde types.
use crate::*;

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
    // ---- R2-309 收尾：月报 PDF（folders/{cid}/scan 需推理管线接线，保持 503 如实标注） ----
    ("GET", "/api/export/monthly.pdf"),
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
        .route("/export/monthly.pdf", get(export_monthly_pdf))
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
    // 2026-11-11：接线 web_dist（此前字段有赋值但从未被路由使用——文件头注释声称
    // 的「静态 web/dist」一直未兑现）。ls-server 独立模式由此直接出 Web UI：
    // - SPA 回退：未命中路径回 index.html（前端 vue-router history 模式）
    // - 缓存策略（2026-11-12 兑现文件头注释）：/assets/*（Vite hash 文件名）→
    //   `immutable, max-age=31536000`；index.html 与 SPA 回退 → `no-cache`
    //   （每次协商校验，保证发版即生效）
    // - 放在 guard 中间件内：Host 白名单（防 DNS Rebinding）同样覆盖静态资源；
    //   读保护只拦 /api/*，页面本身公开（令牌由前端注入）
    // web_dist 为空（测试/内嵌模式）时不挂载，保持纯 API。
    let mut app = Router::new().nest("/api", api).merge(ws);
    if !state.web_dist.is_empty() {
        let index = std::path::Path::new(&state.web_dist).join("index.html");
        // 注意用 .fallback() 而非 .not_found_service()：后者会用 SetStatus 把回退
        // 响应强制改写为 404（tower-http 0.5.2 serve_dir/mod.rs:236）——SPA 回退
        // 需要回退服务的原始 200。实测踩坑记录。
        app = app.fallback_service(
            get_service(ServeDir::new(&state.web_dist).fallback(ServeFile::new(index)))
                .layer(axum::middleware::from_fn(static_cache_policy)),
        );
    }
    app.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        guard_middleware,
    ))
    .with_state(state)
}
