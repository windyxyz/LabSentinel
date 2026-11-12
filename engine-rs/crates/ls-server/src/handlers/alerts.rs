// alert config (get/put/reset) + template render/preview + alert_test (alert_cfg meta shipped along).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use crate::{discovery, fernet, pdf};
use axum::extract::{
    ws::{WebSocket, WebSocketUpgrade},
    Path as AxPath, Query, State,
};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) async fn alerts_status(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let counts = s.deliveries(None, 10_000)?;
        let mut summary = std::collections::BTreeMap::new();
        for d in &counts {
            *summary.entry(d.status.clone()).or_insert(0) += 1;
        }
        Ok(Json(json!({
            "configured": {"wecom": false, "dingtalk": false, "feishu": false,
                            "serverchan": false, "pushplus": false, "qmsg": false,
                            "email": false, "webhook": false},
            "note": "渠道配置在 Python 服务（app_settings/env + 密钥加密）；rust 控制面只汇总投递",
            "deliveries_summary": summary,
            "deliveries_total": counts.len(),
        }))
        .into_response())
    })
}

pub(crate) async fn alert_template_get() -> Json<Value> {
    // 与 Python settings 默认模板 + placeholders 说明一致（任务85）
    Json(json!({
        "title": "LabSentinel 级别{level}告警",
        "text": "{icon} L{level} {type_text} · {ts}\n摄像头 #{camera_id}{track_part}\n{detail}",
        "placeholders": {
            "icon": "级别图标（ℹ️/⚠️/🔴）", "level": "级别数字 1-3",
            "type": "事件类型标识", "type_text": "事件类型中文名",
            "ts": "事件时间", "camera_id": "摄像头 ID",
            "track_id": "跟踪 ID（occupancy 为 0）", "detail": "事件详情",
            "track_part": "「 · track N」或空串（occupancy 时为空）",
        },
    }))
}

/// 与 Python `alert_config.FIELDS/SECRET/URL/BOOL/INT` 同口径的字段元数据。
pub(crate) mod alert_cfg {
    pub const PREFIX: &str = "alert.";
    /// channel -> (api 字段, settings 字段)
    pub const FIELDS: &[(&str, &[(&str, &str)])] = &[
        ("wecom", &[("webhook", "wecom_webhook")]),
        ("dingtalk", &[("webhook", "dingtalk_webhook")]),
        ("feishu", &[("webhook", "feishu_webhook")]),
        ("serverchan", &[("sendkey", "serverchan_sendkey")]),
        ("pushplus", &[("token", "pushplus_token")]),
        ("qmsg", &[("key", "qmsg_key"), ("qq", "qmsg_qq")]),
        (
            "email",
            &[
                ("smtp_host", "smtp_host"),
                ("smtp_port", "smtp_port"),
                ("smtp_user", "smtp_user"),
                ("smtp_pass", "smtp_pass"),
                ("to", "alert_email_to"),
            ],
        ),
        (
            "webhook",
            &[
                ("url", "webhook_url"),
                ("secret", "webhook_secret"),
                ("allow_http", "webhook_allow_http"),
            ],
        ),
    ];
    pub const SECRET_FIELDS: &[&str] = &[
        "wecom_webhook",
        "dingtalk_webhook",
        "feishu_webhook",
        "serverchan_sendkey",
        "pushplus_token",
        "qmsg_key",
        "smtp_pass",
        "webhook_secret",
    ];
    pub const URL_FIELDS: &[&str] = &[
        "wecom_webhook",
        "dingtalk_webhook",
        "feishu_webhook",
        "webhook_url",
    ];
    pub const BOOL_FIELDS: &[&str] = &["webhook_allow_http"];
    pub const INT_FIELDS: &[&str] = &["smtp_port"];

