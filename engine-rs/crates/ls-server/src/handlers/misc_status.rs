// health/system/metrics/diagnostics/meta/signing/status placeholder endpoints/face/ppe/ws.
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::discovery;
use crate::state::Shared;
use crate::url;
use crate::util::*;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) async fn health_live() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

pub(crate) async fn health_ready(State(st): State<Shared>) -> Response {
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

pub(crate) async fn system(State(st): State<Shared>) -> Json<Value> {
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => return Json(json!({"status": "error"})),
    };
    let cams = s.cameras().unwrap_or_default();
    let _total = s.count_events().unwrap_or(0);
    // RA2：system 契约补齐（today_counts/unhandled_today/alerts，SQL 口径对齐
    // Python store.today_event_counts/count_events_today_handled/delivery_counts）
    let today = s.today_event_counts().unwrap_or_default();
    let handled = s.count_events_today_handled().unwrap_or(0);
    let total_today: i64 = today.iter().map(|(_, n)| *n).sum();
    let unhandled = (total_today - handled).max(0);
    let (pending, failed, ok) = s.delivery_counts().unwrap_or((0, 0, 0));
    let mut today_counts = std::collections::BTreeMap::new();
    for (lv, n) in &today {
        today_counts.insert(lv.to_string(), n);
    }
    // UX（2026-11-21）：推理设备——providers 如实反映设置偏好（dml=DX12 统一
    // 层：N/A/Intel 卡与核显；实际生效设备以 worker 日志为准，DML 失败回落 CPU）
    let (providers, gpu_pref, gpu_active) = {
        let ep = ls_store::Store::open(&st.db_path)
            .ok()
            .map(|s| crate::detect::runtime_ep_pref(&s))
            .unwrap_or_else(|| "dml".into());
        let arr = if ep == "dml" {
            json!(["DmlExecutionProvider", "CPUExecutionProvider"])
        } else {
            json!(["CPUExecutionProvider"])
        };
        // 实际生效（模型加载后实时覆盖更新；None=尚无相机启动过）
        let active = ls_infer::yolo::ep_used();
        (arr, ep, active)
    };
    // UX2：检测模式与 cascade 用量（仪表盘卡片/设置横幅共用数据源）
    let detect = {
        let dcfg = crate::detect::DetectCfg::load(&s, std::path::Path::new(&st.data_dir));
        let u = crate::detect::usage_today(&s, &dcfg);
        json!({"mode": dcfg.mode,
               "usage": {"count": u.count, "budget": u.budget,
                         "exhausted": u.exhausted()}})
    };
    // LS-050：Motion Gate 跳过帧统计（总帧/跳过帧/比例）
    let (motion_total, motion_skipped) = st.workers.motion_stats();
    Json(json!({
        "version": st.version,
        "schema_version": s.schema_version().unwrap_or(0),
        "backend": "rust",
        "perf": {"motion": {"total_frames": motion_total, "skipped_frames": motion_skipped,
                 "skip_ratio": if motion_total > 0 { motion_skipped as f64 / motion_total as f64 } else { 0.0 }}},
        "cameras": cams.iter().map(|c| json!({"id": c.id, "name": c.name,
            "status": c.status, "type": c.type_})).collect::<Vec<_>>(),
        "today_counts": today_counts,
        "unhandled_today": unhandled,
        "alerts": {"pending": pending, "failed": failed, "ok": ok},
        "components": {"engine": {"state": "idle"}, "go2rtc": {"ok": false}},
        "config_issues": {"fail": 0, "warn": 0, "source_issues": []},
        "detect": detect,
        "providers": providers,
        "gpu_pref": gpu_pref,
        "gpu_active": gpu_active,
    }))
}

/// LS-051：go2rtc 进程状态 / 一键开关。
pub(crate) async fn go2rtc_status(State(st): State<Shared>) -> Json<Value> {
    Json(st.go2rtc.status())
}

pub(crate) async fn go2rtc_configure(
    State(st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
    let Some(enabled) = body.get("enabled").and_then(|v| v.as_bool()) else {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "enabled 必须是布尔值").into_response();
    };
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        let _ = s.setting_set("go2rtc.enabled", if enabled { "1" } else { "0" });
    }
    st.go2rtc.set_enabled(enabled);
    if enabled {
        let install = crate::deploy::default_install_root();
        let _ = st.go2rtc.ensure_running(&install, crate::go2rtc::DEFAULT_API);
    }
    Json(json!({"ok": true, "status": st.go2rtc.status()})).into_response()
}

// ---- LS-053：日报/周报调度、局域网配置、开机自启 ----

pub(crate) async fn reports_schedule_get(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let get = |k: &str| s.setting_get(k).ok().flatten().unwrap_or_default();
        Ok(Json(json!({
            "daily_enabled": matches!(get("report.daily_enabled").trim(), "1"|"true"|"yes"|"on"),
            "daily_time": if get("report.daily_time").is_empty() { "18:00".to_string() } else { get("report.daily_time") },
            "weekly_enabled": matches!(get("report.weekly_enabled").trim(), "1"|"true"|"yes"|"on"),
            "weekly_day": get("report.weekly_day").parse::<u8>().ok().filter(|d| (1..=7).contains(d)).unwrap_or(1),
            "weekly_time": if get("report.weekly_time").is_empty() { "18:00".to_string() } else { get("report.weekly_time") },
            "last_daily_date": get("report.last_daily_date"),
            "last_weekly_date": get("report.last_weekly_date"),
        })).into_response())
    })
}

pub(crate) async fn reports_schedule_put(
    State(st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
    let hm_ok = |v: &str| {
        let mut it = v.split(':');
        let h = it.next().and_then(|x| x.parse::<u8>().ok());
        let m = it.next().and_then(|x| x.parse::<u8>().ok());
        it.next().is_none() && h.is_some_and(|h| h < 24) && m.is_some_and(|m| m < 60)
    };
    for key in ["daily_time", "weekly_time"] {
        if let Some(v) = body.get(key).and_then(|x| x.as_str()) {
            if !hm_ok(v) {
                return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "时间格式必须 HH:MM")
                    .into_response();
            }
        }
    }
    if let Some(d) = body.get("weekly_day").and_then(|x| x.as_u64()) {
        if !(1..=7).contains(&d) {
            return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "weekly_day 必须在 1~7")
                .into_response();
        }
    }
    with_store(&st, |s| {
        if let Some(v) = body.get("daily_enabled").and_then(|x| x.as_bool()) {
            s.setting_set("report.daily_enabled", if v { "1" } else { "0" })?;
        }
        if let Some(v) = body.get("daily_time").and_then(|x| x.as_str()) {
            s.setting_set("report.daily_time", v)?;
        }
        if let Some(v) = body.get("weekly_enabled").and_then(|x| x.as_bool()) {
            s.setting_set("report.weekly_enabled", if v { "1" } else { "0" })?;
        }
        if let Some(v) = body.get("weekly_day").and_then(|x| x.as_u64()) {
            s.setting_set("report.weekly_day", &v.to_string())?;
        }
        if let Some(v) = body.get("weekly_time").and_then(|x| x.as_str()) {
            s.setting_set("report.weekly_time", v)?;
        }
        Ok(Json(json!({"ok": true})).into_response())
    })
}

