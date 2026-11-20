// UX3（2026-11-20）：release 双击启动无控制台（GUI 子系统）；debug 保留控制台。
// 管道语义已实测（UX3.0 spike）：父进程管道重定向时 stdout/exit code 照常——
// CI 冒烟 / smoke_rust / 安装器 --selftest-http 不受影响；排障走 --console。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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
mod vlm;
mod worker;

// UX2（2026-11-20）：检测模式全局策略（local/api/cascade）——detect.* 三条 API
// + cascade 复核引擎的数据面
mod detect;

pub(crate) use detect::{detect_config_get, detect_config_put, detect_test, detect_usage};

// RA4.2（ADR-0011）：版本归档/回滚（install_portable 语义的 Rust 实现）
mod deploy;

// UX3/UX4（2026-11-20）：无黑框启动三件套——控制台/弹窗 FFI（唯二 unsafe 收口模块
// 之一）、日志文件化（data/logs 按天滚动 14 天）、系统托盘（唯一常驻 UI）
mod logger;
mod tray;
mod wincon;

pub(crate) use tray::graceful_shutdown;

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
    // UX3：--console 必须早于任何 println!（GUI release 下附着父控制台重指
    // CONOUT$；CI 管道/安装器自检已有控制台，幂等直接返回）
    if std::env::args().any(|x| x == "--console") {
        crate::wincon::attach_parent_console();
    }
    // 健康自检（原为容器 HEALTHCHECK 设计，Docker 路线已终止——保留作通用探活）：
    // `labsentinel-server --healthcheck <db>` 只读探活即退出
    let launch_mode = std::env::args().any(|x| x == "--launch");
    let tray_mode = std::env::args().any(|x| x == "--tray");
    if launch_mode {
        // RA4（ADR-0011）：portable_launcher 消解件——端口试探防双实例：
        // 目标端口已被监听 = 已有实例在跑 → 开浏览器即退（锁文件照写对齐 Python 语义）
        let port: u16 = std::env::var("LS_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8000);
        if tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_err()
        {
            println!("[launch] 端口 {port} 已被监听——已有 LabSentinel 实例，直接打开页面");
            open_browser(port);
            std::process::exit(0);
        }
    }
    let mut args = std::env::args().skip(1);
    if let Some(a) = args.next() {
        if a == "--healthcheck" {
            let db = args.next().unwrap_or_else(|| "data/labsentinel.db".into());
            let ok = ls_store::Store::open(&db)
                .ok()
                .and_then(|s| s.integrity_check().ok())
                .is_some_and(|v| v == "ok");
            std::process::exit(if ok { 0 } else { 1 });
        }
        // T-332：DB 自检驱动——对指定库写一条事件/投递并读回（exit 0=成功）。
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
        // RA4（ADR-0011）：health_check.py 消解件——真起服务于 127.0.0.1:0，
        // 轮询 /api/health/ready ≤60s（Inno finalize/升级后自检调用；exit 0=通过）。
        if a == "--selftest-http" {
            let ok = selftest_http().await;
            std::process::exit(if ok { 0 } else { 1 });
        }
        // RA4.2（ADR-0011）：版本归档/回滚——install_portable.py finalize --archive / rollback 同语义，
        // 安装根取位置参数 → LS_INSTALL_DIR → 可执行文件推断。exit 0=成功 / 2=失败（不阻断安装）。
        if a == "--archive" || a == "--rollback" {
            let install = args
                .next()
                .filter(|s| !s.starts_with("--"))
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::var("LS_INSTALL_DIR")
                        .ok()
                        .filter(|s| !s.trim().is_empty())
                        .map(std::path::PathBuf::from)
                })
                .unwrap_or_else(deploy::default_install_root);
            let action = a.trim_start_matches("--");
            let res = if a == "--archive" {
                deploy::archive(&install)
            } else {
                deploy::rollback(&install)
            };
            match res {
                Ok(p) => {
                    println!("[{action}] 完成 → {}", p.display());
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("[{action}] 失败：{e}");
                    std::process::exit(2);
                }
            }
        }
    }
    // RA4.2：数据目录默认相对当前目录的 data/；DB 未显式指定时跟随数据目录
    let data_dir = std::env::var("LS_DATA_DIR").unwrap_or_else(|_| "data".into());
    let db_path = std::env::var("LS_DB_PATH").unwrap_or_else(|_| {
        std::path::Path::new(&data_dir)
            .join("labsentinel.db")
            .to_string_lossy()
            .into_owned()
    });
    let token = std::env::var("LS_ADMIN_TOKEN").unwrap_or_default();
    let read_token = std::env::var("LS_READ_TOKEN").unwrap_or_default();
    let read_protected = std::env::var("LS_AUTH_READ_PROTECTED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let data_dir = std::env::var("LS_DATA_DIR").unwrap_or_else(|_| "server/data".into());
    // RA4.2：版本号优先 LS_VERSION，其次包内 VERSION/app/VERSION（发行包自带），最后兜底
    let version = std::env::var("LS_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| deploy::read_version(&deploy::default_install_root()));
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
    // UX3：日志文件化 + panic hook（无黑框 ≠ 黑盒；data/logs 按天滚动 14 天）
    let log_file = crate::logger::init(std::path::Path::new(&st.data_dir));
    crate::logger::install_panic_hook();
    crate::logger::info(&format!(
        "[start] LabSentinel {} · http://127.0.0.1:{port}/ · 日志 {:?}",
        st.version,
        log_file.unwrap_or_else(|| "<不可写，仅控制台>".into())
    ));
    // RA5：独立部署自举——全新库（零 Python 部署）由 Rust 建全量 schema（幂等；
    // 已有库走 migrate 增量列迁移——Python 退役后迁移链归 Rust）
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        match s.ensure_schema() {
            Ok(true) => println!(
                "[ls-server] 全新库——已自举 v{} schema（Rust 形态独立部署）",
                ls_store::SCHEMA_VERSION_LATEST
            ),
            Ok(false) => {
                // UX2：存量库 v13→v14（events cascade 判定列；幂等可重复执行）
                match s.migrate() {
                    Ok(v) if v >= 14 => {}
                    Ok(_) => {}
                    Err(e) => eprintln!("[ls-server] schema 迁移失败: {e}"),
                }
            }
            Err(e) => eprintln!("[ls-server] schema 自举失败: {e}"),
        }
    }
    let app = build_router(st.clone());
    st.jobs.start();
    // RA2 反思修正：launch 模式下端口试探与 bind 之间存在 TOCTOU 窗口——
    // bind 失败不再 unwrap panic（会崩进程），优雅提示后退出
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            crate::logger::error(&format!(
                "[ls-server] 端口 {port} 绑定失败: {e}——可能已有实例在运行"
            ));
            // UX3：致命启动错误必须可见（GUI 无控制台 → 原生弹窗）
            crate::wincon::fatal_error_box(
                "LabSentinel 启动失败",
                &format!(
                    "端口 {port} 被占用（可能已有 LabSentinel 在运行）。
诊断：
- 日志：数据目录 logs/server-*.log
- 命令行排障：labsentinel-server.exe --console --launch"
                ),
            );
            if launch_mode {
                open_browser(port);
                std::process::exit(0);
            }
            std::process::exit(1);
        }
    };
    println!("labsentinel-server READY port={port}");
    // UX4：系统托盘（唯一常驻 UI；--tray 开启，安装器快捷方式 = --launch --tray）。
    // 创建失败只 warn 降级，不影响服务。
    if tray_mode {
        crate::tray::start(Arc::clone(&st), port);
    }
    // RA4：--launch 模式（portable_launcher 消解件）——单实例锁（对齐 Python
    // "pid port" 契约，位置 data 上一级 logs/）+ 就绪后开浏览器
    if launch_mode {
        let logs = std::path::Path::new(&st.data_dir)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("logs");
        let _ = std::fs::create_dir_all(&logs);
        let _ = std::fs::write(
            logs.join("launcher.lock"),
            format!("{} {port}", std::process::id()),
        );
        open_browser(port);
    }
    axum::serve(listener, app).await.unwrap();
}

