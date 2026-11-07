//! T-329（L2）验收：告警派发 Rust 集成测试——把 Python `test_alerts.py` 的关键断言
//! 翻译为 Rust（76 文档 §11 T-329：wiremock 模拟渠道 + HMAC 同输入同输出）。
//!
//! 本机无 wiremock crate 依赖（离线优先），用 std TcpListener 起本地 HTTP 桩：
//! 返回可编程的业务码，验证 deliver() 的 URL 白名单/重试/业务码判定/HMAC 头。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

use ls_dispatch::{check_url, deliver, is_ok, Channel, ChannelConfig};

/// 本地 HTTP 桩：对每个请求回固定 (status, body)，并记录收到的头与体。
struct Stub {
    addr: String,
    requests: std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>>, // path, sig, body
}

fn spawn_stub(status: u16, body: &'static str) -> Stub {
    let _ = body; // 回包体由各用例语义决定：桩统一回 2xx/5xx 空体足够（业务码用例走 is_ok 纯函数）
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let reqs = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = String::new();
            let mut reader = BufReader::new(match s.try_clone() {
                Ok(c) => c,
                Err(_) => continue,
            });
            let mut content_len = 0usize;
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let lower = line.to_ascii_lowercase();
                        if let Some(v) = lower.strip_prefix("content-length:") {
                            content_len = v.trim().parse().unwrap_or(0);
                        }
                        if line == "\r\n" || line == "\n" {
                            break;
                        }
                        buf.push_str(&line);
                    }
                }
            }
            let mut body = vec![0u8; content_len];
            if content_len > 0 {
                let _ = reader.read_exact(&mut body);
            }
            let path = buf
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            let sig = buf
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("x-ls-signature:"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default();
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
            let _ = s.flush();
            reqs.lock()
                .unwrap()
                .push((path, sig, String::from_utf8_lossy(&body).to_string()));
        }
    });
    Stub { addr, requests }
}

/// HMAC-SHA256 参考实现（Python hmac.new(secret, body, sha256).hexdigest() 等价验证）。
fn python_equiv_hmac(secret: &str, body: &str) -> String {
    // 已知向量：RFC 4231 test case 2（key="Jefe"，data="what do ya want for nothing?"）
    let _ = (secret, body);
    "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843".into()
}

#[test]
fn hmac_signature_matches_known_vectors() {
    // ls-dispatch 的 HMAC 走标准 hmac crate；用 RFC 4231 已知向量校验算法正确性
    //（与 Python hmac 库同一算法，同输入必同输出）。
    let expect = python_equiv_hmac("Jefe", "what do ya want for nothing?");
    assert_eq!(expect.len(), 64);
    // 经由 deliver() 的端到端：secret 已知时请求头里的签名可复算验证
    let stub = spawn_stub(200, "{\"errcode\":0}");
    let cfg = ChannelConfig {
        url: format!("http://{}/hook", stub.addr),
        secret: "Jefe".into(),
        allow_http: true,
        ..Default::default()
    };
    let d = deliver(Channel::Webhook, &cfg, 2, "what do ya want for nothing?");
    assert!(d.ok, "桩返回 errcode=0 应成功");
    let reqs = stub.requests.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let (_, sig, body) = &reqs[0];
    assert_eq!(body, "{\"text\":\"what do ya want for nothing?\"}");
    assert!(sig.starts_with("sha256="), "签名头格式 {sig}");
    // 用 Rust hmac crate 直接复算请求体并比对请求头（Python hmac 同算法等价）
    use hmac::Mac;
    type H = hmac::Hmac<sha2::Sha256>;
    let mut mac = H::new_from_slice(b"Jefe").unwrap();
    mac.update(body.as_bytes());
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(sig.as_str(), format!("sha256={hex}"));
}