/// POST /api/reports/send-now——立即汇总今日并推送（测试日报配置）。
pub(crate) async fn reports_send_now(State(st): State<Shared>) -> Response {
    let Ok(store) = ls_store::Store::open(&st.db_path) else {
        return server_err();
    };
    let today = ls_core::time::local_today_iso();
    let Ok((total, violation, unhandled, top)) = store.report_summary(
        &format!("{today}T00:00:00"),
        &format!("{today}T23:59:59"),
    ) else {
        return server_err();
    };
    let names: std::collections::HashMap<i64, String> = store
        .cameras()
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let text = ls_jobs::report_text("日报", total, violation, unhandled, &top, &names);
    let channels = st.jobs.alert_channels_snapshot();
    let mut sent = 0;
    let mut errors = Vec::new();
    for (ch, cfg) in &channels {
        let d = ls_dispatch::deliver_with_backoff(*ch, cfg, 2, &text, None, &[0, 0, 0]);
        if d.ok {
            sent += 1;
        } else if let Some(e) = d.error {
            errors.push(format!("{}: {e}", ch.name()));
        }
    }
    Json(json!({"ok": sent > 0 || channels.is_empty(), "sent": sent,
                "channels": channels.len(), "errors": errors, "text": text}))
    .into_response()
}

fn env_path(_st: &Shared) -> std::path::PathBuf {
    crate::deploy::default_install_root().join(".env")
}

pub(crate) async fn network_get(State(_st): State<Shared>) -> Response {
    let path = env_path(&_st);
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let host = crate::envfile::get_key(&content, "LS_HOST").unwrap_or_else(|| "127.0.0.1".into());
    let port = crate::envfile::get_key(&content, "LS_PORT")
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(8000);
    let trusted = crate::envfile::get_key(&content, "LS_TRUSTED_HOSTS").unwrap_or_default();
    let token_set = crate::envfile::get_key(&content, "LS_ADMIN_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    Json(json!({
        "lan": host == "0.0.0.0",
        "host": host, "port": port,
        "trusted_hosts": trusted,
        "admin_token_set": token_set,
        "env_path": path.to_string_lossy(),
    }))
    .into_response()
}

/// PUT /api/system/network——写 .env（保留旧键/注释），lan=true 强制要求非空令牌。
pub(crate) async fn network_put(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let path = env_path(&st);
    let mut content = std::fs::read_to_string(&path).unwrap_or_default();
    let lan = body.get("lan").and_then(|v| v.as_bool());
    let port = body.get("port").and_then(|v| v.as_u64());
    if let Some(p) = port {
        if !(1..=65535).contains(&p) {
            return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "端口必须在 1~65535")
                .into_response();
        }
    }
    let mut token = body
        .get("admin_token")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if body.get("generate_token").and_then(|v| v.as_bool()).unwrap_or(false) {
        token = Some(crate::envfile::random_token());
    }
    let existing_token = crate::envfile::get_key(&content, "LS_ADMIN_TOKEN")
        .unwrap_or_default()
        .trim()
        .to_string();
    if lan == Some(true) && token.is_none() && existing_token.is_empty() {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "开启局域网访问前必须设置管理令牌（可点「生成强随机令牌」）",
        )
        .into_response();
    }
    if let Some(t) = &token {
        if t.len() < 16 || t.contains(['\n', '\r']) {
            return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "管理令牌至少 16 字符且不含换行")
                .into_response();
        }
    }
    if let Some(l) = lan {
        content = crate::envfile::set_key(&content, "LS_HOST", if l { "0.0.0.0" } else { "127.0.0.1" });
    }
    if let Some(p) = port {
        content = crate::envfile::set_key(&content, "LS_PORT", &p.to_string());
    }
    if let Some(t) = body.get("trusted_hosts").and_then(|v| v.as_str()) {
        content = crate::envfile::set_key(&content, "LS_TRUSTED_HOSTS", t.trim());
    }
    if let Some(t) = &token {
        content = crate::envfile::set_key(&content, "LS_ADMIN_TOKEN", t);
    }
    if let Err(e) = crate::envfile::write_env(&path, &content) {
        return detail_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("写入 .env 失败: {e}"),
        )
        .into_response();
    }
    Json(json!({"ok": true, "token": token, "env_path": path.to_string_lossy()}))
        .into_response()
}

/// POST /api/system/restart——拉起新进程后优雅退出旧进程（.env 已落盘）。
pub(crate) async fn system_restart(State(st): State<Shared>) -> Response {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("labsentinel-server.exe"));
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--console").collect();
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(&args);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let spawned = cmd.spawn().is_ok();
    let st2 = Arc::clone(&st);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        crate::graceful_shutdown(&st2, "网页重启服务");
    });
    Json(json!({"ok": true, "spawned": spawned})).into_response()
}

/// Windows 开机自启：HKCU\...\Run\LabSentinel（免管理员，reg.exe + CREATE_NO_WINDOW）。
const AUTOSTART_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_NAME: &str = "LabSentinel";

fn reg_run(args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("reg");
    cmd.args(args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("reg.exe 启动失败: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub(crate) async fn autostart_get() -> Response {
    #[cfg(windows)]
    {
        let key = format!("{AUTOSTART_KEY}\\{AUTOSTART_NAME}");
        let out = reg_run(&["query", &key]).unwrap_or_default();
        let enabled = out.to_ascii_lowercase().contains("labsentinel");
        return Json(json!({"supported": true, "enabled": enabled})).into_response();
    }
    #[cfg(not(windows))]
    {
        Json(json!({"supported": false, "enabled": false})).into_response()
    }
}

pub(crate) async fn autostart_put(Json(body): Json<Value>) -> Response {
    let Some(enabled) = body.get("enabled").and_then(|v| v.as_bool()) else {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "enabled 必须是布尔值")
            .into_response();
    };
    #[cfg(windows)]
    {
        let key = format!("{AUTOSTART_KEY}\\{AUTOSTART_NAME}");
        let res = if enabled {
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("labsentinel-server.exe"));
            let val = format!("\"{}\" --launch --tray", exe.display());
            reg_run(&["add", &key, "/v", "", "/t", "REG_SZ", "/d", &val, "/f"])
        } else {
            reg_run(&["delete", &key, "/f"])
        };
        return match res {
            Ok(_) => Json(json!({"supported": true, "enabled": enabled})).into_response(),
            Err(e) => detail_err(StatusCode::INTERNAL_SERVER_ERROR, &e).into_response(),
        };
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Json(json!({"supported": false, "enabled": false})).into_response()
    }
}

// ---- LS-054：只读值守分享链接 + 一键脱敏诊断包 ----

