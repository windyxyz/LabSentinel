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
    Json(json!({
        "version": st.version,
        "schema_version": s.schema_version().unwrap_or(0),
        "backend": "rust",
        "cameras": cams.iter().map(|c| json!({"id": c.id, "name": c.name,
            "status": c.status, "type": c.type_})).collect::<Vec<_>>(),
        "today_counts": today_counts,
        "unhandled_today": unhandled,
        "alerts": {"pending": pending, "failed": failed, "ok": ok},
        "components": {"engine": {"state": "idle"}, "go2rtc": {"ok": false}},
        "config_issues": {"fail": 0, "warn": 0, "source_issues": []},
    }))
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
            "disk": {"data_dir": st.data_dir, "free_bytes": null, "total_bytes": null},
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

pub(crate) async fn model_status() -> Response {
    // rust 控制面无 Python 引擎；文件状态与配置如实上报，引擎字段如实标注。
    let fp32 = std::env::var("LS_MODEL_FP32").unwrap_or_default();
    let ppe = std::env::var("LS_PPE_MODEL").unwrap_or_default();
    let active = if ppe.is_empty() { &fp32 } else { &ppe };
    Json(json!({
        "engine": {"loaded": false, "backend": null,
                   "note": "rust 控制面独立模式不加载 ONNX（数据面在 Python 服务/sidecar）"},
        "models": {"ppe": file_info(std::path::Path::new(active)),
                   "fp32": file_info(std::path::Path::new(&fp32))},
        "backend_order": std::env::var("LS_BACKEND_ORDER").unwrap_or_default(),
    }))
    .into_response()
}

pub(crate) async fn mqtt_status() -> Response {
    // rust 侧未接 MQTT 桥（任务125 在 Python 侧）；未启用形状与 Python 一致
    Json(json!({"enabled": false, "connected": false,
                "note": "MQTT 事件桥在 Python 服务实现；rust 控制面独立模式不启用"}))
    .into_response()
}

pub(crate) async fn genai_status() -> Response {
    Json(json!({"provider": "none", "configured": false,
                "note": "VLM 复核在 Python 服务实现；rust 控制面独立模式不启用"}))
    .into_response()
}

pub(crate) async fn plugins_list() -> Response {
    Json(json!({"plugins": [],
                "note": "插件管理器在 Python 服务实现；rust 控制面独立模式无插件"}))
    .into_response()
}

pub(crate) async fn update_check_status() -> Response {
    // LS_UPDATE_MANIFEST_URL 未配置时 Python 返回 enabled=false（只查静态清单）
    let manifest = std::env::var("LS_UPDATE_MANIFEST_URL").unwrap_or_default();
    Json(
        json!({"enabled": !manifest.is_empty(), "current": st_version(),
        "latest": null, "update_available": false,
        "error": null,
        "note": if manifest.is_empty() {
            "未配置 LS_UPDATE_MANIFEST_URL，更新检查关闭"
        } else {
            "rust 控制面仅上报配置状态；清单拉取在 Python 服务"
        }}),
    )
    .into_response()
}

pub(crate) fn st_version() -> String {
    std::env::var("LS_VERSION").unwrap_or_else(|_| "0.2.0".into())
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
    // rust 控制面的日志走 stdout；若有 Python 侧落盘日志则读取尾部
    let log_path = std::path::Path::new(&st.data_dir).join("logs/server.log");
    let lines: Vec<String> = match std::fs::read_to_string(&log_path) {
        Ok(content) => content
            .lines()
            .rev()
            .take(limit)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        Err(_) => Vec::new(),
    };
    Json(json!({"lines": lines, "file": log_path.to_string_lossy(),
                "note": if lines.is_empty() { "rust 控制面日志走 stdout（无落盘文件）" } else { "" }}))
        .into_response()
}

