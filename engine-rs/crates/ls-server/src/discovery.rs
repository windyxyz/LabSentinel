//! R2-309 批次 4：局域网摄像头发现（任务113 复刻——WS-Discovery UDP 组播 + LAN 过滤）。
//!
//! 安全边界与 Python `discovery.py` 同口径：
//! - 只发一次 UDP 组播 Probe（NetworkVideoTransmitter），只收 timeout 秒内应答；
//! - 应答解析前拒绝 DOCTYPE/ENTITY 头（XXE/实体爆炸加固）；只取 XAddrs/Scopes/name；
//! - 结果仅保留局域网地址（10/8、172.16/12、192.168/16、169.254/16）——绝不扫公网；
//! - probe 结果不含用户凭证；任何异常都返回空/错误文案，绝不 panic。

use std::net::{Ipv4Addr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

const WS_MULTICAST_ADDR: &str = "239.255.255.250";
const WS_PORT: u16 = 3702;
const CACHE_TTL: Duration = Duration::from_secs(30);

/// Probe 模板（与 Python `_PROBE_TMPL` 同形；MessageID 每次随机）
fn probe_message() -> String {
    let mut mid = [0u8; 16];
    let _ = getrandom::getrandom(&mut mid);
    let hex: String = mid.iter().map(|b| format!("{b:02x}")).collect();
    let mid_str = format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?>"#,
            r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" "#,
            r#"xmlns:a="http://schemas.xmlsoap.org/ws/2004/08/addressing">"#,
            r#"<s:Header><a:Action s:mustUnderstand="1">"#,
            r#"http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</a:Action>"#,
            r#"<a:MessageID>urn:uuid:{mid}</a:MessageID>"#,
            r#"<a:To s:mustUnderstand="1">urn:schemas-xmlsoap-org:ws:2005/04:discovery</a:To>"#,
            r#"</s:Header><s:Body><Probe xmlns="http://schemas.xmlsoap.org/ws/2005/04/discovery">"#,
            r#"<d:Types xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery" "#,
            r#"d:Namespace="http://www.onvif.org/ver10/network/wsdl">dn:NetworkVideoTransmitter</d:Types>"#,
            r#"</Probe></s:Body></s:Envelope>"#
        ),
        mid = mid_str
    )
}

/// 仅允许 10/8、172.16/12、192.168/16、169.254/16（规格白名单，回环/公网一律拒绝）。
fn is_lan(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    matches!(
        (o[0], o[1]),
        (10, _) | (172, 16..=31) | (192, 168) | (169, 254)
    )
}

/// 从 WS-Discovery 应答文本提取 (ip, xaddr, name)——标签局部名匹配（命名空间前缀各异），
/// 解析前拒绝 DOCTYPE/ENTITY；畸形输入返回空。多 ProbeMatch 设备逐条展开（调用方按 ip 去重）。
fn parse_probe_response(text: &str) -> Vec<(String, String, String)> {
    if text.is_empty() {
        return Vec::new();
    }
    let head = text.chars().take(1024).collect::<String>().to_lowercase();
    if head.contains("<!doctype") || head.contains("<!entity") {
        return Vec::new();
    }
    // 全文提取所有 XAddrs / Scopes 文本（标签局部名匹配：`:xaddrs>` 后的节点文本）
    let lower = text.to_lowercase();
    let xaddrs_all = extract_all_text(&lower, text, "xaddrs");
    let scopes_all = extract_all_text(&lower, text, "scopes");
    let name = scopes_all
        .split_whitespace()
        .find_map(|s| {
            let marker = "onvif://www.onvif.org/name/";
            let i = s.find(marker)?;
            Some(s[i + marker.len()..].split(['/', ' ']).next()?.to_string())
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for url in xaddrs_all.split_whitespace() {
        let host = host_of_url(url);
        if host.is_empty() {
            continue;
        }
        out.push((host, url.to_string(), name.clone()));
    }
    out
}

/// 在原文中查找所有 `:TagName>text<` 的节点文本（局部名匹配，大小写不敏感）。
fn extract_all_text(lower: &str, orig: &str, local_name: &str) -> String {
    let open = format!(":{local_name}>");
    let mut collected: Vec<String> = Vec::new();
    let mut i = 0usize;
    while let Some(p) = lower[i..].find(&open) {
        let abs = i + p + open.len();
        let rest = &orig[abs..];
        let end = rest.find('<').unwrap_or(rest.len());
        let text = rest[..end].trim();
        if !text.is_empty() {
            collected.push(text.to_string());
        }
        i = abs + end;
    }
    collected.join(" ")
}

/// URL host 提取（宽松实现，容忍大小写/端口/IPv6 方括号）。
fn host_of_url(url: &str) -> String {
    let rest = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => return String::new(),
    };
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_port
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_port);
    host.trim_matches(['[', ']']).to_string()
}

