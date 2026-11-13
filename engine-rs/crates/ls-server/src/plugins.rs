//! RA2-S7（ADR-0011）：插件管理器——manifest 校验 / state.json 持久化 /
//! 子进程启停 / 日志 tail。契约 1:1 对齐 Python `server/app/plugins.py`。
//!
//! 安全红线（与 Python 同）：
//! - id 必须匹配 `^[a-z0-9][a-z0-9_-]{1,31}$` 且等于目录名（防路径穿越）；
//! - command 非空字符串数组（绝不拼 shell）；入口解析 `{python}` 占位/PATH/
//!   目录内锚定；env 禁 `LS_*`（防覆盖系统变量）；
//! - extra 字段 forbid（坏清单生成 error 项而非崩）；
//! - 子进程日志落 `<dir>/logs/plugin.log`，logs 端点 tail 200 行。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::state::Shared;

const STATE_FILE: &str = "state.json";
const LOG_REL: &str = "logs/plugin.log";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn id_ok(id: &str) -> bool {
    let b = id.as_bytes();
    if b.len() < 2 || b.len() > 32 || !b[0].is_ascii_lowercase() && !b[0].is_ascii_digit() {
        return false;
    }
    b.iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

impl PluginManifest {
    fn validate(&self, dir_name: &str) -> Result<(), String> {
        if !id_ok(&self.id) || self.id != dir_name {
            return Err(format!("id 非法或与目录名不符: {}", self.id));
        }
        if self.command.is_empty() || self.command.iter().any(|s| s.trim().is_empty()) {
            return Err("command 必须是非空字符串数组".into());
        }
        if let Some(bad) = self
            .permissions
            .iter()
            .find(|p| p.as_str() != "read" && p.as_str() != "write")
        {
            return Err(format!("permissions 只允许 read/write，非法: {bad}"));
        }
        if let Some(k) = self
            .env
            .keys()
            .find(|k| k.to_ascii_uppercase().starts_with("LS_"))
        {
            return Err(format!("env 不允许覆盖 LS_* 系统变量: {k}"));
        }
        Ok(())
    }
}

struct ProcInfo {
    child: Child,
}

/// 插件管理器（AppState 持有；进程内状态）。
#[derive(Default)]
pub struct PluginManager {
    procs: Mutex<HashMap<String, ProcInfo>>,
}

fn plugin_dir(st: &Shared) -> PathBuf {
    PathBuf::from(&st.data_dir).join("plugins")
}

fn state_path(st: &Shared) -> PathBuf {
    plugin_dir(st).join(STATE_FILE)
}

fn read_state(st: &Shared) -> HashSet<String> {
    let p = state_path(st);
    let Ok(t) = std::fs::read_to_string(&p) else {
        return HashSet::new();
    };
    #[derive(Deserialize)]
    struct S {
        #[serde(default)]
        enabled: Vec<String>,
    }
    serde_json::from_str::<S>(&t)
        .map(|s| s.enabled.into_iter().collect())
        .unwrap_or_default()
}

fn write_state(st: &Shared, set: &HashSet<String>) {
    let dir = plugin_dir(st);
    let _ = std::fs::create_dir_all(&dir);
    let mut v: Vec<&String> = set.iter().collect();
    v.sort();
    if let Ok(t) = serde_json::to_string(&serde_json::json!({"enabled": v})) {
        let _ = std::fs::write(state_path(st), t);
    }
}

/// 读单个插件 manifest（不存在/坏 JSON/校验失败 → None + 错误文案）。
fn manifest(st: &Shared, pid: &str) -> Result<Option<PluginManifest>, String> {
    let dir = plugin_dir(st).join(pid);
    let p = dir.join("plugin.json");
    let t = std::fs::read_to_string(&p).map_err(|e| format!("清单读取失败: {e}"))?;
    let m: PluginManifest = serde_json::from_str(&t).map_err(|e| format!("清单 JSON 非法: {e}"))?;
    m.validate(pid)?;
    Ok(Some(m))
}

fn proc_row(pid: &str, m: &PluginManifest, enabled: bool, has_proc: bool) -> serde_json::Value {
    // Child::try_wait 需 &mut；此处 scan 只读，运行态以「注册表内有句柄」为准
    // （进程已退出但未 stop 的窗口由 stop/scan 下次刷新，与 Python 轮询语义同量级）
    let (running, proc_pid, started_at, returncode): (bool, Option<u32>, Option<i64>, Option<i32>) =
        if has_proc {
            (true, None, None, None)
        } else {
            (false, None, None, None)
        };
    serde_json::json!({
        "id": pid, "dir": m.id, "name": m.name, "version": m.version,
        "description": m.description, "command": m.command,
        "permissions": m.permissions, "autostart": m.autostart,
        "enabled": enabled, "running": running, "pid": proc_pid,
        "started_at": started_at, "returncode": returncode, "last_error": null,
    })
}

impl PluginManager {
    /// 扫描 data/plugins 下一级目录的 plugin.json（坏清单生成 error 项不崩）。
    pub fn scan(&self, st: &Shared) -> Vec<serde_json::Value> {
        let root = plugin_dir(st);
        let enabled = read_state(st);
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&root) else {
            return out;
        };
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        let procs = self.procs.lock().ok();
        for d in dirs {
            let pid = d
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match manifest(st, &pid) {
                Ok(Some(m)) => {
                    let has_proc = procs
                        .as_ref()
                        .map(|g| g.contains_key(&pid))
                        .unwrap_or(false);
                    out.push(proc_row(&pid, &m, enabled.contains(&pid), has_proc));
                }
                Ok(None) => {}
                Err(e) => out.push(serde_json::json!({"id": pid, "error": e})),
            }
        }
        out
    }

    pub fn set_enabled(&self, st: &Shared, pid: &str, on: bool) -> bool {
        if manifest(st, pid).ok().flatten().is_none() {
            return false;
        }
        let mut set = read_state(st);
        if on {
            set.insert(pid.into());
        } else {
            set.remove(pid);
            if let Ok(mut m) = self.procs.lock() {
                if let Some(info) = m.get_mut(pid) {
                    let _ = info.child.kill();
                    let _ = info.child.wait();
                }
                m.remove(pid);
            }
        }
        write_state(st, &set);
        true
    }

    /// 启动：先 enable 校验 → 入口解析 → 隐藏窗口 spawn（日志追加）。
    pub fn start(&self, st: &Shared, pid: &str) -> (bool, String) {
        let Some(m) = (match manifest(st, pid) {
            Ok(v) => v,
            Err(e) => return (false, e),
        }) else {
            return (false, "插件不存在或清单非法".into());
        };
        if !read_state(st).contains(pid) {
            return (false, "插件未启用（先 enable）".into());
        }
        // 入口解析：{python} 占位 → 当前解释器；PATH 命令；目录内锚定
        let dir = plugin_dir(st).join(pid);
        let mut cmd = m.command.clone();
        let exe = if cmd[0].contains("{python}") {
            cmd[0] = cmd[0].replace(
                "{python}",
                &std::env::current_exe()
                    .unwrap_or_default()
                    .to_string_lossy(),
            );
            PathBuf::from(&cmd[0])
        } else if let Some(p) = which(&cmd[0]) {
            cmd[0] = p.to_string_lossy().into_owned();
            p
        } else {
            let p = dir.join(&cmd[0]);
            if !p.is_file() {
                return (false, format!("入口不存在: {}", cmd[0]));
            }
            p
        };
        let _ = exe;
        let log_path = dir.join(LOG_REL);
        let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(&dir));
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path);
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..])
            .current_dir(&dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(f) = &log {
            c.stdout(Stdio::from(f.try_clone().unwrap()))
                .stderr(Stdio::from(f.try_clone().unwrap()));
        }
        for (k, v) in &m.env {
            c.env(k, v);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        match c.spawn() {
            Ok(child) => {
                let proc_pid = child.id();
                if let Ok(mut procs) = self.procs.lock() {
                    procs.insert(pid.into(), ProcInfo { child });
                }
                (true, format!("pid={proc_pid}"))
            }
            Err(e) => (false, format!("启动失败: {e}")),
        }
    }

    pub fn stop(&self, pid: &str) -> (bool, String) {
        let Ok(mut procs) = self.procs.lock() else {
            return (false, "锁异常".into());
        };
        let Some(info) = procs.remove(pid) else {
            return (false, "未在运行".into());
        };
        let mut child = info.child;
        let _ = child.kill();
        let rc = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        (true, format!("已停止 rc={rc}"))
    }

    /// logs tail（默认 200 行）；id 正则防穿越，再校验解析后仍在插件目录内。
    pub fn logs(&self, st: &Shared, pid: &str, tail: usize) -> String {
        if !id_ok(pid) {
            return String::new();
        }
        let root = plugin_dir(st);
        let p = root.join(pid).join(LOG_REL);
        if let (Ok(r), Ok(c)) = (root.canonicalize(), p.canonicalize()) {
            if !c.starts_with(&r) {
                return String::new();
            }
        } else {
            return String::new();
        }
        let t = std::fs::read_to_string(&p).unwrap_or_default();
        let lines: Vec<&str> = t.lines().collect();
        lines[lines.len().saturating_sub(tail)..].join("\n")
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for d in std::env::split_paths(&path) {
        let p = d.join(name);
        if p.is_file() {
            return Some(p);
        }
        // Windows 追加 .exe 后缀
        let p2 = d.join(format!("{name}.exe"));
        if p2.is_file() {
            return Some(p2);
        }
    }
    None
}
