//! RA5-UX（ADR-0011 增量）：接入引导与便捷操作的后端支撑。
//!
//! - `GET /api/fs/list?path=`：目录浏览（盘符/子项）——前端目录选择器数据源，
//!   全盘浏览为本机单用户产品的设计边界（admin 令牌门控）；
//! - `GET /api/setup/status`：Welcome 引导向导所需状态（相机/模型/通知渠道/
//!   设备配对/VLM 配置是否完成）；
//! - `PUT /api/models/active`：检测模型切换（写入 app_settings，重启相机生效）
//!   ——模型模块化的 UI 可选面（本地清单来自 /api/models/local）。

use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::middleware::ct_eq;
use crate::state::Shared;
use crate::util::{detail_err, with_store};

fn admin_guard(st: &Shared, headers: &axum::http::HeaderMap) -> Option<Response> {
    // auth-off 放行；否则要求管理令牌（目录浏览/模型切换均为管理面）
    if st.admin_token.is_empty() {
        return None;
    }
    let provided = headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok().and_then(|s| s.strip_prefix("Bearer ")))
        })
        .unwrap_or("");
    if ct_eq(provided, &st.admin_token) {
        None
    } else {
        Some(detail_err(axum::http::StatusCode::UNAUTHORIZED, "需要管理令牌").into_response())
    }
}

/// GET /api/fs/list?path=<目录>——目录浏览（前端目录选择器数据源）。
/// path 为空时返回盘符列表。条目：{name, path, is_dir}。不可读目录跳过。
pub(crate) async fn fs_list(
    State(st): State<Shared>,
    axum::extract::RawQuery(q): axum::extract::RawQuery,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Some(resp) = admin_guard(&st, &headers) {
        return resp;
    }
    let q = q.unwrap_or_default();
    let path = q
        .split('&')
        .find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == "path"))
        .map(|(_, v)| urldecode(v))
        .unwrap_or_default();
    if path.is_empty() {
        // 盘符列举（Windows：A:\\..Z\\ 探测）
        let mut drives = Vec::new();
        for b in b'A'..=b'Z' {
            let letter = b as char;
            let drive_path = format!("{letter}:\\");
            let p = std::path::Path::new(&drive_path);
            if p.is_dir() {
                drives.push(json!({"name": format!("{letter}:"), "path": drive_path}));
            }
        }
        return Json(json!({"drives": drives})).into_response();
    }
    let dir = std::path::PathBuf::from(&path);
    if !dir.is_dir() {
        return detail_err(axum::http::StatusCode::NOT_FOUND, "目录不存在").into_response();
    }
    let mut entries: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('$') {
                continue; // 系统卷影/隐藏盘符项
            }
            entries.push(json!({
                "name": name,
                "path": e.path().to_string_lossy(),
                "is_dir": ft.is_dir(),
            }));
        }
    }
    entries.sort_by_key(|v: &Value| !v.get("is_dir").and_then(|x| x.as_bool()).unwrap_or(false));
    let parent = dir.parent().map(|p| p.to_string_lossy().into_owned());
    Json(json!({"path": dir.to_string_lossy(), "parent": parent, "entries": entries}))
        .into_response()
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// GET /api/setup/status——Welcome 引导向导所需状态（前端判断未完成步骤）。
pub(crate) async fn setup_status(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let cams = s.cameras().unwrap_or_default();
        let models = std::path::Path::new(&st.data_dir)
            .join("models")
            .read_dir()
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().map(|x| x == "onnx").unwrap_or(false))
                    .count()
            })
            .unwrap_or(0);
        let model_active = s.setting_get("model.active").ok().flatten().is_some()
            || std::env::var("LS_MODEL_PATH")
                .map(|v| !v.is_empty())
                .unwrap_or(false);
        let alert_cfg = s
            .setting_get("alert.wecom_webhook")
            .ok()
            .flatten()
            .is_some()
            || s.setting_get("alert.dingtalk_webhook")
                .ok()
                .flatten()
                .is_some()
            || s.setting_get("alert.feishu_webhook")
                .ok()
                .flatten()
                .is_some()
            || s.setting_get("alert.serverchan_sendkey")
                .ok()
                .flatten()
                .is_some()
            || s.setting_get("alert.pushplus_token")
                .ok()
                .flatten()
                .is_some()
            || s.setting_get("alert.smtp_host").ok().flatten().is_some()
            || s.setting_get("alert.telegram_token")
                .ok()
                .flatten()
                .is_some();
        let devices = s.devices().map(|d| d.len()).unwrap_or(0);
        let vlm = std::env::var("LS_VLM_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Ok(Json(json!({
            "steps": {
                "camera_added": !cams.is_empty(),
                "model_selected": model_active || models > 0,
                "notify_configured": alert_cfg,
                "device_paired": devices > 0,
                "vlm_ready": vlm,
            },
            "cameras": cams.len(),
            "local_models": models,
        }))
        .into_response())
    })
}