/// WS-Discovery 组播一次 Probe 并收集应答（按 ip 去重、只留 LAN）。
/// 任何异常（无组播路由/防火墙）都返回空列表，绝不 panic。
fn ws_discover(timeout_s: u64) -> Vec<serde_json::Value> {
    let mut found: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = sock.set_read_timeout(Some(Duration::from_secs(1)));
    if sock
        .send_to(probe_message().as_bytes(), (WS_MULTICAST_ADDR, WS_PORT))
        .is_err()
    {
        return Vec::new();
    }
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    let mut buf = [0u8; 65535];
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                for (ip, _xaddr, name) in parse_probe_response(&text) {
                    let Ok(v4) = ip.parse::<Ipv4Addr>() else {
                        continue;
                    };
                    if !is_lan(v4) {
                        continue;
                    }
                    found.entry(ip.clone()).or_insert_with(|| {
                        serde_json::json!({"ip": ip, "name": name,
                            "url": format!("rtsp://{ip}:554/")})
                    });
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        }
    }
    found.into_values().collect()
}

/// 30s 进程缓存（与 Python get_devices 同语义；refresh 强制重发）。
pub fn get_devices(timeout_s: u64, refresh: bool) -> Vec<serde_json::Value> {
    static CACHE: std::sync::Mutex<Option<(Instant, Vec<serde_json::Value>)>> =
        std::sync::Mutex::new(None);
    let mut guard = match CACHE.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    if !refresh {
        if let Some((ts, devs)) = guard.as_ref() {
            if !devs.is_empty() && ts.elapsed() < CACHE_TTL {
                return devs.clone();
            }
        }
    }
    let devs = ws_discover(timeout_s);
    *guard = Some((Instant::now(), devs.clone()));
    devs
}

/// 本机局域网 IPv4（std 无接口枚举：UDP connect 公网地址取默认路由 IP）。
/// 与 Python `lan_addresses` 不同——只返回默认路由 IP 一个（展示辅助，如实标注）。
pub fn lan_addresses() -> Vec<String> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    // connect 不发包，只决定路由
    sock.connect("8.8.8.8:80").ok();
    if let Ok(local) = sock.local_addr() {
        let ip = local.ip();
        match ip {
            std::net::IpAddr::V4(v4) => {
                if v4.is_unspecified() || !is_lan(v4) {
                    return Vec::new();
                }
                return vec![v4.to_string()];
            }
            _ => return Vec::new(),
        }
    }
    Vec::new()
}