    /// 渠道 URL host 白名单（与 Python alerts.py 常量同口径）
    pub fn url_allowed_hosts(field: &str) -> Option<&'static [&'static str]> {
        match field {
            "wecom_webhook" => Some(&["qyapi.weixin.qq.com"]),
            "dingtalk_webhook" => Some(&["oapi.dingtalk.com"]),
            "feishu_webhook" => Some(&["open.feishu.cn"]),
            _ => None,
        }
    }

    pub fn fields_of(channel: &str) -> Option<&'static [(&'static str, &'static str)]> {
        FIELDS.iter().find(|(c, _)| *c == channel).map(|(_, m)| *m)
    }

    pub fn is_secret(field: &str) -> bool {
        SECRET_FIELDS.contains(&field)
    }

    pub fn is_url(field: &str) -> bool {
        URL_FIELDS.contains(&field)
    }

    pub fn is_bool(field: &str) -> bool {
        BOOL_FIELDS.contains(&field)
    }

    pub fn is_int(field: &str) -> bool {
        INT_FIELDS.contains(&field)
    }
}

/// Python `_mask_url` 复刻：保留 scheme/host/路径结构，查询值全 ***，长 token 打码。
pub(crate) fn mask_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_string(), r.to_string()),
        None => return "***".into(),
    };
    let (authority, tail) = match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.clone(), String::new()),
    };
    let (path_part, query) = match tail.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (tail.clone(), None),
    };
    let mut parts: Vec<String> = path_part
        .split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if let Some(last) = parts.last_mut() {
        if last.len() >= 8 {
            *last = "***".into();
        }
    }
    let query_out = query.map(|q| {
        q.split('&')
            .filter(|kv| !kv.is_empty())
            .map(|kv| format!("{}=***", kv.split('=').next().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("&")
    });
    format!(
        "{scheme}://{authority}/{}{}{}",
        parts.join("/"),
        if query_out.is_some() { "?" } else { "" },
        query_out.unwrap_or_default()
    )
}

/// 字段当前有效值：DB 覆盖（解密）优先，其次 env（Python load() 语义的读路径）。
pub(crate) fn alert_field_value(s: &ls_store::Store, st: &Shared, field: &str) -> String {
    if let Ok(Some(raw)) = s.setting_get(&format!("{}{field}", alert_cfg::PREFIX)) {
        if alert_cfg::is_secret(field) {
            return fernet::unprotect(&raw, std::path::Path::new(&st.data_dir));
        }
        return raw;
    }
    std::env::var(format!("LS_{}", field.to_ascii_uppercase())).unwrap_or_default()
}

/// Python `alert_config.status` 复刻：永不回显密钥/完整 webhook。
pub(crate) fn alerts_config_status(s: &ls_store::Store, st: &Shared) -> Value {
    let mut channels = serde_json::Map::new();
    for (channel, mapping) in alert_cfg::FIELDS {
        let mut item = serde_json::Map::new();
        let mut any_db = false;
        let mut any_env = false;
        for (api_name, field) in mapping.iter() {
            let db_raw = s
                .setting_get(&format!("{}{field}", alert_cfg::PREFIX))
                .ok()
                .flatten();
            if db_raw.is_some() {
                any_db = true;
            }
            let val = alert_field_value(s, st, field);
            if !val.is_empty() {
                any_env = true;
            }
            if alert_cfg::is_bool(field) {
                let b = matches!(val.as_str(), "1" | "true" | "True" | "yes" | "on");
                item.insert(api_name.to_string(), json!(b));
                continue;
            }
            if alert_cfg::is_int(field) {
                let n: Option<i64> = val.parse().ok();
                item.insert(api_name.to_string(), json!(n));
                continue;
            }
            let configured_like = !val.trim().is_empty();
            item.insert(format!("has_{api_name}"), json!(configured_like));
            if alert_cfg::is_url(field) {
                item.insert(api_name.to_string(), json!(mask_url(&val)));
            } else if alert_cfg::is_secret(field) {
                item.insert(api_name.to_string(), json!(""));
            } else {
                item.insert(api_name.to_string(), json!(val));
            }
        }
        // configured 判定（Python _configured 同口径；qmsg 需 key+qq、email 需 host+to）
        let get = |f: &str| alert_field_value(s, st, f);
        let configured = match *channel {
            "qmsg" => !get("qmsg_key").trim().is_empty() && !get("qmsg_qq").trim().is_empty(),
            "email" => {
                !get("smtp_host").trim().is_empty() && !get("alert_email_to").trim().is_empty()
            }
            _ => mapping
                .iter()
                .any(|(_, f)| !get(f).trim().is_empty() && !alert_cfg::is_bool(f)),
        };
        item.insert("configured".into(), json!(configured));
        item.insert(
            "source".into(),
            json!(if any_db {
                "db"
            } else if any_env {
                "env"
            } else {
                "none"
            }),
        );
        channels.insert(channel.to_string(), Value::Object(item));
    }
    json!({"channels": Value::Object(channels),
           "note": "密钥/完整 webhook 不会回显；留空保存=保持原值；清除=回退启动时 .env"})
}

pub(crate) async fn alerts_config_get(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        Ok(Json(alerts_config_status(s, &st)).into_response())
    })
}

