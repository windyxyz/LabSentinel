//! ls-dispatch（T-329/L2）：告警渠道派发的 Rust 等价实现。
//!
//! 与 Python `alerts.py` 语义对齐：
//! - 厂商渠道域名白名单 + 强制 https + 不跟随重定向；业务码判定（errcode=0 等）
//! - 静默匹配 / 渠道过滤 / 投递记录（alert_deliveries）/ 重试退避 / 死信
//! - 每渠道令牌桶（20/min）+ 溢出摘要（T-305）
//! - 通用 Webhook HMAC 签名（LS_WEBHOOK_SECRET）

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
// T-329：邮件渠道（Transport trait 供 SmtpTransport::send）
use lettre::Transport;

pub mod mqtt;

/// 渠道名（与 Python ch.name 一致）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Wecom,
    Dingtalk,
    Feishu,
    Serverchan,
    Pushplus,
    Qmsg,
    Email,
    Webhook,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Wecom => "wecom",
            Channel::Dingtalk => "dingtalk",
            Channel::Feishu => "feishu",
            Channel::Serverchan => "serverchan",
            Channel::Pushplus => "pushplus",
            Channel::Qmsg => "qmsg",
            Channel::Email => "email",
            Channel::Webhook => "webhook",
        }
    }
    pub fn all() -> [Channel; 8] {
        [
            Channel::Wecom,
            Channel::Dingtalk,
            Channel::Feishu,
            Channel::Serverchan,
            Channel::Pushplus,
            Channel::Qmsg,
            Channel::Email,
            Channel::Webhook,
        ]
    }
    /// 允许的 host（空 = 由配置 URL 白名单校验）
    pub fn allowed_hosts(self) -> &'static [&'static str] {
        match self {
            Channel::Wecom => &["qyapi.weixin.qq.com"],
            Channel::Dingtalk => &["oapi.dingtalk.com"],
            Channel::Feishu => &["open.feishu.cn"],
            Channel::Serverchan => &["sctapi.ftqq.com"],
            Channel::Pushplus => &["www.pushplus.plus"],
            Channel::Qmsg => &["qmsg.zendee.cn"],
            Channel::Email => &[],
            Channel::Webhook => &[],
        }
    }
}

/// 渠道配置（由 settings / alert_config 传入，凭据只进内存）
#[derive(Clone, Default)]
pub struct ChannelConfig {
    pub url: String,
    pub secret: String,
    pub token: String,
    pub key: String,
    pub allow_http: bool,
    /// T-329：Qmsg 接收 QQ 号（key 走 url 路径段）
    pub qq: String,
    /// T-329：邮件渠道（SMTP）
    pub email: EmailConfig,
}

/// SMTP 邮件渠道配置（Python EmailChannel 同名字段；口令只进内存）
#[derive(Clone, Default)]
pub struct EmailConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub to: String,
    /// ssl（默认 465）| starttls（587）| none（仅私网，T-210 F-08）
    pub security: String,
}

/// 事件行（派发所需字段，不含本地绝对路径）
#[derive(Clone)]
pub struct DispatchEvent {
    pub id: i64,
    pub camera_id: i64,
    pub type_: String,
    pub level: i64,
    pub detail: String,
    pub ts: String,
    pub camera_name: String,
}

/// 静默规则（Python match_silence 等价：scope=all|camera|level|type）
#[derive(Clone)]
pub struct Silence {
    pub scope: String,
    pub target: String,
    pub until_ts: String,
}

pub fn match_silence(silences: &[Silence], ev: &DispatchEvent, now: &str) -> Option<usize> {
    for (i, s) in silences.iter().enumerate() {
        if s.until_ts.as_str() <= now {
            continue;
        }
        let hit = match s.scope.as_str() {
            // Python 用 "global"；旧版本文档写过 "all"，两者都接受
            "global" | "all" => true,
            "camera" => s.target == ev.camera_id.to_string(),
            "level" => s.target == ev.level.to_string(),
            "type" => s.target == ev.type_,
            _ => false,
        };
        if hit {
            return Some(i);
        }
    }
    None
}