pub(crate) async fn share_links_list(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let now = ls_core::time::local_now_iso();
        let rows = s.share_links()?;
        Ok(Json(json!({
            "links": rows.iter().map(|l| json!({
                "id": l.id, "name": l.name, "ttl_hours": l.ttl_hours,
                "created_ts": l.created_ts, "expires_ts": l.expires_ts,
                "revoked": l.revoked, "expired": l.expires_ts <= now,
            })).collect::<Vec<_>>(),
        })).into_response())
    })
}

pub(crate) async fn share_link_create(
    State(st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("值守分享")
        .trim()
        .to_string();
    let ttl = body.get("ttl_hours").and_then(|v| v.as_i64()).unwrap_or(24);
    if !matches!(ttl, 24 | 168 | 720) {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "ttl_hours 仅支持 24/168/720")
            .into_response();
    }
    let token = crate::envfile::random_token();
    let hash = sha256_hex(token.as_bytes());
    let created = ls_core::time::local_now_iso();
    let expires = ls_core::time::format_iso(ls_core::time::local_offset_secs(ttl * 3600));
    let out = with_store(&st, |s| {
        let id = s.add_share_link(&name, &hash, ttl, &created, &expires)?;
        // 复用 devices 读令牌体系：同一 hash 登记为只读设备，读保护下即可访问
        let _ = s.add_device(&format!("分享:{name}"), "share", &hash);
        Ok(Json(json!({
            "ok": true, "id": id, "token": token,
            "expires_ts": expires,
            "url": format!("/wall?readonly_token={token}"),
        }))
        .into_response())
    });
    out
}

pub(crate) async fn share_link_delete(
    State(st): State<Shared>,
    AxPath(id): AxPath<i64>,
) -> Response {
    with_store(&st, |s| {
        // 先取 hash 以便同步吊销设备令牌
        let hash: Option<String> = s
            .conn()
            .query_row("SELECT token_hash FROM share_links WHERE id=?", [id], |r| r.get(0))
            .ok();
        let n = s.revoke_share_link(id)?;
        if let Some(h) = hash {
            if let Ok(Some(dev)) = s.device_by_hash(&h) {
                let _ = s.revoke_device(dev.id);
            }
        }
        if n > 0 {
            Ok(Json(json!({"ok": true})).into_response())
        } else {
            Ok(detail_err(StatusCode::NOT_FOUND, "link not found").into_response())
        }
    })
}

/// GET /api/share-links/verify?token=…——前端只读链接进入前校验有效性。
pub(crate) async fn share_link_verify(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let token = q.get("token").map(|s| s.trim()).unwrap_or("");
    if token.is_empty() {
        return Json(json!({"valid": false})).into_response();
    }
    let hash = sha256_hex(token.as_bytes());
    let valid = ls_store::Store::open(&st.db_path)
        .ok()
        .and_then(|s| s.share_link_valid(&hash, &ls_core::time::local_now_iso()).ok())
        .unwrap_or(false);
    Json(json!({"valid": valid})).into_response()
}

/// GET /api/diagnostics/bundle.zip——脱敏诊断包（版本/系统/GPU/工具探测/配置摘要/近2天日志）。
pub(crate) async fn diagnostics_bundle(State(st): State<Shared>) -> Response {
    let mut zip = crate::handlers::events::ZipWriter::new();
    zip.add_file("version.txt", format!("LabSentinel {}\n", st.version).as_bytes());
    let (providers, gpu_active) = (
        json!(["DmlExecutionProvider", "CPUExecutionProvider"]),
        ls_infer::yolo::ep_used(),
    );
    let ffmpeg = ls_core::bin::find("ffmpeg");
    let go2rtc = crate::go2rtc::Go2rtcSupervisor::resolve_bin(&crate::deploy::default_install_root());
    let system = json!({
        "version": st.version,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "data_dir_set": !st.data_dir.is_empty(),
        "providers": providers,
        "gpu_active": gpu_active,
        "ffmpeg": {"path_found": ffmpeg.is_file(), "file": ffmpeg.file_name().map(|s| s.to_string_lossy().to_string())},
        "go2rtc": {"found": go2rtc.is_some()},
        "go2rtc_status": st.go2rtc.status(),
    });
    zip.add_file(
        "system.json",
        serde_json::to_string_pretty(&system).unwrap_or_default().as_bytes(),
    );
    // 配置摘要：敏感键一律 ***
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        let settings = s.settings_by_prefix("").unwrap_or_default();
        let mut redacted = serde_json::Map::new();
        for (k, v) in settings {
            let lower = k.to_ascii_lowercase();
            let sensitive = lower.contains("key")
                || lower.contains("token")
                || lower.contains("secret")
                || lower.contains("password")
                || lower.contains("passwd")
                || lower.contains("webhook")
                || lower.contains("smtp")
                || lower.contains("api");
            redacted.insert(
                k,
                json!(if sensitive { "***".to_string() } else { v }),
            );
        }
        zip.add_file(
            "config.redacted.json",
            serde_json::to_string_pretty(&Value::Object(redacted))
                .unwrap_or_default()
                .as_bytes(),
        );
    }
    // 近 2 天日志（按文件名日期倒序取 2 个）
    let logs_dir = std::path::Path::new(&st.data_dir).join("logs");
    let mut logs: Vec<std::path::PathBuf> = std::fs::read_dir(&logs_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("server-") && n.ends_with(".log"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    logs.sort();
    for p in logs.iter().rev().take(2) {
        if let (Ok(bytes), Some(name)) = (std::fs::read(p), p.file_name().and_then(|n| n.to_str())) {
            zip.add_file(&format!("logs/{name}"), &bytes);
        }
    }
    let bytes = zip.finish();
    (
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"labsentinel-diagnostics.zip\"",
            ),
        ],
        bytes,
    )
        .into_response()
}

pub(crate) async fn metrics(State(st): State<Shared>) -> Response {
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

pub(crate) async fn diagnostics_summary(State(st): State<Shared>) -> Response {
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
            "disk": {"data_dir": "(data)", "free_bytes": null, "total_bytes": null},
            "logs": {"file": null, "size_bytes": 0, "rotated": []},
            "recent_errors": [],
        })))
    })
}