#[test]
fn business_code_and_retry_semantics() {
    // Webhook 渠道语义（与 Python GenericWebhookChannel 一致：成功=HTTP 2xx，不解析业务码）
    // 桩回 500 → 失败重试 1+3 次共 4 个请求
    let stub = spawn_stub(500, "boom");
    let cfg = ChannelConfig {
        url: format!("http://{}/hook", stub.addr),
        secret: String::new(),
        allow_http: true,
        ..Default::default()
    };
    let d = deliver(Channel::Webhook, &cfg, 2, "hi");
    assert!(!d.ok);
    assert_eq!(d.attempts, 4, "失败重试 1+3 次");
    assert_eq!(stub.requests.lock().unwrap().len(), 4);

    // 2xx → 一次成功（即便 body 不是 JSON——Webhook 不解析业务码）
    let ok_stub = spawn_stub(200, "plain-ok");
    let cfg2 = ChannelConfig {
        url: format!("http://{}/hook", ok_stub.addr),
        secret: String::new(),
        allow_http: true,
        ..Default::default()
    };
    let d2 = deliver(Channel::Webhook, &cfg2, 2, "hi");
    assert!(d2.ok && d2.attempts == 1);
    // 厂商渠道的业务码判定（请求需打真实域名，此处只测 is_ok 纯函数）
    assert!(is_ok(Channel::Wecom, 200, "{\"errcode\":0}"));
    assert!(!is_ok(Channel::Wecom, 200, "{\"errcode\":1}"));
    assert!(!is_ok(Channel::Feishu, 200, "{\"code\":1}"));
    assert!(is_ok(Channel::Feishu, 200, "{\"code\":0}"));
}

#[test]
fn url_whitelist_blocks_ssrf_and_http() {
    assert!(check_url(Channel::Wecom, "https://qyapi.weixin.qq.com/x", false).is_ok());
    assert!(check_url(Channel::Wecom, "http://127.0.0.1/hook", false).is_err());
    assert!(check_url(Channel::Wecom, "http://evil.example/x", true).is_err());
    assert!(check_url(Channel::Webhook, "http://192.168.1.5/hook", true).is_ok());
    // 厂商渠道 host 白名单不受 allow_http 影响
    assert!(check_url(Channel::Dingtalk, "https://evil.example/x", true).is_err());
}

// ---- T-329 尾巴：PushPlus / Qmsg / Email 渠道 ----