/// 渠道过滤（Python channel_filtered 等价：min_level / types 列表）
pub fn channel_filtered(
    ch: Channel,
    ev: &DispatchEvent,
    filters: &HashMap<String, Value>,
) -> Option<String> {
    let f = filters.get(ch.name())?;
    if let Some(min) = f.get("min_level").and_then(|v| v.as_i64()) {
        if ev.level < min {
            return Some(format!("level {}<{}", ev.level, min));
        }
    }
    if let Some(types) = f.get("types").and_then(|v| v.as_array()) {
        if !types.iter().any(|t| t.as_str() == Some(ev.type_.as_str())) {
            return Some(format!("type {} 不在白名单", ev.type_));
        }
    }
    None
}

/// URL 安全校验：https（或显式 allow_http）+ host 白名单（防 SSRF/凭据外泄）。
pub fn check_url(ch: Channel, url: &str, allow_http: bool) -> Result<(), String> {
    let (scheme, host) = split_url(url);
    if scheme != "https" && !(allow_http && scheme == "http") {
        return Err(format!("{}: 仅允许 https", ch.name()));
    }
    let hosts = ch.allowed_hosts();
    if !hosts.is_empty() && !hosts.iter().any(|h| host.eq_ignore_ascii_case(h)) {
        return Err(format!("{}: host {} 不在白名单", ch.name(), host));
    }
    Ok(())
}

fn split_url(url: &str) -> (&str, &str) {
    let s = url.split_once("://").unwrap_or(("", url));
    let host =
        s.1.split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("");
    (s.0, host)
}

/// 厂商响应业务码判定（Python `_is_ok` 等价）
pub fn is_ok(_ch: Channel, status: u16, body: &str) -> bool {
    if !(200..300).contains(&status) {
        return false;
    }
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(code) = v.get("errcode").and_then(|x| x.as_i64()) {
            return code == 0; // 企微
        }
        if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
            return code == 0 || code == 200; // 飞书=0 / PushPlus=200（Python `_is_ok` 同 (0,200)）
        }
        if let Some(ok) = v.get("success").and_then(|x| x.as_bool()) {
            return ok;
        }
    }
    true // 无业务码：HTTP 2xx 即成功
}

/// 告警标题（Python alert_title 等价：LS_ALERT_TEMPLATE_TITLE 模板，{level} 替换；
/// 未知占位符按 Python SafeDict 语义原样保留）
pub fn alert_title(level: i64) -> String {
    let tpl = env_str("LS_ALERT_TEMPLATE_TITLE");
    let tpl = if tpl.is_empty() {
        "LabSentinel 级别{level}告警".to_string()
    } else {
        tpl
    };
    tpl.replace("{level}", &level.to_string())
}

/// 构造请求体（与 Python 各渠道 payload 等价；PushPlus 带 token + 模板标题）
pub fn build_body(ch: Channel, level: i64, text: &str, cfg: &ChannelConfig) -> Value {
    match ch {
        Channel::Wecom => json!({"msgtype": "text", "text": {"content": text}}),
        Channel::Dingtalk => json!({"msgtype": "text", "text": {"content": text}}),
        Channel::Feishu => json!({"msg_type": "text", "content": {"text": text}}),
        Channel::Serverchan => json!({"text": "LabSentinel", "desp": text}),
        Channel::Pushplus => json!({
            "token": cfg.token,
            "title": alert_title(level),
            "content": text,
            "template": "txt",
        }),
        Channel::Qmsg => json!({"msg": text}),
        Channel::Email | Channel::Webhook => json!({"text": text}),
    }
}

/// 每渠道令牌桶（T-305：20/min，超限累计摘要）
pub struct RateLimiter {
    inner: Mutex<HashMap<&'static str, (VecDeque<Instant>, u64)>>,
}
impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        RateLimiter {
            inner: Mutex::new(HashMap::new()),
        }
    }
    /// 返回 (放行?, 待合并数)
    pub fn gate(&self, ch: Channel) -> (bool, u64) {
        let mut m = self.inner.lock().unwrap();
        let (q, pending) = m.entry(ch.name()).or_insert_with(|| (VecDeque::new(), 0));
        let now = Instant::now();
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            q.pop_front();
        }
        if q.len() < 20 {
            q.push_back(now);
            let n = *pending;
            *pending = 0;
            (true, n)
        } else {
            *pending += 1;
            (false, *pending)
        }
    }
    pub fn pending_count(&self, ch: Channel) -> u64 {
        self.inner
            .lock()
            .unwrap()
            .get(ch.name())
            .map(|(_, p)| *p)
            .unwrap_or(0)
    }
}