pub(crate) async fn api_v1_meta() -> Json<Value> {
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

/// T-108 对齐：HMAC(path|exp) 短期签名；密钥 = admin_token（未设时进程内随机）。
pub(crate) fn media_secret(st: &Shared) -> String {
    use std::sync::OnceLock;
    static FALLBACK: OnceLock<String> = OnceLock::new();
    if !st.admin_token.is_empty() {
        st.admin_token.clone()
    } else {
        FALLBACK.get_or_init(random_token).clone()
    }
}

pub(crate) async fn sign_media(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let path = q.get("path").cloned().unwrap_or_default();
    if !path.starts_with("/api/") || path.contains('?') || path.contains('#') || path.contains(' ')
    {
        return detail_err(
            StatusCode::BAD_REQUEST,
            "path 只能是 /api/ 媒体路径（查询串自行拼接在签名后）",
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

fn env_num<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// RA5（模型模块化）：本地可用模型清单——models/ 目录 + LS_MODEL_PATH/
/// LS_PPE_MODEL 指向项，供网页端"检测后端"选择（用户自备第三方 ONNX 亦入册）。
pub(crate) async fn models_local(State(st): State<Shared>) -> Response {
    let mut items: Vec<Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();
    // 扫描目录：data_dir/models/（按需下载/自备模型落位）+ env 指向项
    let models_dir = std::path::Path::new(&st.data_dir).join("models");
    if let Ok(rd) = std::fs::read_dir(&models_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "onnx").unwrap_or(false) {
                let name = p
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if seen.insert(name.clone()) {
                    items.push(json!({
                        "name": name,
                        "path": p.to_string_lossy(),
                        "kind": "onnx",
                        "size_bytes": std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
                    }));
                }
            }
        }
    }
    for env_key in ["LS_MODEL_PATH", "LS_PPE_MODEL", "LS_MODEL_FP32"] {
        if let Ok(p) = std::env::var(env_key) {
            let pp = std::path::Path::new(p.trim()).to_path_buf();
            if pp.is_file() {
                let name = pp
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if seen.insert(name.clone()) {
                    items.push(json!({
                        "name": name,
                        "path": pp.to_string_lossy(),
                        "kind": env_key,
                        "size_bytes": std::fs::metadata(&pp).map(|m| m.len()).unwrap_or(0),
                    }));
                }
            }
        }
    }
    Json(json!({"models": items})).into_response()
}

pub(crate) async fn model_status() -> Response {
    // RA2-S9：形状对齐 Python api.model_status（只读上报，不加载 ONNX）。
    let fp32 = std::env::var("LS_MODEL_FP32").unwrap_or_default();
    let ppe = std::env::var("LS_PPE_MODEL").unwrap_or_default();
    let active = if ppe.is_empty() { &fp32 } else { &ppe };
    let backend_order = std::env::var("LS_BACKEND_ORDER").unwrap_or_default();
    let backend = backend_order
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    // classes 文件：模型同目录 <stem>.classes.txt，回退 yolo11n_ppe.classes.txt
    let classes_file: Option<String> = if active.is_empty() {
        None
    } else {
        let p = std::path::Path::new(active);
        let cand1 = p.with_extension("classes.txt");
        let cand2 = p
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("yolo11n_ppe.classes.txt");
        if cand1.is_file() {
            Some(cand1.to_string_lossy().into_owned())
        } else if cand2.is_file() {
            Some(cand2.to_string_lossy().into_owned())
        } else {
            None
        }
    };
    let (class_names, class_count) = match &classes_file {
        Some(f) => {
            let raw = std::fs::read_to_string(f).unwrap_or_default();
            let all: Vec<String> = raw
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            (all.iter().take(50).cloned().collect::<Vec<_>>(), all.len())
        }
        None => (Vec::new(), 0),
    };
    let ppe_class_names: Vec<String> = std::env::var("LS_PPE_CLASS_NAMES")
        .unwrap_or_else(|_| "lab coat".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Json(json!({
        "backend": backend,
        "imgsz": env_num("LS_IMGSZ", 640u32),
        "conf": env_num("LS_CONF_THRES", 0.40f64),
        "analysis_fps": env_num("LS_ANALYSIS_FPS", 5.0f64),
        "models": {"ppe": file_info(std::path::Path::new(active)),
                   "fp32": file_info(std::path::Path::new(&fp32))},
        "classes": {"file": classes_file, "exists": classes_file.is_some(),
                    "count": class_count, "names": class_names},
        "ppe": {"class_names": ppe_class_names,
                "violation_class": std::env::var("LS_PPE_VIOLATION_CLASS")
                    .unwrap_or_else(|_| "no lab coat".into()),
                "conf": env_num("LS_PPE_CONF", 0.30f64),
                "wearing_conf": env_num("LS_PPE_WEARING_CONF", 0.20f64)},
        "face_enabled": std::env::var("LS_FACE_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
        "backend_order": backend_order,
        "engine": {"loaded": false, "backend": null,
                   "note": "rust 控制面独立模式不加载 ONNX（worker 运行时按 LS_MODEL_PATH 加载）"},
    }))
    .into_response()
}

pub(crate) async fn mqtt_status(State(st): State<Shared>) -> Response {
    // RA2-S2：MQTT 事件桥接线（默认关零行为变化）。字段对齐 Python
    // mqtt_bridge.status()：queue_depth/error 为 Rust stats 未覆盖项，如实 0/null。
    let enabled = std::env::var("LS_MQTT_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let host = std::env::var("LS_MQTT_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let topic_prefix =
        std::env::var("LS_MQTT_TOPIC_PREFIX").unwrap_or_else(|_| "labsentinel".into());
    let (published, dropped, failed, connected) = match &st.mqtt {
        Some(bus) => {
            let (p, d, f, c) = bus.stats.snapshot();
            (p as i64, d as i64, f as i64, c)
        }
        None => (0, 0, 0, false),
    };
    Json(json!({
        "enabled": enabled,
        "available": st.mqtt.is_some(),
        "connected": connected,
        "published": published, "dropped": dropped, "failed": failed,
        "queue_depth": 0,
        "host": host, "topic_prefix": topic_prefix,
        "error": null,
    }))
    .into_response()
}

pub(crate) async fn genai_status(State(st): State<Shared>) -> Response {
    // RA2-S5：真实九字段（默认 provider=none 时其余计数 0）
    Json(st.genai.status()).into_response()
}

pub(crate) async fn plugins_list(State(st): State<Shared>) -> Response {
    // RA2-S7：真实扫描 data/plugins（坏清单生成 error 项不崩）
    Json(json!({"plugins": st.plugins.scan(&st)})).into_response()
}

/// 版本比较（只升不降，T-110 / LS-044 修复字典序：`"0.3.9" > "0.4.0"` 必须为 false）。
fn version_gt(a: &str, b: &str) -> bool {
    let pa: Vec<u64> = a
        .split('.')
        .map(|s| s.trim().parse().unwrap_or(0))
        .collect();
    let pb: Vec<u64> = b
        .split('.')
        .map(|s| s.trim().parse().unwrap_or(0))
        .collect();
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let va = pa.get(i).copied().unwrap_or(0);
        let vb = pb.get(i).copied().unwrap_or(0);
        if va != vb {
            return va > vb;
        }
    }
    false
}

fn url_allowed(url: &str, hosts_csv: &str) -> bool {
    // T-110：https 前置 + 主机在白名单（逗号分隔，空=拒绝全部）
    if !url.starts_with("https://") {
        return false;
    }
    let host = url
        .strip_prefix("https://")
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    hosts_csv
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .any(|h| !h.is_empty() && h == host)
}

static UPDATE_CACHE: std::sync::Mutex<Option<(std::time::Instant, Value)>> =
    std::sync::Mutex::new(None);

pub(crate) async fn update_check_status(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    // RA2-S8：清单拉取真实现（1h 缓存 + https 强制 + url 白名单）。
    // minisign 签名验证未实现（分水岭决策：不引新 crate）——error 字段如实标注。
    let manifest = std::env::var("LS_UPDATE_MANIFEST_URL").unwrap_or_default();
    // 2026-11-21 修复：此前 st_version() 兜底写死 "0.2.0"，网页「版本」卡恒显示
    // 0.2.0（用户实测反馈）。改用启动时解析的 AppState.version（VERSION 文件）。
    let current = st.version.clone();
    let mut result = json!({
        "enabled": !manifest.is_empty(), "current": current,
        "update_available": false, "latest": null, "url": null,
        "sha256": null, "notes": null, "published_at": null,
        "channel": null, "error": "",
    });
    if manifest.is_empty() {
        return Json(result).into_response();
    }
    let refresh = q.get("refresh").map(|v| v == "1").unwrap_or(false);
    if !refresh {
        if let Ok(g) = UPDATE_CACHE.lock() {
            if let Some((ts, cached)) = g.as_ref() {
                if ts.elapsed() < std::time::Duration::from_secs(3600) {
                    return Json(cached.clone()).into_response();
                }
            }
        }
    }
    let allow_http = std::env::var("LS_UPDATE_ALLOW_HTTP")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if manifest.starts_with("http://") && !allow_http {
        result["error"] = json!("清单 URL 必须 https（LS_UPDATE_ALLOW_HTTP=1 可豁免内网测试）");
        return Json(result).into_response();
    }
    let fetched: Result<Value, String> = (|| {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(5)))
            .build()
            .into();
        let mut resp = agent
            .get(&manifest)
            .call()
            .map_err(|e| format!("清单拉取失败: {e}"))?;
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("清单读取失败: {e}"))?;
        serde_json::from_str(&text).map_err(|_| "清单非 JSON".to_string())
    })();
    match fetched {
        Err(e) => {
            // 错误不缓存：下次调用可重试（网络瞬时失败不粘住 1h）
            result["error"] = json!(e);
        }
        Ok(data) => {
            result["latest"] = data.get("version").cloned().unwrap_or(Value::Null);
            if let Some(l) = data.get("version").and_then(|v| v.as_str()) {
                result["update_available"] = json!(version_gt(l, &current));
            }
            for k in ["url", "sha256", "notes", "published_at", "channel"] {
                result[k] = match data.get(k) {
                    Some(Value::String(s)) => json!(s.clone()),
                    _ => Value::Null,
                };
            }
            let hosts = std::env::var("LS_UPDATE_URL_HOSTS").unwrap_or_default();
            if let Some(u) = result["url"].as_str().map(|s| s.to_string()) {
                if !url_allowed(&u, &hosts) {
                    result["url"] = Value::Null; // 前端只渲染白名单内 https 链接
                }
            }
            if let Ok(mut g) = UPDATE_CACHE.lock() {
                *g = Some((std::time::Instant::now(), result.clone()));
            }
        }
    }
    Json(result).into_response()
}

pub(crate) async fn logs_tail(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let limit: usize = q
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
        .clamp(1, 2000);
    // rust 控制面日志：UX3 起落盘 data/logs/server-YYYYMMDD.log（按天滚动），
    // 回退旧 server.log（Python 时代历史文件）
    let log_dir = std::path::Path::new(&st.data_dir).join("logs");
    let today = ls_core::time::local_today_compact();
    let candidates = [
        log_dir.join(format!("server-{today}.log")),
        log_dir.join("server.log"),
    ];
    let picked = candidates
        .iter()
        .find(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false));
    let log_name = picked
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "logs/server.log".into());
    // P2-3（安全审计）：尾部读取（2MB 上限防内存放大）+ 脱敏（对齐 Python redact：
    // TOKEN/KEY/SECRET/PASSWORD/WEBHOOK 类值打码），file 字段不再回显绝对路径
    let lines: Vec<String> = match picked {
        Some(p) => {
            let content = std::fs::read(p).unwrap_or_default();
            let start = (content.len() as u64).saturating_sub(2 * 1024 * 1024) as usize;
            let text = String::from_utf8_lossy(&content[start..]);
            text.lines()
                .rev()
                .take(limit)
                .map(redact_line)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        }
        None => Vec::new(),
    };
    Json(json!({"lines": lines, "file": format!("logs/{log_name}"),
                "note": if lines.is_empty() { "暂无落盘日志（GUI 启动后写 data/logs/server-*.log）" } else { "" }}))
        .into_response()
}

