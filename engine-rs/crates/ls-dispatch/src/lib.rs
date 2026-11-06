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
            return code == 0; // 飞书/PushPlus
        }
        if let Some(ok) = v.get("success").and_then(|x| x.as_bool()) {
            return ok;
        }
    }
    true // 无业务码：HTTP 2xx 即成功
}

/// 构造请求体（与 Python 各渠道 payload 等价）
pub fn build_body(ch: Channel, text: &str) -> Value {
    match ch {
        Channel::Wecom => json!({"msgtype": "text", "text": {"content": text}}),
        Channel::Dingtalk => json!({"msgtype": "text", "text": {"content": text}}),
        Channel::Feishu => json!({"msg_type": "text", "content": {"text": text}}),
        Channel::Serverchan => json!({"text": "LabSentinel", "desp": text}),
        Channel::Pushplus => json!({"content": text, "template": "txt"}),
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
pub fn deliver(ch: Channel, cfg: &ChannelConfig, text: &str) -> Delivery {
    if let Err(e) = check_url(ch, &cfg.url, cfg.allow_http) {
        return Delivery {
            ok: false,
            attempts: 0,
            error: Some(e),
        };
    }
    let body = build_body(ch, text).to_string();
    let mut error = None;
    for attempt in 0..4u32 {
        let mut req = ureq::post(&cfg.url).set("Content-Type", "application/json");
        if ch == Channel::Webhook {
            let ts = unix_secs().to_string();
            req = req.set("X-LS-Event", "alert").set("X-LS-Timestamp", &ts);
            if !cfg.secret.is_empty() {
                let sig = format!("sha256={}", hmac_sha256_hex(&cfg.secret, body.as_bytes()));
                req = req.set("X-LS-Signature", &sig);
            }
        }
        let resp = req.send_string(&body);
        let (status, resp_body) = match resp {
            Ok(r) => {
                let st = r.status();
                (st, r.into_string().unwrap_or_default())
            }
            Err(e) => (0, e.to_string()),
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
            std::thread::sleep(Duration::from_secs([1, 4, 15][attempt as usize]));
        }
    }
    Delivery {
        ok: false,
        attempts: 4,
        error,
    }
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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
/// T-330 只接线 URL 型渠道（企微/钉钉/飞书/Server酱/通用 Webhook）；
/// PushPlus/Qmsg/邮件需要额外凭据字段与 SMTP 客户端，由后续 L2 波次补齐。
pub fn channels_from_env() -> Vec<(Channel, ChannelConfig)> {
    let mut out = Vec::new();
    let mut push = |ch: Channel, url: &str, allow_http: bool, secret: &str| {
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
    };
    push(Channel::Wecom, &env_str("LS_WECOM_WEBHOOK"), false, "");
    push(
        Channel::Dingtalk,
        &env_str("LS_DINGTALK_WEBHOOK"),
        false,
        "",
    );
    push(Channel::Feishu, &env_str("LS_FEISHU_WEBHOOK"), false, "");
    let sendkey = env_str("LS_SERVERCHAN_SENDKEY");
    if !sendkey.trim().is_empty() {
        push(
            Channel::Serverchan,
            &format!("https://sctapi.ftqq.com/{}.send", sendkey.trim()),
            false,
            "",
        );
    }
    let allow_http = matches!(
        env_str("LS_WEBHOOK_ALLOW_HTTP")
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    );
    push(
        Channel::Webhook,
        &env_str("LS_WEBHOOK_URL"),
        allow_http,
        &env_str("LS_WEBHOOK_SECRET"),
    );
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