pub(crate) async fn ppe_status(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        // ppe_model 优先级：env > app_settings（Q-08 按需下载写入）
        let env_ppe = std::env::var("LS_PPE_MODEL").ok().filter(|v| !v.is_empty());
        let db_ppe = s.setting_get("ppe_model").ok().flatten();
        let model_path = env_ppe.or(db_ppe);
        let installed = model_path
            .as_ref()
            .map(|p| std::path::Path::new(p).is_file())
            .unwrap_or(false);
        Ok(Json(json!({
            "installed": installed,
            "mode": if installed { "ppe" } else { "person_only" },
            "active": installed,
            "license_confirmed_required": !installed,
            "license_notice": "实验服模型权重由 Roboflow 训练数据集派生，具体许可仍待确认（Q-08）。下载即表示你已阅读并接受该数据集的用途限制。",
            "model_path": model_path,
            "note": "rust 控制面只读状态；下载端点在 Python 服务（含 SHA256 校验）",
        }))
        .into_response())
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

pub(crate) async fn shutdown_server() -> Response {
    // Python _do_shutdown 同语义：先应答再退出（延迟给响应送达留窗口）
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::process::exit(0);
    });
    Json(json!({"ok": true, "message": "服务将在 300ms 内退出"})).into_response()
}

pub(crate) fn plugin_not_found() -> Response {
    detail_err(
        StatusCode::NOT_FOUND,
        "plugin not found（rust 控制面无插件管理器，列表见 GET /api/plugins）",
    )
    .into_response()
}

pub(crate) async fn plugin_logs(AxPath(_pid): AxPath<String>) -> Response {
    plugin_not_found()
}

pub(crate) async fn plugin_action(AxPath(_pid): AxPath<String>) -> Response {
    plugin_not_found()
}

// ---- face（合规敏感：默认关闭。分支与 Python _face_guard 同语义） ----

pub(crate) fn face_enabled() -> bool {
    std::env::var("LS_FACE_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(crate) fn face_guard() -> Result<(), Box<Response>> {
    if !face_enabled() {
        return Err(Box::new(
            detail_err(
                StatusCode::FORBIDDEN,
                "人脸比对功能未开启（LS_FACE_ENABLED=false）。开启方式与合规要求见 README。",
            )
            .into_response(),
        ));
    }
    let key = std::env::var("LS_COMPREFACE_API_KEY").unwrap_or_default();
    if key.is_empty() {
        return Err(Box::new(
            detail_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "人脸服务未配置（缺少 LS_COMPREFACE_API_KEY）",
            )
            .into_response(),
        ));
    }
    // CompreFace HTTP 客户端在 Python 服务实现（注册/识别/1:N 全链）；rust 独立
    // 模式开启开关后如实返回 503 而不是假装可用（§11.3 证据约束）。
    Err(Box::new((
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"detail": "CompreFace 客户端在 Python 服务实现；rust 控制面独立模式不支持人脸注册/识别",
                    "code": "not_available_standalone"})),
    )
        .into_response()))
}

pub(crate) async fn face_status() -> Json<Value> {
    let key = std::env::var("LS_COMPREFACE_API_KEY").unwrap_or_default();
    Json(
        json!({"enabled": face_enabled(), "configured": !key.is_empty(),
                "threshold": 0.75,
                "note": "CompreFace 客户端在 Python 服务实现"}),
    )
}

pub(crate) async fn face_register(State(_st): State<Shared>, _body: axum::body::Bytes) -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    unreachable!("face_guard 在独立模式下恒 Err（见上）")
}

pub(crate) async fn face_persons_list() -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    unreachable!("face_guard 在独立模式下恒 Err")
}

pub(crate) async fn face_clear_all() -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    unreachable!("face_guard 在独立模式下恒 Err")
}

pub(crate) async fn face_delete(AxPath(_subject): AxPath<String>) -> Response {
    if let Err(resp) = face_guard() {
        return *resp;
    }
    unreachable!("face_guard 在独立模式下恒 Err")
}

// ---- 告警自检（真实发送：复用 ls-dispatch，渠道 URL 读 env） ----

pub(crate) fn asset_lock_path(st: &Shared) -> std::path::PathBuf {
    // 便携布局 <包>/scripts/assets.lock.json → 开发布局 <repo>/scripts/assets.lock.json
    std::path::Path::new(&st.data_dir)
        .ancestors()
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
            Json(json!({"detail": format!("assets.lock.json 不存在: {}", lock_path.display())})),
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
            match ureq::get(url)
                .timeout(std::time::Duration::from_secs(60))
                .call()
            {
                Ok(resp) => {
                    let mut bytes = Vec::new();
                    use std::io::Read;
                    if resp.into_reader().read_to_end(&mut bytes).is_err() {
                        last_err = "下载中断".into();
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