/// 派发结果（供投递记录：ok / failed + attempts）
pub struct Delivery {
    pub ok: bool,
    pub attempts: u32,
    pub error: Option<String>,
}

/// 单渠道投递（真实 HTTP；失败按 Python RETRY_BACKOFF=[1,4,15] 重试）。
/// Email 渠道不走 HTTP，转 `deliver_email`（由派发层路由）。
pub fn deliver(ch: Channel, cfg: &ChannelConfig, level: i64, text: &str) -> Delivery {
    deliver_with_backoff(ch, cfg, level, text, &[1, 4, 15])
}

/// N-34/R2-505：退避可注入（测试用 `[0,0,0]` 消除 20s 真实等待与桩计数竞态窗口）。
pub fn deliver_with_backoff(
    ch: Channel,
    cfg: &ChannelConfig,
    level: i64,
    text: &str,
    backoff_secs: &[u64],
) -> Delivery {
    if ch == Channel::Email {
        return deliver_email(cfg, level, text);
    }
    if let Err(e) = check_url(ch, &cfg.url, cfg.allow_http) {
        return Delivery {
            ok: false,
            attempts: 0,
            error: Some(e),
        };
    }
    let mut error = None;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .http_status_as_error(false)
        .build()
        .into();
    for attempt in 0..4u32 {
        // Python httpx timeout=5 同口径
        let req = agent.post(&cfg.url);
        let (status, resp_body) = match ch {
            // Qmsg：表单 qq+msg（Python data= dict 同口径）
            Channel::Qmsg => resp_body(req.send_form([("qq", cfg.qq.as_str()), ("msg", text)])),
            _ => {
                let body = build_body(ch, level, text, cfg).to_string();
                let req = req.header("Content-Type", "application/json");
                let req = if ch == Channel::Webhook {
                    let ts = unix_secs().to_string();
                    let r = req
                        .header("X-LS-Event", "alert")
                        .header("X-LS-Timestamp", &ts);
                    if !cfg.secret.is_empty() {
                        let sig =
                            format!("sha256={}", hmac_sha256_hex(&cfg.secret, body.as_bytes()));
                        r.header("X-LS-Signature", &sig)
                    } else {
                        r
                    }
                } else {
                    req
                };
                resp_body(req.send(body.as_str()))
            }
        };
        if is_ok(ch, status, &resp_body) {
            return Delivery {
                ok: true,
                attempts: attempt + 1,
                error: None,
            };
        }
        error = Some(format!(
            "HTTP {}: {}",
            status,
            resp_body.chars().take(120).collect::<String>()
        ));
        if attempt < 3 {
            // 退避可注入（N-34）：越界/0 值不等待
            if let Some(&s) = backoff_secs.get(attempt as usize) {
                if s > 0 {
                    std::thread::sleep(Duration::from_secs(s));
                }
            }
        }
    }
    Delivery {
        ok: false,
        attempts: 4,
        error,
    }
}

fn resp_body(r: Result<http::Response<ureq::Body>, ureq::Error>) -> (u16, String) {
    match r {
        Ok(mut resp) => {
            let st = resp.status().as_u16();
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            (st, body)
        }
        Err(ureq::Error::StatusCode(code)) => (code, String::new()),
        Err(e) => (0, e.to_string()),
    }
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 邮件正文/信头构造（Python EmailChannel 同口径：MIME plain utf-8、
/// 主题 `[LabSentinel] 级别{level}告警`、From=smtp_user、To=alert_email_to）。
pub fn build_email(level: i64, text: &str, cfg: &EmailConfig) -> Result<lettre::Message, String> {
    let from = if cfg.user.is_empty() {
        "labsentinel@localhost".to_string()
    } else {
        cfg.user.clone()
    };
    lettre::Message::builder()
        .from(from.parse().map_err(|e| format!("from 地址非法: {e}"))?)
        .to(cfg.to.parse().map_err(|e| format!("to 地址非法: {e}"))?)
        .subject(format!("[LabSentinel] 级别{level}告警"))
        .body(text.to_string())
        .map_err(|e| format!("邮件构造失败: {e}"))
}

/// T-210（F-08）：security=none 仅允许私网/回环 SMTP（明文口令风险）。
/// Python ipaddress.is_private 的 IPv4 语义 + IPv6 ULA fc00::/7。
fn is_private_host(host: &str) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(mut addrs) = (host, 0u16).to_socket_addrs() else {
        return false; // 解析失败按不允许处理（与 Python gethostbyname 抛错路径等价）
    };
    addrs.any(|a| match a.ip() {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback(),
        std::net::IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
    })
}

