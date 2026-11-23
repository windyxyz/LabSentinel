//! LS-051：Rust 进程内托管 go2rtc（WebRTC 流媒体网关）。
//!
//! - 检测 `<install_root>/tools/go2rtc.exe`（或 `LS_GO2RTC` 显式路径），按需拉起；
//! - 子进程走 `ls_ingest::child_guard::ChildGuard`（CREATE_NO_WINDOW + Job Object，
//!   主进程死亡无孤儿）；
//! - 通过 go2rtc HTTP API（默认 127.0.0.1:1984）注册/注销 RTSP 流；
//! - 开关配置存 app_settings `go2rtc.enabled`（默认开，缺失二进制优雅降级）；
//! - 状态供 `/api/system` 与 `GET /api/go2rtc/status` 读取。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ls_ingest::child_guard::ChildGuard;

/// go2rtc API 默认地址（与 handlers/cameras.rs::go2rtc_reachable 同口径）。
const DEFAULT_PORT: u16 = 1984;
const DEFAULT_RTSP_LISTEN: &str = "127.0.0.1:8554";

pub struct Go2rtcSupervisor {
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
    external: Option<String>,
    address: SocketAddr,
    rtsp_listen: String,
}

struct Inner {
    child: Option<ChildGuard>,
    last_attempt: Option<Instant>,
    last_error: String,
    enabled: bool,
    stopped: bool,
    registered: HashMap<i64, String>,
}

impl Go2rtcSupervisor {
    pub fn new(enabled: bool) -> Self {
        let port = std::env::var("LS_GO2RTC_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .filter(|&port| port > 0)
            .unwrap_or(DEFAULT_PORT);
        Self {
            inner: Mutex::new(Inner {
                child: None,
                last_attempt: None,
                last_error: String::new(),
                enabled,
                stopped: false,
                registered: HashMap::new(),
            }),
            wake: tokio::sync::Notify::new(),
            external: std::env::var("LS_GO2RTC_ADDR")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            address: SocketAddr::from(([127, 0, 0, 1], port)),
            rtsp_listen: std::env::var("LS_GO2RTC_RTSP_LISTEN")
                .unwrap_or_else(|_| DEFAULT_RTSP_LISTEN.into()),
        }
    }

    /// 查找 go2rtc 可执行文件：`LS_GO2RTC` > `<install>/tools/go2rtc.exe`。
    pub fn resolve_bin(install_root: &Path) -> Option<PathBuf> {
        if let Some(p) = std::env::var("LS_GO2RTC")
            .ok()
            .filter(|s| !s.trim().is_empty())
        {
            let p = PathBuf::from(p);
            if p.is_file() {
                return Some(p);
            }
        }
        let p = install_root.join("tools").join("go2rtc.exe");
        p.is_file().then_some(p)
    }

    /// 尝试确保 go2rtc 运行（幂等），启动失败时限频重试。
    /// 返回是否处于运行状态。二进制缺失 / 启动失败都优雅降级（仅记录 last_error）。
    pub fn ensure_running(&self, install_root: &Path, api_base: &str) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        if !inner.enabled || inner.stopped {
            return false;
        }
        if let Some(c) = inner.child.as_mut() {
            if c.is_running() {
                return true;
            }
            inner.child = None; // 进程已退出，准备重启
            inner.registered.clear();
        }
        if inner
            .last_attempt
            .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
        {
            return false;
        }
        inner.last_attempt = Some(Instant::now());
        let Some(bin) = Self::resolve_bin(install_root).and_then(|p| p.canonicalize().ok()) else {
            inner.last_error = "未找到 go2rtc.exe（tools/ 或 LS_GO2RTC）".into();
            return false;
        };
        // 使用内联配置，避免将注册的相机源写入安装目录。
        let api_host = api_base
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or("127.0.0.1:1984");
        if api_host.parse::<SocketAddr>().is_ok_and(api_reachable) {
            inner.last_error = "go2rtc 本地端口已被占用".into();
            return false;
        }
        let mut cmd = std::process::Command::new(&bin);
        let config = serde_json::json!({
            "api": {"listen": api_host},
            "rtsp": {"listen": self.rtsp_listen},
            "webrtc": {"listen": std::env::var("LS_GO2RTC_WEBRTC_LISTEN").unwrap_or_else(|_| ":8555".into())}
        });
        cmd.arg("-c").arg(config.to_string());
        cmd.current_dir(bin.parent().unwrap_or(install_root));
        match ChildGuard::spawn(cmd) {
            Ok(g) => {
                inner.last_error.clear();
                inner.child = Some(g);
                true
            }
            Err(e) => {
                inner.last_error = e;
                false
            }
        }
    }

