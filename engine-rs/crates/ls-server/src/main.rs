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

mod access;
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
mod envfile;
mod face;
mod genai;
mod go2rtc;
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

pub(crate) use detect::{
    detect_config_get, detect_config_put, detect_models, detect_profile_activate,
    detect_profile_delete, detect_profile_upsert, detect_profiles_get, detect_test, detect_usage,
};

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

/// LS-045：启动前读取 `.env`（安装根优先，其次当前目录），仅填充未设置的环境变量
/// （绝不覆盖已有的系统/命令行环境变量）。
fn load_dotenv(install_root: &std::path::Path) {
    let mut candidates = vec![install_root.join(".env")];
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join(".env");
        if p != candidates[0] {
            candidates.push(p);
        }
    }
    for path in candidates {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for raw in content.lines() {
            let line = raw.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let key = k.trim();
            if key.is_empty() || std::env::var_os(key).is_some() {
                continue;
            }
            let mut val = v.trim();
            if val.len() >= 2
                && ((val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\'')))
            {
                val = &val[1..val.len() - 1];
            }
            std::env::set_var(key, val);
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "" | "127.0.0.1" | "localhost" | "::1" | "[::1]"
    )
}

#[tokio::main]
async fn main() {
    // UX3：--console 必须早于任何 println!（GUI release 下附着父控制台重指
    // CONOUT$；CI 管道/安装器自检已有控制台，幂等直接返回）
    if std::env::args().any(|x| x == "--console") {
        crate::wincon::attach_parent_console();
    }
    let install_root = deploy::default_install_root();
    let explicit_network = envfile::NETWORK_KEYS.map(|key| std::env::var_os(key).is_some());
    load_dotenv(&install_root);
    let dist_layout = install_root.join("web").join("index.html").is_file();
    let env_path = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
    let data_dir = env_path("LS_DATA_DIR").unwrap_or_else(|| {
        if dist_layout {
            deploy::default_data_dir(&install_root)
        } else {
            "data".into()
        }
    });
    let db_path = env_path("LS_DB_PATH").unwrap_or_else(|| {
        std::path::Path::new(&data_dir)
            .join("labsentinel.db")
            .to_string_lossy()
            .into_owned()
    });
    if let Ok(content) = std::fs::read_to_string(envfile::network_path(&data_dir)) {
        for (key, explicit) in envfile::NETWORK_KEYS.into_iter().zip(explicit_network) {
            if !explicit {
                if let Some(value) = envfile::get_key(&content, key) {
                    std::env::set_var(key, value);
                }
            }
        }
    }
    let wait_port = std::env::args()
        .skip_while(|arg| arg != "--wait-for-old-port")
        .nth(1)
        .and_then(|port| port.parse::<u16>().ok());
    if let Some(port) = wait_port {
        let wait_host = std::env::var("LS_HOST")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "127.0.0.1".into());
        let ready = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            // Windows 上 0.0.0.0 仍被占用时 bind(127.0.0.1) 可能误成功，故先探测旧端口仍可连，
            // 并确认实际监听地址 (wait_host, port) 已完全释放。
            loop {
                let still_alive = tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .is_ok();
                if !still_alive
                    && tokio::net::TcpListener::bind((wait_host.as_str(), port))
                        .await
                        .is_ok()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        })
        .await;
        if ready.is_err() {
            eprintln!("[restart] 旧进程未在 30 秒内释放端口 {port}");
            std::process::exit(1);
        }
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
    let mut args = std::env::args().skip(1).filter(|arg| arg != "--console");
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
        if a == "--access-recovery-code" {
            match access::recovery_code(&db_path) {
                Ok(code) => println!("本机访问恢复码（十分钟内有效）\n{code}"),
                Err(err) => {
                    eprintln!("[access] {err}");
                    std::process::exit(2);
                }
            }
            return;
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
    // RA4.2 / 0.3.0 修复：发行形态（安装版/便携包）的默认路径
    // - 数据目录：LS_DATA_DIR 优先；发行形态 Windows 落 %LOCALAPPDATA%\LabSentinel\data
    //   （安装到 Program Files 时安装目录不可写，此前 data/ 相对路径导致 DB 打不开、
    //   /api/health/ready=503）；源码开发形态沿用 data/（仓库相对）。
    // - 前端目录：LS_WEB_DIST 优先；发行形态用 <安装根>/web（包内布局），
    //   源码开发用 web/dist——此前安装版根路径 404 即因默认 web/dist 不存在。
    let token = std::env::var("LS_ADMIN_TOKEN").unwrap_or_default();
    let read_token = std::env::var("LS_READ_TOKEN").unwrap_or_default();
    let read_protected = std::env::var("LS_AUTH_READ_PROTECTED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    // RA4.2：版本号优先 LS_VERSION，其次包内 VERSION/app/VERSION（发行包自带），最后兜底
    let version = env_path("LS_VERSION").unwrap_or_else(|| deploy::read_version(&install_root));
    let web_dist = env_path("LS_WEB_DIST").unwrap_or_else(|| {
        if dist_layout {
            install_root.join("web").to_string_lossy().into_owned()
        } else {
            "web/dist".into()
        }
    });
    let host = std::env::var("LS_HOST")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = std::env::var("LS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8000);
    // LS-045：非回环监听（如 LS_HOST=0.0.0.0）强制要求设置 LS_ADMIN_TOKEN，拒绝无令牌暴露局域网
    if !is_loopback_host(&host) && token.trim().is_empty() {
        let msg = format!(
            "拒绝启动：LS_HOST={host} 为非回环地址，但未配置 LS_ADMIN_TOKEN。\n\
             局域网/远程暴露必须设置足够长的管理令牌（LS_ADMIN_TOKEN），可在 .env 或环境变量中配置。"
        );
        eprintln!("[ls-server] {msg}");
        crate::wincon::fatal_error_box("LabSentinel 安全拦截", &msg);
        std::process::exit(2);
    }
    let mut trusted_hosts = vec!["127.0.0.1".into(), "localhost".into(), "::1".into()];
    if !is_loopback_host(&host) && host != "0.0.0.0" && host != "::" {
        trusted_hosts.push(host.clone());
    }
    if let Ok(extra) = std::env::var("LS_TRUSTED_HOSTS") {
        for item in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if !trusted_hosts.iter().any(|h| h.eq_ignore_ascii_case(item)) {
                trusted_hosts.push(item.to_string());
            }
        }
    }
    let jobs = Arc::new(ls_jobs::Jobs::new(
        std::path::PathBuf::from(&db_path),
        ls_jobs::JobsConfig::from_env(std::path::Path::new(&db_path)),
    ));
    let st = Arc::new(AppState {
        db_path,
        admin_token: token,
        read_token,
        read_protected,
        access: access::AccessControl::new(
            std::env::var("LS_TRUST_PROXY").is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes")),
        ),
        trusted_hosts,
        allowed_origins: vec![],
        // 2026-11-21：web_dist 已按发行/源码布局解析（LS_WEB_DIST 仍可覆盖）
        web_dist,
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
        go2rtc: Arc::new(go2rtc::Go2rtcSupervisor::new(true)),
        detections_tx: broadcast::channel(64).0,
    });
    // R2-305：网页端配置的渠道（app_settings alert.*，Fernet 解密）注入 ls-jobs
    // escalation 发送——"网页端保存企微 webhook → rust 路径生效"的真实验收点。
    // 解密失败的渠道（Windows DPAPI 密文）跳过（如实降级）；无 DB 覆盖回退 env。
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        // UX（2026-11-21）：快照构造抽为 channels_from_settings——启动注入与
        // 「网页保存后即时重建」共用（保存/重置/测试/派发四处同源）
        let channels =
            crate::handlers::alerts::channels_from_settings(&s, std::path::Path::new(&st.data_dir));
        if !channels.is_empty() {
            println!(
                "[ls-server] alert channels injected from app_settings: {} 渠道",
                channels.len()
            );
            st.jobs.set_alert_channels(channels);
        }
    }
    // UX3：日志文件化 + panic hook（无黑框 ≠ 黑盒；data/logs 按天滚动 14 天）
    let log_file = crate::logger::init(std::path::Path::new(&st.data_dir));
    crate::logger::install_panic_hook();
    crate::logger::info(&format!(
        "[start] LabSentinel {} · http://{host}:{port}/ · 日志 {:?}",
        st.version,
        log_file.unwrap_or_else(|| "<不可写，仅控制台>".into())
    ));
    // RA5：独立部署自举——全新库（零 Python 部署）由 Rust 建全量 schema（幂等；
    // 已有库走 migrate 增量列迁移——Python 退役后迁移链归 Rust）
    if let Ok(s) = ls_store::Store::open(&st.db_path) {
        // UX（2026-11-21）：网页端保存的告警文案（alert.template.text）注入
        // 运行时 override——升级提醒/预览渲染即时生效、跨重启持久
        if let Ok(Some(t)) = s.setting_get("alert.template.text") {
            if !t.trim().is_empty() {
                st.jobs.set_alert_template(t);
                println!("[ls-server] 告警文案已从设置载入（网页端自定义）");
            }
        }
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
    st.access.load(&st.db_path);
    let app = build_router(st.clone());
    st.jobs.start();
    // LS-051：按 app_settings 决定是否托管 go2rtc（缺失二进制优雅降级，不阻塞启动）
    let go2rtc_enabled = ls_store::Store::open(&st.db_path)
        .ok()
        .and_then(|s| s.setting_get("go2rtc.enabled").ok().flatten())
        .map(|v| !matches!(v.trim(), "0" | "false" | "off" | "no"))
        .unwrap_or(true);
    st.go2rtc.set_enabled(go2rtc_enabled);
    // RA2 反思修正：launch 模式下端口试探与 bind 之间存在 TOCTOU 窗口——
    // bind 失败不再 unwrap panic（会崩进程），优雅提示后退出
    let listener = match tokio::net::TcpListener::bind((host.as_str(), port)).await {
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
    st.go2rtc.start(st.db_path.clone(), install_root);
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
        // 0.3.0 修复：等 axum::serve 真正开始 accept 再开浏览器（此前在 serve 前
        // 调用；慢启动/冷迁移时浏览器先到 → ERR_CONNECTION_REFUSED）。
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            open_browser(port);
        });
    }
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
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
        access: access::AccessControl::default(),
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
        go2rtc: Arc::new(go2rtc::Go2rtcSupervisor::new(true)),
        detections_tx: broadcast::channel(64).0,
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
