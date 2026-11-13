//! ls-server（L3/T-331）：axum 控制面单二进制。
//!
//! 契约 1:1 对齐 Python FastAPI（docs/openapi.json 快照）：
//! - GET /api/health/live、/api/health/ready、/api/system、/api/events、
//!   /api/cameras、/api/cameras/{id}/status、/api/stats/summary、/api/config/summary
//! - POST /api/events/{id}/status（处置）、/api/cameras（新增）、/api/backups/run
//! - WS /ws/events（tokio broadcast，慢客户端丢弃策略）
//!
//! 中间件：request-id、Host 白名单、CSRF(Origin/Sec-Fetch-Site)、管理令牌、安全响应头。
//! 静态：web/dist（SPA 回退 + 缓存策略）。数据库：ls-store（与 Python 同 SQLite/双向兼容）。

use std::sync::Arc;

mod discovery;
mod fernet;
mod pdf;

use axum::routing::{delete, get, get_service, post, put};
use axum::Router;
use tokio::sync::broadcast;
use tower_http::services::{ServeDir, ServeFile};

// ---- 2026-11-12 batch A: domain split (mechanical move, zero logic change) ----
// 原 middleware::{self,..} 导入已改 `middleware::Next`（根 `mod middleware` 让名）。

// ---- 2026-11-12 批次 A：按功能域拆分（结构化重构，机械搬移零逻辑改动）----
// 原 middleware::{self,..} 导入已改 `middleware::Next`（根 `mod middleware` 让名）。
mod face;
mod genai;
mod handlers;
mod middleware;
mod plugins;
mod router;
mod state;
mod util;
mod worker;

mod url {
    /// 从 URL 提取 host（标准库解析；与 Python urlparse(url).hostname 等价的宽松实现）
    pub(crate) fn host_of(url: &str) -> Option<String> {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))?;
        let host_port = rest.split(['/', '?', '#']).next()?;
        let host = host_port
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_port);
        if host.is_empty() || host.contains('@') {
            return None;
        }
        Some(host.to_string())
    }
}

pub(crate) use handlers::*;
pub(crate) use middleware::{guard_middleware, static_cache_policy};
pub use router::{build_router, dump_routes, ROUTE_INVENTORY};
pub(crate) use state::{AppState, PairState, Shared};

#[tokio::main]
async fn main() {
    // 健康自检（原为容器 HEALTHCHECK 设计，Docker 路线已终止——保留作通用探活）：
    // `labsentinel-server --healthcheck <db>` 只读探活即退出
    let mut args = std::env::args().skip(1);
    if let Some(a) = args.next() {
        if a == "--healthcheck" {
            let db = args
                .next()
                .unwrap_or_else(|| "server/data/labsentinel.db".into());
            let ok = ls_store::Store::open(&db)
                .ok()
                .and_then(|s| s.integrity_check().ok())
                .is_some_and(|v| v == "ok");
            std::process::exit(if ok { 0 } else { 1 });
        }
        // T-332：跨语言 DB 兼容驱动——对指定库写一条事件/投递并读回（exit 0=成功）。
        // 由 server/tests/test_store_bidirectional.py 调用，实现三路径兼容验收。
        if a == "--selftest-db" {
            let db = args.next().expect("--selftest-db <path>");
            let code = ls_store::selftest::run(&db);
            std::process::exit(code);
        }
        // R2-307: print the route inventory JSON and exit (coverage script).
        if a == "--dump-routes" {
            println!(
                "{}",
                serde_json::to_string_pretty(&dump_routes()).expect("json")
            );
            std::process::exit(0);
        }
    }
    let db_path =
        std::env::var("LS_DB_PATH").unwrap_or_else(|_| "server/data/labsentinel.db".into());
    let token = std::env::var("LS_ADMIN_TOKEN").unwrap_or_default();
    let read_token = std::env::var("LS_READ_TOKEN").unwrap_or_default();
    let read_protected = std::env::var("LS_AUTH_READ_PROTECTED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let data_dir = std::env::var("LS_DATA_DIR").unwrap_or_else(|_| "server/data".into());
    let version = std::env::var("LS_VERSION").unwrap_or_else(|_| "0.2.0".into());
    let port: u16 = std::env::var("LS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8000);
    let jobs = Arc::new(ls_jobs::Jobs::new(
        std::path::PathBuf::from(&db_path),
        ls_jobs::JobsConfig::from_env(std::path::Path::new(&db_path)),
    ));
    let st = Arc::new(AppState {
        db_path,
        admin_token: token,
        read_token,
        read_protected,
        trusted_hosts: vec!["127.0.0.1".into(), "localhost".into(), "::1".into()],
        allowed_origins: vec![],
        // 2026-11-11：与 Python 同名环境变量（LS_WEB_DIST），未设时按仓库相对布局。
        web_dist: std::env::var("LS_WEB_DIST").unwrap_or_else(|_| "web/dist".into()),
        data_dir,
        version,
        events_tx: broadcast::channel(256).0,
        jobs,
        pair_state: std::sync::Mutex::new(PairState::default()),
        testing: std::env::var("LS_TESTING")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        workers: worker::Workers::default(),
        // RA2-S2：MQTT 事件桥（默认关零行为变化；连接失败不阻塞事件链——库保证）
        mqtt: ls_jobs::mqtt_from_env().map(Arc::new),
        plugins: plugins::PluginManager::default(),
        genai: genai::GenAI::from_env(),
    });
    // R2-305：网页端配置的渠道（app_settings alert.*，Fernet 解密）注入 ls-jobs
    // escalation 发送——"网页端保存企微 webhook → rust 路径生效"的真实验收点。
    // 解密失败的渠道（Windows DPAPI 密文）跳过（如实降级）；无 DB 覆盖回退 env。
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        {
            let rows = s.settings_by_prefix("alert.").unwrap_or_default();
            if !rows.is_empty() {
                let get = |field: &str| -> String {
                    for (k, v) in &rows {
                        if *k == format!("alert.{field}") {
                            let f = &k["alert.".len()..];
                            if alert_cfg::is_secret(f) {
                                return fernet::unprotect(v, std::path::Path::new(&st.data_dir));
                            }
                            return v.clone();
                        }
                    }
                    String::new()
                };
                let mut channels: Vec<(ls_dispatch::Channel, ls_dispatch::ChannelConfig)> =
                    Vec::new();
                let push = |name: &str,
                            ch: ls_dispatch::Channel,
                            cfg: ls_dispatch::ChannelConfig,
                            configured: bool,
                            out: &mut Vec<_>| {
                    if configured {
                        out.push((ch, cfg));
                    } else {
                        let _ = name;
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
                st.jobs.set_alert_channels(channels);
            }
        }
    }
    let app = build_router(st.clone());
    st.jobs.start();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    println!("labsentinel-server READY port={port}");
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests;