    /// 运行时开关（网页一键开关）。
    pub fn set_enabled(&self, enabled: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.enabled = enabled;
            if !enabled {
                if let Some(c) = inner.child.as_mut() {
                    c.stop();
                }
                inner.child = None;
                inner.registered.clear();
            } else {
                inner.last_attempt = None; // 允许立即重试
            }
        }
        self.refresh();
    }

    /// 显式停止（graceful_shutdown 调用）。
    pub fn stop(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.stopped = true;
            if let Some(c) = inner.child.as_mut() {
                c.stop();
            }
            inner.child = None;
            inner.registered.clear();
        }
        self.refresh();
    }

    /// 状态摘要（供 API/系统状态）。
    pub fn status(&self) -> serde_json::Value {
        if let Ok(mut inner) = self.inner.lock() {
            let running = inner.child.as_mut().is_some_and(ChildGuard::is_running);
            serde_json::json!({
                "enabled": inner.enabled,
                "running": running,
                "pid": inner.child.as_ref().and_then(|c| c.pid()).filter(|_| running),
                "last_error": inner.last_error,
                "registered_streams": inner.registered.len(),
                "external": self.external.is_some(),
            })
        } else {
            serde_json::json!({"enabled": false, "running": false, "last_error": "state poisoned"})
        }
    }

    /// 注册/注销 RTSP 流到 go2rtc API（PUT /api/streams?name&src；DELETE 同路径）。
    /// 纯 HTTP 调用，失败如实返回 Err（不阻塞主流程）。
    pub fn apply_stream(api_base: &str, name: &str, source: Option<&str>) -> Result<(), String> {
        let base = api_base.trim_end_matches('/');
        let name = percent_encode(name);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(2)))
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into();
        let mut response = match source {
            Some(src) => {
                let url = format!("{base}/api/streams?name={name}&src={}", percent_encode(src));
                agent
                    .put(&url)
                    .send_empty()
                    .map_err(|_| "go2rtc 注册流失败".to_string())?
            }
            None => agent
                .delete(&format!("{base}/api/streams?src={name}"))
                .call()
                .map_err(|_| "go2rtc 注销流失败".to_string())?,
        };
        let status = response.status();
        // 内联配置在内存中完成增删后，持久化会返回该提示。
        // 这里刻意使用内联配置，保持 RTSP 源只保存在 LabSentinel 的数据库。
        let applied_in_memory = status.as_u16() == 400
            && response
                .body_mut()
                .with_config()
                .limit(256)
                .read_to_string()
                .is_ok_and(|body| body.trim() == "config file disabled");
        if status.is_success() || applied_in_memory {
            Ok(())
        } else {
            Err(format!("go2rtc 流同步 HTTP {status}"))
        }
    }

    pub fn refresh(&self) {
        self.wake.notify_one();
    }

    pub fn start(self: &Arc<Self>, db_path: String, install_root: PathBuf) {
        let supervisor = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let task = Arc::clone(&supervisor);
                let path = db_path.clone();
                let root = install_root.clone();
                if tokio::task::spawn_blocking(move || task.sync_cameras(&path, &root))
                    .await
                    .is_err()
                {
                    break;
                }
                if supervisor.inner.lock().map_or(true, |state| state.stopped) {
                    break;
                }
                tokio::select! {
                    _ = supervisor.wake.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                }
            }
        });
    }

    fn sync_cameras(&self, db_path: &str, install_root: &Path) {
        if self.external.is_some() {
            return;
        }
        let base = format!("http://{}", self.address);
        if !self.ensure_running(install_root, &base) || !api_reachable(self.address) {
            return;
        }
        let Ok(cameras) = ls_store::Store::open(db_path).and_then(|s| s.cameras()) else {
            return;
        };
        let desired: HashMap<_, _> = cameras
            .into_iter()
            .filter(|c| c.type_ == "rtsp")
            .map(|c| (c.id, c.source))
            .collect();
        let (pid, registered) = match self.inner.lock() {
            Ok(inner) => (
                inner.child.as_ref().and_then(ChildGuard::pid),
                inner.registered.clone(),
            ),
            Err(_) => return,
        };
        let changes = desired
            .iter()
            .filter(|(id, source)| registered.get(id) != Some(source))
            .map(|(&id, source)| (id, Some(source.as_str())))
            .chain(
                registered
                    .keys()
                    .filter(|id| !desired.contains_key(id))
                    .map(|&id| (id, None)),
            );
        for (id, source) in changes {
            let result = Self::apply_stream(&base, &format!("cam_{id}"), source);
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            if pid.is_none() || inner.child.as_ref().and_then(ChildGuard::pid) != pid {
                return;
            }
            match result {
                Ok(()) => match source {
                    Some(url) => {
                        inner.registered.insert(id, url.to_string());
                    }
                    None => {
                        inner.registered.remove(&id);
                    }
                },
                Err(err) => inner.last_error = format!("相机 #{id}: {err}"),
            }
        }
    }

    pub fn address_for_camera(&self, id: i64) -> Option<String> {
        let mut inner = self.inner.lock().ok()?;
        if !inner.enabled || inner.stopped {
            return None;
        }
        if let Some(address) = &self.external {
            return Some(address.clone());
        }
        (inner.registered.contains_key(&id)
            && inner.child.as_mut().is_some_and(ChildGuard::is_running))
        .then(|| self.address.to_string())
    }

    pub(crate) fn local_analysis_source(&self, id: i64, main: &str) -> Option<String> {
        let rtsp: SocketAddr = self.rtsp_listen.parse().ok()?;
        if self.external.is_some() || !rtsp.ip().is_loopback() || rtsp.port() == 0 {
            return None;
        }
        let mut inner = self.inner.lock().ok()?;
        (inner.enabled
            && !inner.stopped
            && inner
                .registered
                .get(&id)
                .is_some_and(|source| source == main)
            && inner.child.as_mut().is_some_and(ChildGuard::is_running))
        .then(|| format!("rtsp://{rtsp}/cam_{id}"))
    }
}