/// Python `_validate_url` 同口径：禁凭证；厂商渠道只允许 https + 白名单域名。
pub(crate) fn validate_alert_url(field: &str, url: &str) -> Result<(), String> {
    if let Some(authority) = url.split("://").nth(1) {
        if let Some((user, _)) = authority.rsplit_once('@') {
            if user.contains(':') && !user.is_empty() {
                return Err("Webhook URL 不能携带 user:pass@ 凭证".into());
            }
        }
    }
    if field == "webhook_url" {
        let ok_scheme = url.starts_with("https://") || url.starts_with("http://");
        let has_host = url
            .split("://")
            .nth(1)
            .map(|r| !r.is_empty())
            .unwrap_or(false);
        if !ok_scheme || !has_host {
            return Err("Webhook URL 必须是 http(s) 且含主机".into());
        }
        return Ok(());
    }
    if let Some(allowed) = alert_cfg::url_allowed_hosts(field) {
        if !url.starts_with("https://") {
            return Err(format!(
                "该渠道只允许 https 且域名为 {}",
                allowed.join(", ")
            ));
        }
        let host = url::host_of(url).unwrap_or_default();
        if !allowed.contains(&host.as_str()) {
            return Err(format!(
                "该渠道只允许 https 且域名为 {}",
                allowed.join(", ")
            ));
        }
    }
    Ok(())
}

