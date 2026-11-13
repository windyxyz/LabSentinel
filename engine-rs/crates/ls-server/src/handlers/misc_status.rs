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

/// 版本比较（只升不降，T-110）：点分段数字逐段比较，短段补 0。
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
    (0..n).any(|i| pa.get(i).copied().unwrap_or(0) > pb.get(i).copied().unwrap_or(0))
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
    State(_st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    // RA2-S8：清单拉取真实现（1h 缓存 + https 强制 + url 白名单）。
    // minisign 签名验证未实现（分水岭决策：不引新 crate）——error 字段如实标注。
    let manifest = std::env::var("LS_UPDATE_MANIFEST_URL").unwrap_or_default();
    let current = st_version();
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
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(5))
            .build();
        let text = agent
            .get(&manifest)
            .call()
            .map_err(|e| format!("清单拉取失败: {e}"))?
            .into_string()
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
    // P2-3（安全审计）：尾部读取（2MB 上限防内存放大）+ 脱敏（对齐 Python redact：
    // TOKEN/KEY/SECRET/PASSWORD/WEBHOOK 类值打码），file 字段不再回显绝对路径
    let lines: Vec<String> = match std::fs::metadata(&log_path) {
        Ok(m) if m.len() > 0 => {
            let content = std::fs::read(&log_path).unwrap_or_default();
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
        _ => Vec::new(),
    };
    Json(json!({"lines": lines, "file": "logs/server.log",
                "note": if lines.is_empty() { "rust 控制面日志走 stdout（无落盘文件）" } else { "" }}))
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
            "model_path": model_path.as_ref().map(|p| {
                // P2-6：只回显文件名（不泄露目录布局）
                std::path::Path::new(p).file_name().map(|f| f.to_string_lossy().into_owned())
            }),
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
            // P2-6（安全审计）：错误体不回显内部绝对路径
            Json(json!({"detail": "assets.lock.json 不存在（检查 data_dir/scripts 布局）"})),
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
                    // P2-8（安全审计）：流式下载 + want_size 提前截断——
                    // 原 read_to_end 无上限，白名单源被攻陷时可内存耗尽
                    let mut bytes: Vec<u8> = Vec::with_capacity(want_size.max(0) as usize);
                    use std::io::Read;
                    let mut reader = resp.into_reader();
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