/// TCP/ffprobe 只读探测（凭证仅用于本次 URL，不写日志/不回传）。
pub fn probe_host(
    host: &str,
    port: u16,
    username: Option<&str>,
    password: Option<&str>,
) -> serde_json::Value {
    let mut out = serde_json::json!({"host": host, "port": port, "ip": null,
        "reachable": false, "port_open": false, "ffprobe": null, "error": ""});
    // 解析：std DNS（host:port → IP 列表）
    let addrs = match (host, port).to_socket_addrs() {
        Ok(a) => a
            .filter_map(|a| match a.ip() {
                std::net::IpAddr::V4(v4) => Some(v4),
                _ => None,
            })
            .collect::<Vec<_>>(),
        Err(_) => {
            out["error"] = "主机名无法解析".into();
            return out;
        }
    };
    let Some(ip) = addrs.first().copied() else {
        out["error"] = "主机名无法解析".into();
        return out;
    };
    out["ip"] = ip.to_string().into();
    if !is_lan(ip) {
        out["error"] = "仅允许探测局域网地址（10/172.16/192.168/169.254）".into();
        return out;
    }
    match std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from((ip, port)),
        Duration::from_secs(1),
    ) {
        Ok(_) => {
            out["reachable"] = serde_json::json!(true);
            out["port_open"] = serde_json::json!(true);
        }
        Err(e) => {
            out["error"] = serde_json::json!(format!("端口 {port} 连接失败：超时或被拒绝"));
            let _ = e;
            return out;
        }
    }
    // ffprobe 增强：凭证只用于本次 URL；错误信息不得包含 url
    let ff = ls_core::bin::find("ffprobe");
    let url = if let Some(u) = username {
        let q = |s: &str| {
            s.replace('%', "%25")
                .replace(':', "%3A")
                .replace('/', "%2F")
        };
        format!("rtsp://{}:{}@{ip}:{port}/", q(u), q(password.unwrap_or("")))
    } else {
        format!("rtsp://{ip}:{port}/")
    };
    let Ok(mut child) = std::process::Command::new(&ff)
        .args([
            "-v",
            "error",
            "-rtsp_transport",
            "tcp",
            "-show_entries",
            "stream=codec_name,width,height,avg_frame_rate",
            "-of",
            "json",
            &url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return out;
    };
    let deadline = Duration::from_secs(6);
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => return out,
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > deadline => {
                let _ = child.kill();
                return out;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return out,
        }
    }
    let Ok(out_bytes) = child.wait_with_output() else {
        return out;
    };
    let Ok(j) = serde_json::from_slice::<serde_json::Value>(&out_bytes.stdout) else {
        return out;
    };
    let Some(streams) = j.get("streams").and_then(|v| v.as_array()) else {
        return out;
    };
    for s in streams {
        let codec = s.get("codec_name").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(codec, "h264" | "hevc" | "h265" | "mjpeg") {
            out["ffprobe"] = serde_json::json!({
                "codec": codec,
                "width": s.get("width").cloned().unwrap_or(serde_json::json!(0)),
                "height": s.get("height").cloned().unwrap_or(serde_json::json!(0)),
                "fps": eval_fraction(s.get("avg_frame_rate").and_then(|v| v.as_str()).unwrap_or("")),
            });
            break;
        }
    }
    out
}

/// "num/den" → f32（Python eval_fraction 同口径；0/非法 → 0.0）
fn eval_fraction(s: &str) -> f64 {
    let mut it = s.split('/');
    let num: f64 = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0.0);
    let den: f64 = it.next().and_then(|v| v.trim().parse().ok()).unwrap_or(1.0);
    if den == 0.0 || num == 0.0 {
        return 0.0;
    }
    (num / den * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_filter() {
        assert!(is_lan("192.168.1.10".parse().unwrap()));
        assert!(is_lan("10.0.0.5".parse().unwrap()));
        assert!(is_lan("172.16.3.4".parse().unwrap()));
        assert!(is_lan("172.31.255.1".parse().unwrap()));
        assert!(is_lan("169.254.9.9".parse().unwrap()));
        assert!(!is_lan("127.0.0.1".parse().unwrap()));
        assert!(!is_lan("8.8.8.8".parse().unwrap()));
        assert!(!is_lan("172.32.0.1".parse().unwrap()));
    }

    #[test]
    fn response_parse_rejects_xxe() {
        assert!(parse_probe_response("<!DOCTYPE foo><ProbeMatch>").is_empty());
        assert!(parse_probe_response("").is_empty());
        let good = r#"<s:Envelope><s:Body><d:ProbeMatch>
            <d:XAddrs>http://192.168.1.64:8899/onvif/device_service</d:XAddrs>
            <d:Scopes>onvif://www.onvif.org/name/CamA onvif://www.onvif.org/type/video_encoder</d:Scopes>
            </d:ProbeMatch></s:Body></s:Envelope>"#;
        let rows = parse_probe_response(good);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "192.168.1.64");
        assert_eq!(rows[0].2, "CamA");
    }

    #[test]
    fn fraction() {
        assert!((eval_fraction("25/5") - 5.0).abs() < 1e-9);
        assert_eq!(eval_fraction("0/5"), 0.0);
        assert_eq!(eval_fraction("x/y"), 0.0);
    }
}