/// RA4：默认浏览器打开服务页（Windows `cmd /C start`；其他平台 xdg-open/open）。
fn open_browser(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .creation_flags(0x0800_0000)
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
}

/// RA4：health_check.py 消解件——真起服务于 127.0.0.1:0（隔离临时数据目录），
/// 轮询 /api/health/ready ≤60s。退出码由调用方转进程码（0=通过）。
async fn selftest_http() -> bool {
    let dir = std::env::temp_dir().join(format!("ls-selftest-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let db = dir.join("selftest.db");
    let st = Arc::new(AppState {
        db_path: db.to_string_lossy().into_owned(),
        admin_token: String::new(),
        read_token: String::new(),
        read_protected: false,
        trusted_hosts: vec!["127.0.0.1".into(), "localhost".into()],
        allowed_origins: vec![],
        web_dist: String::new(), // 不挂载静态（自检只验 API/DB 链路）
        data_dir: dir.join("data").to_string_lossy().into_owned(),
        version: std::env::var("LS_VERSION").unwrap_or_else(|_| "0.0.0".into()),
        events_tx: broadcast::channel(16).0,
        jobs: Arc::new(ls_jobs::Jobs::new(
            db.clone(),
            ls_jobs::JobsConfig::default_for_db(&db),
        )),
        pair_state: std::sync::Mutex::new(PairState::default()),
        testing: false,
        workers: worker::Workers::default(),
        mqtt: None,
        plugins: plugins::PluginManager::default(),
        genai: genai::GenAI::from_env(),
    });
    // 不 start() 后台作业：自检只验 API/DB 链路（顺带消除空 DB 的 backup 噪音）
    let app = build_router(st.clone());
    let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await else {
        return false;
    };
    let addr = match listener.local_addr() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(2)))
        .build()
        .into();
    let mut ok = false;
    for _ in 0..60 {
        let url = format!("{base}/api/health/ready");
        if let Ok(r) = agent.get(&url).call() {
            if r.status().as_u16() == 200 {
                ok = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
    println!("[selftest-http] ready={ok}");
    ok
}

#[cfg(test)]
mod tests;