fn api_reachable(address: SocketAddr) -> bool {
    std::net::TcpStream::connect_timeout(&address, Duration::from_millis(80)).is_ok()
}

/// 最小百分号编码（流名进 query）。
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 全局共享 supervisor（懒启动：Arc 由 AppState 持有，见 state.rs）。
pub type SharedGo2rtc = Arc<Go2rtcSupervisor>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_bin_prefers_env_then_install() {
        let tmp = std::env::temp_dir().join(format!("ls-go2rtc-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(tmp.join("tools"));
        let bin = tmp.join("tools/go2rtc.exe");
        let _ = std::fs::write(&bin, b"stub");
        assert_eq!(Go2rtcSupervisor::resolve_bin(&tmp), Some(bin.clone()));
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[test]
    fn status_disabled_by_default() {
        let sup = Go2rtcSupervisor::new(false);
        let s = sup.status();
        assert_eq!(s["enabled"], false);
        assert_eq!(s["running"], false);
    }

    #[test]
    fn percent_encode_safe() {
        assert_eq!(percent_encode("cam 1/流"), "cam%201%2F%E6%B5%81");
    }

    #[test]
    fn missing_go2rtc_uses_snapshot_fallback() {
        if std::env::var_os("LS_GO2RTC").is_some() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let sup = Go2rtcSupervisor::new(true);
        assert!(!sup.ensure_running(root.path(), "http://127.0.0.1:1984"));
        let status = sup.status();
        assert_eq!(status["enabled"], true);
        assert_eq!(status["running"], false);
    }

    #[cfg(windows)]
    #[test]
    fn bundled_go2rtc_accepts_stream_registration() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        if !root.join("tools/go2rtc.exe").is_file() {
            return;
        }
        let socket = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = socket.local_addr().unwrap();
        drop(socket);
        let api = format!("http://{address}");
        let sup = Go2rtcSupervisor::new(true);
        assert!(sup.ensure_running(&root, &api));
        for _ in 0..40 {
            if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Go2rtcSupervisor::apply_stream(&api, "cam_42", Some("rtsp://127.0.0.1:1/test?foo=bar"))
            .unwrap();
        let mut response = ureq::get(format!("{api}/api/streams")).call().unwrap();
        let streams: serde_json::Value =
            serde_json::from_str(&response.body_mut().read_to_string().unwrap()).unwrap();
        assert!(streams.get("cam_42").is_some());
        Go2rtcSupervisor::apply_stream(&api, "cam_42", None).unwrap();
        let mut response = ureq::get(format!("{api}/api/streams")).call().unwrap();
        let streams: serde_json::Value =
            serde_json::from_str(&response.body_mut().read_to_string().unwrap()).unwrap();
        assert!(streams.get("cam_42").is_none());
        sup.stop();
        assert_eq!(sup.status()["running"], false);
    }

    #[cfg(windows)]
    #[test]
    fn supervised_gateway_recovers_existing_streams() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        if !root.join("tools/go2rtc.exe").is_file() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("go2rtc.db");
        let db = db.to_str().unwrap();
        let store = ls_store::Store::open(db).unwrap();
        store.ensure_schema().unwrap();
        let cid = store
            .conn_add_camera("test", "rtsp://127.0.0.1:1/fixture", "rtsp")
            .unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let sup = Go2rtcSupervisor {
            address,
            external: None,
            ..Go2rtcSupervisor::new(true)
        };
        let wait_for_registration = || {
            for _ in 0..60 {
                sup.sync_cameras(db, &root);
                if let Some(address) = sup.address_for_camera(cid) {
                    return address;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            panic!("stream registry never became ready: {}", sup.status());
        };
        assert_eq!(wait_for_registration(), address.to_string());
        let pid = sup.status()["pid"].clone();
        {
            let mut inner = sup.inner.lock().unwrap();
            inner.child.as_mut().unwrap().stop();
            inner.last_attempt = None;
        }
        assert!(sup.address_for_camera(cid).is_none());
        assert_eq!(wait_for_registration(), address.to_string());
        assert_ne!(sup.status()["pid"], pid);
        sup.stop();
    }
}