/// POST /api/plugins/upload（RA5-UX）：插件 zip 上传安装（multipart 字段 file）。
/// admin 门控；解压防 zip slip + manifest 校验，失败整包拒绝。
pub(crate) async fn plugins_upload(
    State(st): State<Shared>,
    headers: axum::http::HeaderMap,
    mut mp: Multipart,
) -> Response {
    if let Some(resp) = admin_guard(&st, &headers) {
        return resp;
    }
    const MAX_ZIP: usize = 50 * 1024 * 1024;
    let mut zip_bytes: Vec<u8> = Vec::new();
    while let Ok(Some(mut field)) = mp.next_field().await {
        if field.name() == Some("file") {
            loop {
                match field.chunk().await {
                    Ok(Some(chunk)) => {
                        if zip_bytes.len() + chunk.len() > MAX_ZIP {
                            return detail_err(
                                StatusCode::PAYLOAD_TOO_LARGE,
                                "插件包超过 50MB 上限",
                            )
                            .into_response();
                        }
                        zip_bytes.extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(_) => {
                        return detail_err(StatusCode::BAD_REQUEST, "上传读取失败").into_response()
                    }
                }
            }
        }
    }
    if zip_bytes.is_empty() {
        return detail_err(StatusCode::BAD_REQUEST, "上传内容为空").into_response();
    }
    match st.plugins.install_zip(&st, &zip_bytes) {
        Ok(id) => Json(json!({"ok": true, "id": id,
                "note": "安装成功；插件默认停用，请在列表中启用"}))
        .into_response(),
        Err((code, msg)) => detail_err(
            axum::http::StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_REQUEST),
            &msg,
        )
        .into_response(),
    }
}

/// PUT /api/models/active {path}——检测模型切换（本地清单内路径；重启相机生效）。
pub(crate) async fn models_active(
    State(st): State<Shared>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Some(resp) = admin_guard(&st, &headers) {
        return resp;
    }
    let path = body
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if path.is_empty() {
        // 空 = 清除选择（回退 env/NullDetector）
        if let Ok(s) = ls_store::Store::open(&st.db_path) {
            let _ = s.setting_set("model.active", "");
        }
        return Json(json!({"ok": true, "cleared": true})).into_response();
    }
    let p = std::path::Path::new(&path);
    if !p.is_file() || p.extension().map(|x| x != "onnx").unwrap_or(true) {
        return detail_err(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "path 必须指向一个 .onnx 文件",
        )
        .into_response();
    }
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        match s.setting_set("model.active", &path) {
            Ok(_) => Json(json!({"ok": true,
                    "note": "已切换检测模型；重启摄像头后生效"}))
            .into_response(),
            Err(e) => detail_err(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("写入失败: {e}"),
            )
            .into_response(),
        }
    } else {
        detail_err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "DB 打开失败").into_response()
    }
}