/// 行内脱敏（对齐 Python redact.redact_line 口径）：键含敏感词时值打码。
fn redact_line(line: &str) -> String {
    let sensitive = ["token", "key", "secret", "password", "webhook"];
    let lower = line.to_ascii_lowercase();
    if sensitive.iter().any(|w| lower.contains(w)) {
        // 键=值 形态：保留键，值打码
        match line.split_once('=') {
            Some((k, _)) => format!("{k}=***"),
            None => {
                if lower.contains("bearer ") {
                    format!("{} ***", &line[..lower.find("bearer").unwrap_or(0) + 6])
                } else {
                    line.to_string()
                }
            }
        }
    } else {
        line.to_string()
    }
}

pub(crate) async fn ppe_status(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        // ppe_model 优先级：env > app_settings（下载写入）> 随包 models/
        // 2026-11-21（用户指令「随包发布」）：发行包内置 yolo11n_ppe.onnx，
        // 安装即有实验服检测，不再要求网页端确认许可后下载。
        let env_ppe = std::env::var("LS_PPE_MODEL").ok().filter(|v| !v.is_empty());
        let db_ppe = s.setting_get("ppe_model").ok().flatten();
        let bundled = crate::deploy::default_install_root().join("models/yolo11n_ppe.onnx");
        let bundled = bundled
            .is_file()
            .then(|| bundled.to_string_lossy().into_owned());
        let model_path = env_ppe
            .filter(|p| std::path::Path::new(p).is_file())
            .or_else(|| db_ppe.filter(|p| std::path::Path::new(p).is_file()))
            .or_else(|| bundled.clone());
        let installed = model_path.is_some();
        // 0.4.0：启停开关（默认开）。关闭后 worker 不装配 PPE，仅人员检测。
        let enabled = s
            .setting_get("ppe.enabled")
            .ok()
            .flatten()
            .map(|v| !matches!(v.trim(), "0" | "false" | "off" | "no"))
            .unwrap_or(true);
        let source = if std::env::var("LS_PPE_MODEL")
            .ok()
            .filter(|v| !v.is_empty())
            .is_some_and(|p| std::path::Path::new(&p).is_file())
        {
            "env"
        } else if s
            .setting_get("ppe_model")
            .ok()
            .flatten()
            .is_some_and(|p| std::path::Path::new(&p).is_file())
        {
            "custom"
        } else if bundled.is_some() {
            "bundled"
        } else {
            "none"
        };
        // 展示实际文件的大小与 SHA256（按 path+len+mtime 缓存，避免高频轮询反复读盘阻塞）
        let (size_bytes, sha256) = model_path
            .as_ref()
            .and_then(|p| cached_file_sha256(std::path::Path::new(p)))
            .unwrap_or((-1, String::new()));
        let classes: Vec<String> = model_path
            .as_ref()
            .map(|p| ls_analyzer::ppe::load_class_names(std::path::Path::new(p)))
            .unwrap_or_default();
        let warning: Option<String> = if source == "env"
            && s.setting_get("ppe_model")
                .ok()
                .flatten()
                .is_some_and(|p| !p.trim().is_empty())
        {
            Some("当前由环境变量 LS_PPE_MODEL 覆盖，网页上传的自定义模型暂未生效".into())
        } else if installed && enabled && classes.is_empty() {
            Some("模型缺少配套 .classes.txt 类名表，已自动退化为仅人员检测".into())
        } else {
            None
        };
        let violation_conf = s
            .setting_get("ppe.violation_conf")
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.30);
        let wearing_conf = s
            .setting_get("ppe.wearing_conf")
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.20);
        Ok(Json(json!({
            "installed": installed,
            "enabled": enabled,
            "violation_conf": violation_conf,
            "wearing_conf": wearing_conf,
            "source": source,
            "bundled": bundled.is_some(),
            "mode": if installed && enabled && !classes.is_empty() { "ppe" } else { "person_only" },
            "active": installed && enabled && !classes.is_empty(),
            "classes": classes,
            "warning": warning,
            "license_confirmed_required": !installed,
            "license_notice": if bundled.is_some() {
                "实验服模型已随安装包提供（权重由 Roboflow 训练数据集派生，数据集许可 CC BY 4.0，署名见 NOTICE）。已默认启用，可随时关闭或替换。"
            } else {
                "实验服模型权重由 Roboflow 训练数据集派生，具体许可仍待确认（Q-08）。下载即表示你已阅读并接受该数据集的用途限制。"
            },
            "sha256": sha256,
            "size_bytes": size_bytes,
            "model_path": model_path.as_ref().map(|p| {
                // P2-6：只回显文件名（不泄露目录布局）
                std::path::Path::new(p).file_name().map(|f| f.to_string_lossy().into_owned())
            }),
            "note": if !installed {
                "安装包未内置实验服模型；可上传自定义 ONNX 或更新到随包版本"
            } else if enabled {
                "实验服模型已启用"
            } else {
                "实验服检测已关闭（仅人员检测）"
            },
        }))
        .into_response())
    })
}