/// 邮件渠道投递（单次尝试，与 Python 无重试同口径；超时 10s）。
pub fn deliver_email(cfg: &ChannelConfig, level: i64, text: &str) -> Delivery {
    let ec = &cfg.email;
    if ec.host.is_empty() || ec.to.is_empty() {
        return Delivery {
            ok: false,
            attempts: 0,
            error: Some("email 未配置（host/to 缺失）".into()),
        };
    }
    let email = match build_email(level, text, ec) {
        Ok(m) => m,
        Err(e) => {
            return Delivery {
                ok: false,
                attempts: 0,
                error: Some(e),
            }
        }
    };
    let build = |f: Result<
        lettre::transport::smtp::SmtpTransportBuilder,
        lettre::transport::smtp::Error,
    >| { f.map(|b| b.port(ec.port)).map_err(|e| format!("{e}")) };
    let builder = match ec.security.to_ascii_lowercase().as_str() {
        "ssl" => build(lettre::SmtpTransport::relay(&ec.host)),
        "starttls" => build(lettre::SmtpTransport::starttls_relay(&ec.host)),
        "none" => {
            if !is_private_host(&ec.host) {
                return Delivery {
                    ok: false,
                    attempts: 0,
                    error: Some("LS_SMTP_SECURITY=none 仅允许私网 SMTP（明文口令风险）".into()),
                };
            }
            Ok(lettre::SmtpTransport::builder_dangerous(&ec.host).port(ec.port))
        }
        other => {
            return Delivery {
                ok: false,
                attempts: 0,
                error: Some(format!("未知 LS_SMTP_SECURITY: {other}")),
            }
        }
    };
    let builder = match builder {
        Ok(b) => b,
        Err(e) => {
            return Delivery {
                ok: false,
                attempts: 0,
                error: Some(format!("SMTP 传输构造失败: {e}")),
            }
        }
    };
    let builder = if !ec.user.is_empty() {
        builder.credentials(lettre::transport::smtp::authentication::Credentials::new(
            ec.user.clone(),
            ec.pass.clone(),
        ))
    } else {
        builder
    };
    let transport = builder.timeout(Some(Duration::from_secs(10))).build();
    match transport.send(&email) {
        Ok(_) => Delivery {
            ok: true,
            attempts: 1,
            error: None,
        },
        Err(e) => Delivery {
            ok: false,
            attempts: 1,
            error: Some(format!("send failed: {e}")),
        },
    }
}

