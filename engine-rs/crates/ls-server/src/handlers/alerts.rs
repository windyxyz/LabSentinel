// alert config (get/put/reset) + template render/preview + alert_test (alert_cfg meta shipped along).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::fernet;
use crate::state::Shared;
use crate::url;
use crate::util::*;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

pub(crate) async fn alerts_status(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let counts = s.deliveries(None, 10_000)?;
        let mut summary = std::collections::BTreeMap::new();
        let mut per_ch: std::collections::BTreeMap<String, (usize, usize, usize)> =
            Default::default();
        for d in &counts {
            *summary.entry(d.status.clone()).or_insert(0) += 1;
            let e = per_ch.entry(d.channel.clone()).or_insert((0, 0, 0));
            match d.status.as_str() {
                "ok" => e.0 += 1,
                "failed" => e.1 += 1,
                _ => e.2 += 1,
            }
        }
        let snapshot = st.jobs.alert_channels_snapshot();
        let all: &[(&str, ls_dispatch::Channel)] = &[
            ("wecom", ls_dispatch::Channel::Wecom),
            ("dingtalk", ls_dispatch::Channel::Dingtalk),
            ("feishu", ls_dispatch::Channel::Feishu),
            ("serverchan", ls_dispatch::Channel::Serverchan),
            ("pushplus", ls_dispatch::Channel::Pushplus),
            ("qmsg", ls_dispatch::Channel::Qmsg),
            ("email", ls_dispatch::Channel::Email),
            ("webhook", ls_dispatch::Channel::Webhook),
        ];
        let mut configured = serde_json::Map::new();
        let mut channels = Vec::new();
        for (name, ch) in all {
            let is_cfg = snapshot.iter().any(|(c, _)| c == ch);
            configured.insert((*name).to_string(), json!(is_cfg));
            let (ok, failed, pending) = per_ch.get(*name).copied().unwrap_or((0, 0, 0));
            channels.push(json!({
                "name": name,
                "configured": is_cfg,
                "deliveries": {"ok": ok, "failed": failed, "pending": pending},
            }));
        }
        Ok(Json(json!({
            "configured": Value::Object(configured),
            "channels": channels,
            "note": "渠道配置优先取网页端保存（app_settings，密钥加密），其次环境变量；此处汇总投递状态",
            "deliveries_summary": summary,
            "deliveries_total": counts.len(),
        }))
        .into_response())
    })
}

pub(crate) async fn alert_template_get(State(st): State<Shared>) -> Response {
    // 与 Python settings 默认模板 + placeholders 说明一致（任务85）。
    // UX（2026-11-21）：app_settings（alert.template.*）覆盖默认——网页端可编辑。
    with_store(&st, |s| {
        let title = s
            .setting_get("alert.template.title")
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "LabSentinel 级别{level}告警".into());
        let text = s
            .setting_get("alert.template.text")
            .ok()
            .flatten()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ALERT_TEMPLATE.into());
        Ok(Json(json!({
            "title": title,
            "text": text,
            "placeholders": {
                "icon": "级别图标（ℹ️/⚠️/🔴）", "level": "级别数字 1-3",
                "type": "事件类型标识", "type_text": "事件类型中文名",
                "ts": "事件时间", "camera_id": "摄像头 ID",
                "track_id": "跟踪 ID（occupancy 为 0）", "detail": "事件详情",
                "track_part": "「 · track N」或空串（occupancy 时为空）",
            },
        }))
        .into_response())
    })
}

/// PUT /api/alert-template——网页端保存告警文案（UX 2026-11-21）。
/// body {title, text}；空串=回默认（不落 override）；立即生效
/// （ls-jobs 渲染 override 同步 + 持久化 app_settings 跨重启）。
pub(crate) async fn alert_template_put(
    State(st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let text = body
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !text.is_empty() && !text.contains("{detail}") && !text.contains("{type_text}") {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "文案需至少包含 {detail} 或 {type_text} 占位符（否则告警内容无法区分事件）",
        )
        .into_response();
    }
    with_store(&st, |s| {
        if title.is_empty() {
            s.settings_delete(&["alert.template.title".to_string()])?;
        } else {
            s.setting_set("alert.template.title", &title)?;
        }
        if text.is_empty() {
            s.settings_delete(&["alert.template.text".to_string()])?;
        } else {
            s.setting_set("alert.template.text", &text)?;
        }
        // 运行时立即生效（清空时同步清除内存 override，不再滞留旧文案）
        if text.is_empty() {
            st.jobs.clear_alert_template();
        } else {
            st.jobs.set_alert_template(text.clone());
        }
        let effective = s
            .setting_get("alert.template.text")
            .ok()
            .flatten()
            .unwrap_or_else(|| DEFAULT_ALERT_TEMPLATE.into());
        Ok(Json(json!({"ok": true, "effective": effective})).into_response())
    })
}