/// PUT /api/models/ppe/config——启停实验服检测及可调置信度阈值 {enabled?, violation_conf?, wearing_conf?}；运行中相机软重启。
pub(crate) async fn ppe_config(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let enabled = body.get("enabled").and_then(|v| v.as_bool());
    let v_conf = body.get("violation_conf").and_then(|v| v.as_f64());
    let w_conf = body.get("wearing_conf").and_then(|v| v.as_f64());
    if enabled.is_none() && v_conf.is_none() && w_conf.is_none() {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "至少提供 enabled / violation_conf / wearing_conf 之一",
        )
        .into_response();
    }
    if let Some(v) = v_conf {
        if !(0.05..=0.95).contains(&v) {
            return detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "violation_conf 必须在 0.05~0.95 之间",
            )
            .into_response();
        }
    }
    if let Some(v) = w_conf {
        if !(0.05..=0.95).contains(&v) {
            return detail_err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "wearing_conf 必须在 0.05~0.95 之间",
            )
            .into_response();
        }
    }
    with_store(&st, |s| {
        if let Some(en) = enabled {
            s.setting_set("ppe.enabled", if en { "1" } else { "0" })?;
        }
        if let Some(v) = v_conf {
            s.setting_set("ppe.violation_conf", &format!("{v:.2}"))?;
        }
        if let Some(v) = w_conf {
            s.setting_set("ppe.wearing_conf", &format!("{v:.2}"))?;
        }
        let restarted = crate::worker::restart_all_running(&st, s);
        let cur_en = s
            .setting_get("ppe.enabled")
            .ok()
            .flatten()
            .map(|v| !matches!(v.trim(), "0" | "false" | "off" | "no"))
            .unwrap_or(true);
        Ok(Json(json!({"ok": true, "enabled": cur_en, "restarted": restarted})).into_response())
    })
}

/// 按 (path, len, mtime_ns) 缓存大文件 SHA256，避免 `/api/models/ppe/status` 轮询反复整文件读盘。
fn cached_file_sha256(path: &std::path::Path) -> Option<(i64, String)> {
    type CacheEntry = (String, u64, u128, String);
    static CACHE: std::sync::Mutex<Option<CacheEntry>> = std::sync::Mutex::new(None);
    let md = std::fs::metadata(path).ok()?;
    let len = md.len();
    let mtime_ns = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = path.to_string_lossy().into_owned();
    if let Ok(g) = CACHE.lock() {
        if let Some((k, l, m, h)) = g.as_ref() {
            if k == &key && *l == len && *m == mtime_ns {
                return Some((len as i64, h.clone()));
            }
        }
    }
    let bytes = std::fs::read(path).ok()?;
    let hash = sha256_hex(&bytes);
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((key, len, mtime_ns, hash.clone()));
    }
    Some((len as i64, hash))
}