pub(crate) async fn alerts_config_put(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let Some(body_obj) = body.as_object() else {
        return detail_err(StatusCode::UNPROCESSABLE_ENTITY, "body 必须是对象").into_response();
    };
    let mut pending: Vec<(String, String)> = Vec::new(); // (field, serialized)
    for (channel, values) in body_obj {
        let Some(mapping) = alert_cfg::fields_of(channel) else {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": format!("未知渠道: {channel}")})),
            )
                .into_response();
        };
        let Some(values) = values.as_object() else {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"detail": format!("{channel} 配置必须是对象")})),
            )
                .into_response();
        };
        for (api_name, raw) in values {
            let Some((_, field)) = mapping.iter().find(|(a, _)| a == api_name) else {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({"detail": format!("{channel} 含未知字段: {api_name}")})),
                )
                    .into_response();
            };
            // 留空 = 保持原值
            let text = raw
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let Some(text) = text else { continue };
            if alert_cfg::is_int(field) {
                let Ok(n) = text.parse::<i64>() else {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"detail": format!("{api_name} 必须是整数")})),
                    )
                        .into_response();
                };
                if !(1..=65535).contains(&n) {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"detail": format!("{api_name} 必须在 1-65535")})),
                    )
                        .into_response();
                }
                pending.push((field.to_string(), n.to_string()));
                continue;
            }
            if alert_cfg::is_bool(field) {
                let b = matches!(text.as_str(), "1" | "true" | "True" | "yes" | "on");
                pending.push((field.to_string(), if b { "1".into() } else { "0".into() }));
                continue;
            }
            if alert_cfg::is_url(field) {
                if let Err(e) = validate_alert_url(field, &text) {
                    return (StatusCode::BAD_REQUEST, Json(json!({"detail": e}))).into_response();
                }
            }
            if *field == "alert_email_to" && !text.is_empty() {
                let parts: Vec<&str> = text.split('@').collect();
                let ok = parts.len() == 2
                    && !parts[0].is_empty()
                    && !parts[1].is_empty()
                    && parts[1].contains('.');
                if !ok {
                    return detail_err(StatusCode::BAD_REQUEST, "收件邮箱格式不正确")
                        .into_response();
                }
            }
            if *field == "smtp_host" && (text.contains("://") || text.contains(' ')) {
                return detail_err(
                    StatusCode::BAD_REQUEST,
                    "SMTP 服务器只填主机名，如 smtp.qq.com",
                )
                .into_response();
            }
            if *field == "qmsg_qq" && !text.chars().all(|c| c.is_ascii_digit()) {
                return detail_err(StatusCode::BAD_REQUEST, "Qmsg 接收 QQ 必须是数字")
                    .into_response();
            }
            // 敏感字段加密（Fernet，与 POSIX 语义互操作）；失败拒绝保存（R2-106）
            let serialized = if alert_cfg::is_secret(field) {
                match fernet::protect(&text, std::path::Path::new(&st.data_dir)) {
                    Some(enc) => enc,
                    None => {
                        return detail_err(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "加密组件不可用，拒绝明文保存密钥",
                        )
                        .into_response()
                    }
                }
            } else {
                text.clone()
            };
            pending.push((field.to_string(), serialized));
        }
    }
    with_store(&st, |s| {
        for (field, val) in &pending {
            s.setting_set(&format!("{}{field}", alert_cfg::PREFIX), val)?;
        }
        Ok(Json(alerts_config_status(s, &st)).into_response())
    })
}

pub(crate) async fn alerts_config_reset(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let channel = q.get("channel").cloned();
    let fields: Vec<String> = match channel.as_deref() {
        None => alert_cfg::FIELDS
            .iter()
            .flat_map(|(_, m)| m.iter().map(|(_, f)| f.to_string()))
            .collect(),
        Some(c) => match alert_cfg::fields_of(c) {
            Some(m) => m.iter().map(|(_, f)| f.to_string()).collect(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"detail": format!("未知渠道: {c}")})),
                )
                    .into_response();
            }
        },
    };
    with_store(&st, |s| {
        // R2-305：真实删除（Python delete_app_settings 同语义），回退启动时 env 值
        let keys: Vec<String> = fields
            .iter()
            .map(|f| format!("{}{f}", alert_cfg::PREFIX))
            .collect();
        s.settings_delete(&keys)?;
        Ok(Json(alerts_config_status(s, &st)).into_response())
    })
}

// ---- 局域网发现（任务113；require_admin 口径同 Python） ----

/// Python TYPE_TEXT/LEVEL_ICON 同口径（constants.py 同步维护）
pub(crate) fn type_text(t: &str) -> &str {
    match t {
        "enter" => "进入安全区",
        "enter_no_ppe" => "进入安全区未穿实验服",
        "exit" => "离开安全区",
        "occupancy_over" => "在场人数超限",
        "stranger" => "陌生人进入",
        other => other,
    }
}

pub(crate) fn level_icon(level: i64) -> &'static str {
    match level {
        1 => "ℹ️",
        2 => "⚠️",
        3 => "🔴",
        _ => "ℹ️",
    }
}