/// 对原始请求体字节做 HMAC-SHA256（Python GenericWebhookChannel 同口径）。
fn hmac_sha256_hex(secret: &str, body: &[u8]) -> String {
    use hmac::Mac;
    type HmacSha256 = hmac::Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC 接受任意长度密钥");
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn env_str(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

/// 从环境变量构造已配置渠道（与 Python settings 同名）。
///
/// T-329：8 渠道全量接线——PushPlus（token）/Qmsg（key+qq）/Email（SMTP）。
pub fn channels_from_env() -> Vec<(Channel, ChannelConfig)> {
    let mut out = Vec::new();
    push_url(
        &mut out,
        Channel::Wecom,
        &env_str("LS_WECOM_WEBHOOK"),
        false,
        "",
    );
    push_url(
        &mut out,
        Channel::Dingtalk,
        &env_str("LS_DINGTALK_WEBHOOK"),
        false,
        "",
    );
    push_url(
        &mut out,
        Channel::Feishu,
        &env_str("LS_FEISHU_WEBHOOK"),
        false,
        "",
    );
    let sendkey = env_str("LS_SERVERCHAN_SENDKEY");
    if !sendkey.trim().is_empty() {
        push_url(
            &mut out,
            Channel::Serverchan,
            &format!("https://sctapi.ftqq.com/{}.send", sendkey.trim()),
            false,
            "",
        );
    }
    let pushplus = env_str("LS_PUSHPLUS_TOKEN");
    if !pushplus.trim().is_empty() {
        out.push((
            Channel::Pushplus,
            ChannelConfig {
                url: "https://www.pushplus.plus/send".into(),
                token: pushplus.trim().to_string(),
                ..Default::default()
            },
        ));
    }
    let (qkey, qqq) = (env_str("LS_QMSG_KEY"), env_str("LS_QMSG_QQ"));
    if !qkey.trim().is_empty() && !qqq.trim().is_empty() {
        out.push((
            Channel::Qmsg,
            ChannelConfig {
                url: format!("https://qmsg.zendee.cn/send/{}", percent_encode(&qkey)),
                qq: qqq.trim().to_string(),
                ..Default::default()
            },
        ));
    }
    let eh = env_str("LS_SMTP_HOST");
    let eto = env_str("LS_ALERT_EMAIL_TO");
    if !eh.trim().is_empty() && !eto.trim().is_empty() {
        let port = env_str("LS_SMTP_PORT").trim().parse::<u16>().unwrap_or(465); // Python settings.smtp_port 默认 465
        let security = {
            let s = env_str("LS_SMTP_SECURITY").trim().to_ascii_lowercase();
            if s.is_empty() {
                "ssl".to_string()
            } else {
                s
            }
        };
        out.push((
            Channel::Email,
            ChannelConfig {
                email: EmailConfig {
                    host: eh.trim().to_string(),
                    port,
                    user: env_str("LS_SMTP_USER").trim().to_string(),
                    pass: env_str("LS_SMTP_PASS"),
                    to: eto.trim().to_string(),
                    security,
                },
                ..Default::default()
            },
        ));
    }
    let allow_http = matches!(
        env_str("LS_WEBHOOK_ALLOW_HTTP")
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    );
    push_url(
        &mut out,
        Channel::Webhook,
        &env_str("LS_WEBHOOK_URL"),
        allow_http,
        &env_str("LS_WEBHOOK_SECRET"),
    );
    out
}

fn push_url(
    out: &mut Vec<(Channel, ChannelConfig)>,
    ch: Channel,
    url: &str,
    allow_http: bool,
    secret: &str,
) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    out.push((
        ch,
        ChannelConfig {
            url: url.to_string(),
            secret: secret.trim().to_string(),
            allow_http,
            ..Default::default()
        },
    ));
}

/// Qmsg key 的路径段编码（Python `quote(key, safe='')` 等价：RFC 3986 保留集外全编码）。
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_whitelist_blocks_ssrf() {
        assert!(check_url(Channel::Wecom, "https://qyapi.weixin.qq.com/x", false).is_ok());
        assert!(check_url(Channel::Wecom, "http://qyapi.weixin.qq.com/x", false).is_err());
        assert!(check_url(Channel::Wecom, "https://evil.example/x", false).is_err());
    }

    #[test]
    fn business_code_semantics() {
        assert!(is_ok(Channel::Wecom, 200, "{\"errcode\":0}"));
        assert!(!is_ok(Channel::Wecom, 200, "{\"errcode\":40001}"));
        assert!(!is_ok(Channel::Wecom, 500, ""));
    }

    #[test]
    fn silence_and_filter_equivalent() {
        let ev = DispatchEvent {
            id: 1,
            camera_id: 3,
            type_: "enter".into(),
            level: 2,
            detail: String::new(),
            ts: String::new(),
            camera_name: String::new(),
        };
        let sil = vec![Silence {
            scope: "camera".into(),
            target: "3".into(),
            until_ts: "9999".into(),
        }];
        assert_eq!(match_silence(&sil, &ev, "2026-01-01"), Some(0));
        let mut f = HashMap::new();
        f.insert("wecom".to_string(), json!({"min_level": 3}));
        assert!(channel_filtered(Channel::Wecom, &ev, &f).is_some());
    }

    #[test]
    fn rate_limiter_merges_overflow() {
        let rl = RateLimiter::new();
        for _ in 0..20 {
            assert!(rl.gate(Channel::Wecom).0);
        }
        let (allowed, pending) = rl.gate(Channel::Wecom);
        assert!(!allowed && pending == 1);
        assert_eq!(rl.pending_count(Channel::Wecom), 1);
    }
}