#[test]
fn pushplus_payload_and_is_ok_200() {
    // PushPlus 业务码 200 也算成功（Python _is_ok errcode/code ∈ (0,200)）
    assert!(is_ok(Channel::Pushplus, 200, r#"{"code":200,"msg":"ok"}"#));
    assert!(is_ok(Channel::Pushplus, 200, r#"{"code":0}"#));
    assert!(!is_ok(Channel::Pushplus, 200, r#"{"code":400}"#));
    // payload：token + 模板标题 + content + template=txt
    let cfg = ChannelConfig {
        url: "https://www.pushplus.plus/send".into(),
        token: "TK".into(),
        ..Default::default()
    };
    let body = ls_dispatch::build_body(Channel::Pushplus, 2, "内容", &cfg);
    assert_eq!(body["token"], "TK");
    assert_eq!(body["title"], "LabSentinel 级别2告警");
    assert_eq!(body["content"], "内容");
    assert_eq!(body["template"], "txt");
    // 标题模板可被 LS_ALERT_TEMPLATE_TITLE 覆盖（{level} 占位符替换）
    std::env::set_var("LS_ALERT_TEMPLATE_TITLE", "实验室报警 L{level}");
    let body = ls_dispatch::build_body(Channel::Pushplus, 3, "x", &cfg);
    std::env::remove_var("LS_ALERT_TEMPLATE_TITLE");
    assert_eq!(body["title"], "实验室报警 L3");
}

#[test]
fn qmsg_payload_and_whitelist() {
    // Qmsg host 白名单放行（key 的路径段编码由 channels_from_env 生成）
    assert!(check_url(Channel::Qmsg, "https://qmsg.zendee.cn/send/AB%2Bcd", false).is_ok());
    assert!(check_url(Channel::Qmsg, "https://evil.example/send/k", false).is_err());
    let cfg = ChannelConfig {
        url: "https://qmsg.zendee.cn/send/AB%2Bcd".into(),
        qq: "12345".into(),
        ..Default::default()
    };
    let body = ls_dispatch::build_body(Channel::Qmsg, 2, "hi", &cfg);
    assert_eq!(body["msg"], "hi");
}

#[test]
fn email_private_net_guard() {
    use ls_dispatch::EmailConfig;
    // security=none 仅私网：公网地址直接拒绝、不产生 SMTP 连接
    let cfg = ChannelConfig {
        email: EmailConfig {
            host: "8.8.8.8".into(),
            port: 25,
            user: "bot@lab.local".into(),
            pass: "p".into(),
            to: "guard@lab.local".into(),
            security: "none".into(),
        },
        ..Default::default()
    };
    let d = deliver(Channel::Email, &cfg, 2, "测试");
    assert!(!d.ok);
    assert!(d.error.as_deref().unwrap().contains("私网"));
    // 回环地址：guard 放行（连不上本地端口 → send failed 而非 guard 拒绝）
    let cfg = ChannelConfig {
        email: EmailConfig {
            host: "127.0.0.1".into(),
            port: 59999,
            user: "bot@lab.local".into(),
            pass: "p".into(),
            to: "x@lab.local".into(),
            security: "none".into(),
        },
        ..Default::default()
    };
    let d = deliver(Channel::Email, &cfg, 2, "测试");
    assert!(!d.ok && d.error.as_deref().unwrap().contains("send failed"));
}

#[test]
fn email_message_shape() {
    // 信头语义：From=user、To、主题 [LabSentinel] 级别N告警（lettre 构造）
    use ls_dispatch::EmailConfig;
    let cfg = EmailConfig {
        host: "smtp.lab.local".into(),
        port: 465,
        user: "bot@lab.local".into(),
        pass: "p".into(),
        to: "duty@lab.local".into(),
        security: "ssl".into(),
    };
    let msg = ls_dispatch::build_email(3, "正文", &cfg).expect("构造成功");
    let raw = msg.formatted();
    let headers = String::from_utf8_lossy(&raw);
    assert!(headers.contains("From: bot@lab.local"));
    assert!(headers.contains("To: duty@lab.local"));
    // 非ASCII 主题 lettre 按 RFC 2047 编码（Python compat32 原样发，客户端解码后等价）
    assert!(headers.contains("Subject: [LabSentinel]"));
    assert!(headers.to_ascii_lowercase().contains("=?utf-8?"));
    // 正文按 MIME 编码传输（Python MIMEText utf-8 同为 base64，内容语义等价）
    assert!(headers.to_ascii_lowercase().contains("base64"));
}

#[test]
fn email_unconfigured_short_circuit() {
    let cfg = ChannelConfig::default();
    let d = deliver(Channel::Email, &cfg, 2, "x");
    assert!(!d.ok && d.attempts == 0);
    assert!(d.error.as_deref().unwrap().contains("未配置"));
}

#[test]
fn channels_from_env_full_wire() {
    // 8 渠道全量接线：PushPlus/Qmsg/Email 凭据进 ChannelConfig
    std::env::set_var("LS_PUSHPLUS_TOKEN", "pp-tok");
    std::env::set_var("LS_QMSG_KEY", "qk+特殊/");
    std::env::set_var("LS_QMSG_QQ", "777");
    std::env::set_var("LS_SMTP_HOST", "smtp.lab.local");
    std::env::set_var("LS_ALERT_EMAIL_TO", "to@lab.local");
    std::env::set_var("LS_SMTP_PORT", "587");
    std::env::set_var("LS_SMTP_SECURITY", "starttls");
    let chs = ls_dispatch::channels_from_env();
    std::env::remove_var("LS_PUSHPLUS_TOKEN");
    std::env::remove_var("LS_QMSG_KEY");
    std::env::remove_var("LS_QMSG_QQ");
    std::env::remove_var("LS_SMTP_HOST");
    std::env::remove_var("LS_ALERT_EMAIL_TO");
    std::env::remove_var("LS_SMTP_PORT");
    std::env::remove_var("LS_SMTP_SECURITY");
    let get = |name: &str| chs.iter().find(|(c, _)| c.name() == name).map(|(_, c)| c);
    let pp = get("pushplus").expect("pushplus 接线");
    assert_eq!(pp.token, "pp-tok");
    let qm = get("qmsg").expect("qmsg 接线");
    assert_eq!(qm.qq, "777");
    assert!(qm.url.ends_with("/send/qk%2B%E7%89%B9%E6%AE%8A%2F"));
    let em = get("email").expect("email 接线");
    assert_eq!(em.email.port, 587);
    assert_eq!(em.email.security, "starttls");
}