/// R2-305/UX（2026-11-21）：从 app_settings（alert.*，Fernet 解密）组装渠道快照。
/// 启动注入与「网页保存后即时重建」共用同一构造——派发/测试/保存三处同源。
pub(crate) fn channels_from_settings(
    s: &ls_store::Store,
    data_dir: &std::path::Path,
) -> Vec<(ls_dispatch::Channel, ls_dispatch::ChannelConfig)> {
    let rows = s.settings_by_prefix("alert.").unwrap_or_default();
    if rows.is_empty() {
        return Vec::new();
    }
    let get = |field: &str| -> String {
        for (k, v) in &rows {
            if *k == format!("alert.{field}") {
                let f = &k["alert.".len()..];
                if alert_cfg::is_secret(f) {
                    return fernet::unprotect(v, data_dir);
                }
                return v.clone();
            }
        }
        String::new()
    };
    let mut channels: Vec<(ls_dispatch::Channel, ls_dispatch::ChannelConfig)> = Vec::new();
    let push = |name: &str,
                ch: ls_dispatch::Channel,
                cfg: ls_dispatch::ChannelConfig,
                configured: bool,
                out: &mut Vec<_>| {
        if configured {
            out.push((ch, cfg));
        } else {
            println!("[ls-server] 渠道 {name} 配置不完整，跳过");
        }
    };
    let wecom = get("wecom_webhook");
    push(
        "wecom",
        ls_dispatch::Channel::Wecom,
        ls_dispatch::ChannelConfig {
            url: wecom.clone(),
            ..Default::default()
        },
        !wecom.trim().is_empty(),
        &mut channels,
    );
    let dingtalk = get("dingtalk_webhook");
    push(
        "dingtalk",
        ls_dispatch::Channel::Dingtalk,
        ls_dispatch::ChannelConfig {
            url: dingtalk.clone(),
            ..Default::default()
        },
        !dingtalk.trim().is_empty(),
        &mut channels,
    );
    let feishu = get("feishu_webhook");
    push(
        "feishu",
        ls_dispatch::Channel::Feishu,
        ls_dispatch::ChannelConfig {
            url: feishu.clone(),
            ..Default::default()
        },
        !feishu.trim().is_empty(),
        &mut channels,
    );
    let sendkey = get("serverchan_sendkey");
    if !sendkey.trim().is_empty() {
        channels.push((
            ls_dispatch::Channel::Serverchan,
            ls_dispatch::ChannelConfig {
                url: format!("https://sctapi.ftqq.com/{}.send", sendkey.trim()),
                ..Default::default()
            },
        ));
    }
    let token = get("pushplus_token");
    if !token.trim().is_empty() {
        channels.push((
            ls_dispatch::Channel::Pushplus,
            ls_dispatch::ChannelConfig {
                url: "https://www.pushplus.plus/send".into(),
                token: token.clone(),
                ..Default::default()
            },
        ));
    }
    let key = get("qmsg_key");
    let qq = get("qmsg_qq");
    if !key.trim().is_empty() && !qq.trim().is_empty() {
        channels.push((
            ls_dispatch::Channel::Qmsg,
            ls_dispatch::ChannelConfig {
                url: format!("https://qmsg.zendee.cn/send/{}", key.trim()),
                key: key.clone(),
                qq: qq.clone(),
                ..Default::default()
            },
        ));
    }
    let wh = get("webhook_url");
    if !wh.trim().is_empty() {
        channels.push((
            ls_dispatch::Channel::Webhook,
            ls_dispatch::ChannelConfig {
                url: wh.clone(),
                secret: get("webhook_secret"),
                allow_http: get("webhook_allow_http") == "1",
                ..Default::default()
            },
        ));
    }
    let (sh, sp, su, spt, sto) = (
        get("smtp_host"),
        get("smtp_port"),
        get("smtp_user"),
        get("smtp_pass"),
        get("alert_email_to"),
    );
    if !sh.trim().is_empty() && !sto.trim().is_empty() {
        channels.push((
            ls_dispatch::Channel::Email,
            ls_dispatch::ChannelConfig {
                email: ls_dispatch::EmailConfig {
                    host: sh,
                    port: sp.parse().unwrap_or(465),
                    user: su,
                    pass: spt,
                    to: sto,
                    security: {
                        let s = get("smtp_security");
                        if s.is_empty() {
                            "ssl".to_string()
                        } else {
                            s
                        }
                    },
                },
                ..Default::default()
            },
        ));
    }
    if !channels.is_empty() {
        println!(
            "[ls-server] alert channels injected from app_settings: {} 渠道",
            channels.len()
        );
    }
    channels
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
                ("smtp_security", "smtp_security"),
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

pub(crate) async fn alerts_config_put(
    State(st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
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
            let text = raw.as_str().map(|s| s.trim().to_string());
            // UX 修复（2026-11-21 用户实测）：bool 字段以 JSON bool 提交（UI
            // checkbox v-model）——此前只处理字符串形式，bool 被 continue 静默
            // 丢弃 → allow_http=false → webhook「仅允许 https」全拒
            if alert_cfg::is_bool(field) {
                let b = match raw {
                    Value::Bool(v) => *v,
                    Value::String(v) => matches!(v.trim(), "1" | "true" | "True" | "yes" | "on"),
                    _ => false,
                };
                pending.push((field.to_string(), if b { "1".into() } else { "0".into() }));
                continue;
            }
            // LS-041：同时兼容 JSON Number（如前端 `smtp_port: 587`）与字符串形式，避免被静默丢弃
            if alert_cfg::is_int(field) {
                let parsed = match raw {
                    Value::Number(num) => num.as_i64(),
                    Value::String(s) => {
                        let t = s.trim();
                        if t.is_empty() {
                            continue;
                        }
                        t.parse::<i64>().ok()
                    }
                    Value::Null => continue,
                    _ => None,
                };
                let Some(n) = parsed else {
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
            let Some(text) = text.filter(|s| !s.is_empty()) else {
                continue;
            };
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
        // UX（2026-11-21）：保存即生效——重建渠道快照注入派发（此前要重启服务）
        st.jobs.set_alert_channels(channels_from_settings(
            s,
            std::path::Path::new(&st.data_dir),
        ));
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
        // UX（2026-11-21）：清除同样即时重建快照（回退到 env/默认）
        st.jobs.set_alert_channels(channels_from_settings(
            s,
            std::path::Path::new(&st.data_dir),
        ));
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

/// 默认告警文案（与 ls-jobs escalation::DEFAULT_TEMPLATE 同串；单一来源归 jobs，
/// 此处为渲染/回退副本——改动须两处同步）。
pub(crate) const DEFAULT_ALERT_TEMPLATE: &str =
    "{icon} L{level} {type_text} · {ts}\n摄像头 #{camera_id}{track_part}\n{detail}";

pub(crate) async fn alert_template_preview(
    State(st): State<Shared>,
    Json(body): Json<Value>,
) -> Response {
    // UX（2026-11-21）：用当前生效模板渲染（app_settings override > 默认），
    // 保证「预览」所见 = 升级提醒实际收到。
    let effective = ls_store::Store::open(&st.db_path)
        .ok()
        .and_then(|s| s.setting_get("alert.template.text").ok().flatten())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ALERT_TEMPLATE.into());
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
    // UX（2026-11-21）：body 带 text（前端「用当前内容预览」未保存的编辑稿）
    // 时直渲编辑稿，否则用已保存的生效模板
    let body_text = body
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let text = render_alert_row(
        body_text.as_deref().unwrap_or(&effective),
        &ts,
        camera_id,
        &type_,
        level,
        track_id,
        &detail,
    );
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("LabSentinel 级别{level}告警"));
    Json(json!({"title": title, "text": text})).into_response()
}

pub(crate) async fn alert_test(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let want = q.get("channel").map(|s| s.as_str().to_string());
    // UX 修复（2026-11-21 用户实测）：测试必须与实际派发**同源**——此前自读
    // env（网页保存的 SMTP/渠道凭据被无视）且 email/serverchan 等不在清单，
    // 「保存后点发送测试」永远「未知错误」。改用 jobs 渠道快照（启动注入 +
    // ApplyConfig 更新，与派发同一数据源）。
    let snapshot = st.jobs.alert_channels_snapshot();
    let all: &[(&str, ls_dispatch::Channel)] = &[
        ("wecom", ls_dispatch::Channel::Wecom),
        ("dingtalk", ls_dispatch::Channel::Dingtalk),
        ("feishu", ls_dispatch::Channel::Feishu),
        ("serverchan", ls_dispatch::Channel::Serverchan),
        ("pushplus", ls_dispatch::Channel::Pushplus),
        ("qmsg", ls_dispatch::Channel::Qmsg),
        ("email", ls_dispatch::Channel::Email),
        ("webhook", ls_dispatch::Channel::Webhook),
    ];
    let mut results: Vec<Value> = Vec::new();
    let mut sent = 0usize;
    for (name, ch) in all {
        if let Some(w) = &want {
            if w != name {
                continue;
            }
        }
        match snapshot.iter().find(|(c, _)| c == ch) {
            Some((_, cfg)) => {
                let d = ls_dispatch::deliver(*ch, cfg, 2, "LabSentinel 渠道自检测试");
                if d.ok {
                    sent += 1;
                }
                results.push(json!({"channel": name, "ok": d.ok, "attempts": d.attempts,
                                     "error": d.error}));
            }
            None => {
                if want.as_deref() == Some(*name) {
                    results.push(json!({"channel": name, "ok": false,
                                         "error": "未配置——请先在网页保存该渠道凭据（或设置对应环境变量）"}));
                }
            }
        }
    }
    if let Some(w) = &want {
        if !all.iter().any(|(n, _)| n == w) {
            results.push(json!({"channel": w, "ok": false, "error": "未知渠道"}));
        }
    }
    Json(json!({"ok": sent > 0,
                "sent": sent, "results": results,
                "note": "渠道凭据与实际派发同源：网页保存（app_settings）优先，其次环境变量"}))
    .into_response()
}

/// UX（2026-11-21）：推送策略——告警随消息带截图开关 + 同人推送冷却秒数。
/// GET /api/alerts/push-policy；PUT 同形状（即写即生效——派发每次现读设置）。
pub(crate) async fn push_policy_get(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        Ok(Json(json!({
            "send_image": push_send_image(s),
            "cooldown_s": push_cooldown_s(s),
        }))
        .into_response())
    })
}

pub(crate) async fn push_policy_put(State(st): State<Shared>, Json(body): Json<Value>) -> Response {
    with_store(&st, |s| {
        if let Some(b) = body.get("send_image") {
            match b.as_bool() {
                Some(v) => {
                    s.setting_set("alert.send_image", if v { "1" } else { "0" })?;
                }
                None => {
                    return Ok(detail_err(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "send_image 必须是布尔",
                    )
                    .into_response())
                }
            }
        }
        if let Some(c) = body.get("cooldown_s") {
            let n = c
                .as_i64()
                .or_else(|| c.as_str().and_then(|v| v.trim().parse().ok()));
            match n {
                Some(v) if (0..=3600).contains(&v) => {
                    s.setting_set("alert.push_cooldown_s", &v.to_string())?;
                }
                _ => {
                    return Ok(detail_err(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "cooldown_s 必须在 0-3600（秒；0=关闭冷却）",
                    )
                    .into_response())
                }
            }
        }
        Ok(Json(json!({
            "ok": true,
            "send_image": push_send_image(s),
            "cooldown_s": push_cooldown_s(s),
        }))
        .into_response())
    })
}

/// 派发时读取（与 ls-jobs dispatch_event 同键同默认）。
fn push_send_image(s: &ls_store::Store) -> bool {
    s.setting_get("alert.send_image")
        .ok()
        .flatten()
        .map(|v| v.trim() != "0")
        .unwrap_or(true)
}

fn push_cooldown_s(s: &ls_store::Store) -> i64 {
    s.setting_get("alert.push_cooldown_s")
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(60)
}

// ---- PPE 按需下载（Q-08 复刻：锁文件白名单 + SHA256 + 许可确认） ----