/// POST /api/models/ppe/upload——上传自定义实验服 ONNX（multipart 字段 file，可选 classes）。
/// 落 `<data_dir>/models/ppe-custom.onnx`（并同步落盘 `ppe-custom.classes.txt`），写入 ppe_model 并启用。
pub(crate) async fn ppe_upload(
    State(st): State<Shared>,
    mut mp: axum::extract::Multipart,
) -> Response {
    const MAX: usize = 200 * 1024 * 1024;
    let mut data: Option<Vec<u8>> = None;
    let mut classes_raw: Option<String> = None;
    let mut filename = String::new();
    loop {
        match mp.next_field().await {
            Ok(Some(mut field)) => {
                let name = field.name().unwrap_or("").to_string();
                if name == "classes" {
                    if let Ok(b) = field.bytes().await {
                        if b.len() <= 256 * 1024 {
                            if let Ok(s) = String::from_utf8(b.to_vec()) {
                                if !s.trim().is_empty() {
                                    classes_raw = Some(s);
                                }
                            }
                        }
                    }
                    continue;
                }
                if name != "file" {
                    continue;
                }
                filename = field.file_name().unwrap_or("").to_string();
                let mut buf = Vec::new();
                loop {
                    match field.chunk().await {
                        Ok(Some(chunk)) => {
                            if buf.len().saturating_add(chunk.len()) > MAX {
                                return detail_err(
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                    "模型文件超过 200MB",
                                )
                                .into_response();
                            }
                            buf.extend_from_slice(&chunk);
                        }
                        Ok(None) => break,
                        Err(e) => {
                            return detail_err(
                                StatusCode::BAD_REQUEST,
                                &format!("读取上传内容失败: {e}"),
                            )
                            .into_response();
                        }
                    }
                }
                data = Some(buf);
            }
            Ok(None) => break,
            Err(e) => {
                return detail_err(StatusCode::BAD_REQUEST, &format!("解析上传失败: {e}"))
                    .into_response()
            }
        }
    }
    let Some(bytes) = data.filter(|d| !d.is_empty()) else {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "缺少 file 字段或文件为空")
            .into_response();
    };
    if !filename.is_empty() && !filename.to_ascii_lowercase().ends_with(".onnx") {
        // 不强制拦截（浏览器/导出器命名差异），仅记录便于排障
        crate::logger::warn(&format!("[ppe] 上传文件扩展名非 .onnx: {filename}"));
    }
    let dir = std::path::Path::new(&st.data_dir).join("models");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return detail_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("创建模型目录失败: {e}"),
        )
        .into_response();
    }
    let dest = dir.join("ppe-custom.onnx");
    let tmp = dir.join("ppe-custom.onnx.tmp");
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        return detail_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("写入模型失败: {e}"),
        )
        .into_response();
    }
    if std::fs::rename(&tmp, &dest).is_err() {
        // Windows 下目标被占用时先删再改名
        let _ = std::fs::remove_file(&dest);
        if let Err(e) = std::fs::rename(&tmp, &dest) {
            let _ = std::fs::remove_file(&tmp);
            return detail_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("保存模型失败: {e}"),
            )
            .into_response();
        }
    }
    // LS-041：同步落盘 ppe-custom.classes.txt（优先用上传的 classes 字段；否则继承随包类表，
    // 防止 make_ppe_runner 因找不到类表而静默退化为仅人员检测）
    let classes_dest = dir.join("ppe-custom.classes.txt");
    if let Some(txt) = classes_raw {
        let _ = std::fs::write(&classes_dest, txt);
    } else if !classes_dest.is_file() {
        let bundled_cls =
            crate::deploy::default_install_root().join("models/yolo11n_ppe.classes.txt");
        if bundled_cls.is_file() {
            let _ = std::fs::copy(&bundled_cls, &classes_dest);
        } else {
            let repo_cls = std::path::Path::new("m0-spike/yolo11n_ppe.classes.txt");
            if repo_cls.is_file() {
                let _ = std::fs::copy(repo_cls, &classes_dest);
            }
        }
    }
    let loaded_classes = ls_analyzer::ppe::load_class_names(&dest);
    let env_override = std::env::var("LS_PPE_MODEL")
        .ok()
        .is_some_and(|v| !v.trim().is_empty());
    let warning: Option<&str> = if env_override {
        Some("已保存，但当前设置了 LS_PPE_MODEL 环境变量，需移除该环境变量后自定义模型才会生效")
    } else if loaded_classes.is_empty() {
        Some("模型已保存，但未找到类名文件 ppe-custom.classes.txt，请随模型补充类名表")
    } else {
        None
    };
    with_store(&st, |s| {
        s.setting_set("ppe_model", &dest.to_string_lossy())?;
        s.setting_set("ppe.enabled", "1")?;
        let restarted = crate::worker::restart_all_running(&st, s);
        Ok(Json(json!({
            "ok": true,
            "enabled": true,
            "size_bytes": bytes.len(),
            "restarted": restarted,
            "classes": loaded_classes,
            "warning": warning,
        }))
        .into_response())
    })
}

/// POST /api/models/ppe/reset——恢复随包模型（清除自定义路径）并启用。
pub(crate) async fn ppe_reset(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        s.setting_set("ppe_model", "")?;
        s.setting_set("ppe.enabled", "1")?;
        let restarted = crate::worker::restart_all_running(&st, s);
        Ok(Json(json!({"ok": true, "enabled": true, "restarted": restarted})).into_response())
    })
}

pub(crate) async fn discovery_cameras(
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

pub(crate) async fn discovery_probe(
    State(_st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
    let host = body
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if host.is_empty() {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "host 必填").into_response();
    }
    let port = body.get("port").and_then(|v| v.as_i64()).unwrap_or(554);
    if !(1..=65535).contains(&port) {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "port 必须在 1-65535").into_response();
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

pub(crate) async fn shutdown_server(State(st): State<Shared>) -> Response {
    // Python _do_shutdown 同语义：先应答再退出（延迟给响应送达留窗口）。
    // UX4：托盘退出与本 handler 共用 graceful_shutdown（停 workers → flush WAL → exit）
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        crate::graceful_shutdown(&st, "网页停止服务");
    });
    Json(json!({"ok": true, "message": "服务将在 300ms 内退出"})).into_response()
}

pub(crate) fn plugin_not_found() -> Response {
    detail_err(StatusCode::NOT_FOUND, "plugin not found").into_response()
}

pub(crate) async fn plugin_logs(
    State(st): State<Shared>,
    AxPath(pid): AxPath<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    // RA2-S7：日志 tail（默认 200 行，与 Python plugins.logs 同口径）
    if st.plugins.scan(&st).iter().all(|p| p["id"] != pid) {
        return plugin_not_found();
    }
    let tail = q
        .get("tail")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(200)
        .clamp(1, 1000);
    Json(json!({"id": pid, "logs": st.plugins.logs(&st, &pid, tail)})).into_response()
}

pub(crate) async fn plugin_action(
    State(st): State<Shared>,
    AxPath(pid): AxPath<String>,
    req: axum::http::Request<axum::body::Body>,
) -> Response {
    // RA2-S7：enable/disable/start/stop 按 URI 尾段分发（与 Python 同契约：
    // enable/disable → {id,enabled}；start/stop → {id,status,detail}）
    let action = req
        .uri()
        .path()
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    match action.as_str() {
        "enable" | "disable" => {
            let on = action == "enable";
            if st.plugins.set_enabled(&st, &pid, on) {
                Json(json!({"id": pid, "enabled": on})).into_response()
            } else {
                plugin_not_found()
            }
        }
        "start" | "stop" => {
            let (ok, detail) = if action == "start" {
                st.plugins.start(&st, &pid)
            } else {
                st.plugins.stop(&pid)
            };
            let status = if ok { "ok" } else { "error" };
            Json(json!({"id": pid, "status": status, "detail": detail})).into_response()
        }
        _ => detail_err(StatusCode::BAD_REQUEST, "未知插件动作").into_response(),
    }
}

// ---- face（合规敏感：默认关闭。分支与 Python _face_guard 同语义） ----

pub(crate) fn face_enabled() -> bool {
    std::env::var("LS_FACE_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(crate) fn face_guard() -> Result<(), Box<Response>> {
    // RA2-S6：两道合规门（403 未开启 / 503 未配 key），之后放行真实现
    if !face_enabled() {
        return Err(Box::new(
            detail_err(
                StatusCode::FORBIDDEN,
                "人脸比对功能未开启（LS_FACE_ENABLED=false）。开启方式与合规要求见 README。",
            )
            .into_response(),
        ));
    }
    if !crate::face::FaceClient::from_env().configured() {
        return Err(Box::new(
            detail_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "人脸服务未配置（缺少 LS_COMPREFACE_API_KEY）",
            )
            .into_response(),
        ));
    }
    Ok(())
}