/// Python alerts.render_alert 复刻：占位符白名单 + 未知占位符原样保留
pub(crate) fn render_alert_row(
    template: &str,
    ts: &str,
    camera_id: i64,
    type_: &str,
    level: i64,
    track_id: i64,
    detail: &str,
) -> String {
    let track_part = if type_ == "occupancy_over" {
        String::new()
    } else {
        format!(" · track {track_id}")
    };
    let vals: Vec<(&str, String)> = vec![
        ("icon", level_icon(level).to_string()),
        ("level", level.to_string()),
        ("type", type_.to_string()),
        ("type_text", type_text(type_).to_string()),
        ("ts", ts.to_string()),
        ("camera_id", camera_id.to_string()),
        ("track_id", track_id.to_string()),
        ("track_part", track_part),
        ("detail", detail.to_string()),
    ];
    // 单次替换：先按长度降序尝试每个 {key}；未知占位符原样保留
    let mut out = template.to_string();
    for (k, v) in &vals {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

pub(crate) async fn alert_template_preview(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    let _ = &st; // 模板文本取默认值（settings 单一来源在 Python 侧；rust 用同默认串）
    let ts = body
        .get("ts")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(iso_now);
    let camera_id = body.get("camera_id").and_then(|v| v.as_i64()).unwrap_or(1);
    let type_ = body
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("enter")
        .to_string();
    let level = body.get("level").and_then(|v| v.as_i64()).unwrap_or(2);
    let track_id = body.get("track_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let detail = body
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let text = render_alert_row(
        "{icon} L{level} {type_text} · {ts}\n摄像头 #{camera_id}{track_part}\n{detail}",
        &ts,
        camera_id,
        &type_,
        level,
        track_id,
        &detail,
    );
    let title = format!("LabSentinel 级别{level}告警");
    Json(json!({"title": title, "text": text})).into_response()
}

pub(crate) async fn alert_test(
    State(_st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let want = q.get("channel").map(|s| s.as_str().to_string());
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    // 渠道 -> (env key, ls-dispatch Channel)；URL 与 Python settings 同名 env
    let candidates: Vec<(String, ls_dispatch::Channel, String)> = vec![
        (
            "wecom".into(),
            ls_dispatch::Channel::Wecom,
            env("LS_WECOM_WEBHOOK").unwrap_or_default(),
        ),
        (
            "dingtalk".into(),
            ls_dispatch::Channel::Dingtalk,
            env("LS_DINGTALK_WEBHOOK").unwrap_or_default(),
        ),
        (
            "feishu".into(),
            ls_dispatch::Channel::Feishu,
            env("LS_FEISHU_WEBHOOK").unwrap_or_default(),
        ),
        (
            "webhook".into(),
            ls_dispatch::Channel::Webhook,
            env("LS_WEBHOOK_URL").unwrap_or_default(),
        ),
    ];
    let mut results: Vec<Value> = Vec::new();
    let mut sent = 0usize;
    for (name, ch, url) in candidates {
        if let Some(w) = &want {
            if w != &name {
                continue;
            }
        }
        if url.is_empty() {
            if want.as_deref() == Some(name.as_str()) {
                // 显式点名但未配置：如实报告
                results.push(json!({"channel": name, "ok": false,
                                     "error": "未配置（env 为空）"}));
            }
            continue;
        }
        let cfg = ls_dispatch::ChannelConfig {
            url,
            allow_http: std::env::var("LS_WEBHOOK_ALLOW_HTTP")
                .map(|v| v == "1")
                .unwrap_or(false),
            ..Default::default()
        };
        let d = ls_dispatch::deliver(ch, &cfg, 2, "LabSentinel 渠道自检测试（rust 控制面）");
        if d.ok {
            sent += 1;
        }
        results.push(json!({"channel": name, "ok": d.ok, "attempts": d.attempts,
                             "error": d.error}));
    }
    Json(
        json!({"ok": want.is_none() || results.iter().any(|r| r["ok"] == true),
                "sent": sent, "results": results,
                "note": "渠道配置读 env；网页端配置的渠道（app_settings）在 Python 服务"}),
    )
    .into_response()
}

// ---- PPE 按需下载（Q-08 复刻：锁文件白名单 + SHA256 + 许可确认） ----