pub(crate) async fn face_status() -> Json<Value> {
    let c = crate::face::FaceClient::from_env();
    Json(
        json!({"enabled": face_enabled(), "configured": c.configured(),
                "threshold": c.threshold}),
    )
}

pub(crate) async fn face_register(
    State(st): State<Shared>,
    mut mp: axum::extract::Multipart,
) -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    let mut subject = String::new();
    let mut consent = false;
    let mut photo: Option<Vec<u8>> = None;
    const MAX: usize = 10 * 1024 * 1024;
    while let Ok(Some(mut field)) = mp.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "subject" => subject = field.text().await.unwrap_or_default(),
            "consent" => {
                consent = matches!(
                    field
                        .text()
                        .await
                        .unwrap_or_default()
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "true" | "1" | "on"
                )
            }
            "photo" => {
                // RA2 反思修正：流式累积 + 超限即断——原 field.bytes() 会把
                // 任意大小的上传先全量读进内存再检查 10MB 上限（DoS 面）
                let mut buf: Vec<u8> = Vec::new();
                loop {
                    match field.chunk().await {
                        Ok(Some(chunk)) => {
                            if buf.len() + chunk.len() > MAX {
                                return detail_err(
                                    StatusCode::PAYLOAD_TOO_LARGE,
                                    "照片超过 10MB 上限",
                                )
                                .into_response();
                            }
                            buf.extend_from_slice(&chunk);
                        }
                        Ok(None) => break,
                        Err(_) => {
                            return detail_err(StatusCode::BAD_REQUEST, "照片读取失败")
                                .into_response()
                        }
                    }
                }
                photo = Some(buf);
            }
            _ => {}
        }
    }
    if !consent {
        return detail_err(
            StatusCode::BAD_REQUEST,
            "注册人员必须 consent=true（合规留痕）",
        )
        .into_response();
    }
    let Some(data) = photo else {
        return detail_err(StatusCode::BAD_REQUEST, "照片为空").into_response();
    };
    if data.is_empty() || subject.trim().is_empty() {
        return detail_err(StatusCode::BAD_REQUEST, "照片为空或 subject 必填").into_response();
    }
    let client = crate::face::FaceClient::from_env();
    if let Err(e) = client.add_person(subject.trim(), &data) {
        return detail_err(StatusCode::BAD_GATEWAY, &e).into_response();
    }
    // 同意记录落本地库（只存 subject/ts，不落人脸图片）
    let ts = crate::genai::chrono_now();
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        let _ = s.add_face_consent(subject.trim(), &ts);
    }
    Json(json!({"ok": true, "subject": subject.trim(), "consent_ts": ts})).into_response()
}

pub(crate) async fn face_persons_list(State(st): State<Shared>) -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    let client = crate::face::FaceClient::from_env();
    let persons = match client.list_persons() {
        Ok(p) => p,
        Err(e) => return detail_err(StatusCode::BAD_GATEWAY, &e).into_response(),
    };
    // Python：CompreFace subjects ∩ 本地 face_consents（带同意时间戳）
    with_store(&st, |s| {
        let consents: std::collections::HashMap<String, String> =
            s.face_consents().unwrap_or_default().into_iter().collect();
        let rows: Vec<Value> = persons
            .iter()
            .map(|p| json!({"subject": p, "consent_ts": consents.get(p)}))
            .collect();
        Ok(Json(rows).into_response())
    })
}

pub(crate) async fn face_clear_all(State(st): State<Shared>) -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    let client = crate::face::FaceClient::from_env();
    let mut errors = Vec::new();
    match client.list_persons() {
        Ok(persons) => {
            for p in persons {
                if let Err(e) = client.delete_person(&p) {
                    errors.push(format!("{p}: {e}"));
                }
            }
        }
        Err(e) => errors.push(e),
    }
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        let _ = s.clear_face_consents();
    }
    Json(json!({"ok": errors.is_empty(), "errors": errors})).into_response()
}

pub(crate) async fn face_delete(AxPath(subject): AxPath<String>) -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    match crate::face::FaceClient::from_env().delete_person(&subject) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => detail_err(StatusCode::BAD_GATEWAY, &e).into_response(),
    }
}

// ---- 告警自检（真实发送：复用 ls-dispatch，渠道 URL 读 env） ----

pub(crate) fn asset_lock_path(st: &Shared) -> std::path::PathBuf {
    // 查找顺序：数据目录祖先 → 安装根（发行包）→ 仓库相对（开发布局）
    let install_root = crate::deploy::default_install_root();
    std::path::Path::new(&st.data_dir)
        .ancestors()
        .map(std::path::Path::to_path_buf)
        .chain(std::iter::once(install_root))
        .find_map(|p| {
            p.join("scripts/assets.lock.json")
                .is_file()
                .then(|| p.join("scripts/assets.lock.json"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("scripts/assets.lock.json"))
}

pub(crate) async fn ppe_download(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    if !body
        .get("confirm_license")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return detail_err(
            StatusCode::BAD_REQUEST,
            "必须先确认实验服模型的数据集许可说明",
        )
        .into_response();
    }
    let lock_path = asset_lock_path(&st);
    let Ok(lock_raw) = std::fs::read_to_string(&lock_path) else {
        return (
            StatusCode::NOT_FOUND,
            // P2-6（安全审计）：错误体不回显内部绝对路径
            Json(json!({"detail": "未找到 assets.lock.json：本安装包可能未内置实验服模型，请更新到随包版本（0.3.1+）"})),
        )
            .into_response();
    };
    let Ok(lock) = serde_json::from_str::<Value>(&lock_raw) else {
        return detail_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "assets.lock.json 解析失败",
        )
        .into_response();
    };
    let assets = match lock.get("assets").and_then(|v| v.as_object()) {
        Some(a) => a,
        None => {
            return detail_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "assets.lock.json 缺 assets",
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
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(60)))
                .build()
                .into();
            match agent.get(url).call() {
                Ok(resp) => {
                    // P2-8（安全审计）：流式下载 + want_size 提前截断——
                    // 原 read_to_end 无上限，白名单源被攻陷时可内存耗尽
                    let mut bytes: Vec<u8> = Vec::with_capacity(want_size.max(0) as usize);
                    use std::io::Read;
                    let mut reader = resp.into_body().into_reader();
                    let mut overflow = false;
                    let mut io_err = false;
                    loop {
                        let mut chunk = [0u8; 65536];
                        match reader.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(n) => {
                                bytes.extend_from_slice(&chunk[..n]);
                                if (bytes.len() as i64) > want_size {
                                    overflow = true;
                                    break;
                                }
                            }
                            Err(_) => {
                                io_err = true;
                                break;
                            }
                        }
                    }
                    if io_err {
                        last_err = "下载中断".into();
                        continue;
                    }
                    if overflow {
                        last_err = "响应超过期望大小（已截断）".into();
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
