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
    pub fn validate(&self, dir_name: &str) -> Result<(), String> {
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
        // RA2 反思修正：原子写（临时文件 + os.replace）——半写的 state.json
        // 会让下次启动丢失全部启用状态（Python 侧同口径）
        let target = state_path(st);
        let tmp = target.with_extension("json.tmp");
        if std::fs::write(&tmp, t).is_ok() {
            let _ = std::fs::rename(&tmp, &target);
        }
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
    /// P1-5（安全审计）：spawn 前（持锁）检查重入——旧进程存活则拒绝，
    /// 防句柄覆盖成孤儿 + 进程增殖（P1-4 同批：环境白名单重建见下方 spawn）。
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
        if let Ok(procs) = self.procs.lock() {
            if procs.contains_key(pid) {
                return (false, "已在运行".into());
            }
        }
        // 入口解析：{python} 占位 → Python 解释器（Rust 形态无嵌入式 runtime：
        // 依次尝试安装目录 runtime\python.exe → PATH；current_exe 是本服务自身，
        // 绝不能作为 {python} 替换目标——RA2 反思修正）；PATH 命令；目录内锚定
        let dir = plugin_dir(st).join(pid);
        let mut cmd = m.command.clone();
        let exe = if cmd[0].contains("{python}") {
            let mut py: Option<PathBuf> = None;
            for p in [
                plugin_dir(st)
                    .parent()
                    .map(|d| d.join("runtime").join("python.exe")),
                Some(PathBuf::from("python.exe")),
            ]
            .into_iter()
            .flatten()
            {
                if p.is_file() {
                    py = Some(p);
                    break;
                }
                if p.parent().is_none() && which("python.exe").is_some() {
                    py = Some(p);
                    break;
                }
            }
            let Some(py) = py else {
                return (
                    false,
                    "{python} 占位不可用：未找到 Python 解释器（Rust 形态不自带）".into(),
                );
            };
            cmd[0] = cmd[0].replace("{python}", &py.to_string_lossy());
            py
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
        // P1-4（安全审计）：环境白名单重建——绝不让子进程继承父环境
        // （LS_ADMIN_TOKEN/LS_COMPREFACE_API_KEY 等会被插件直接读走）。
        // 对齐 Python _ENV_PASS：保留系统必需项 + 清 LS_* + 按 manifest.env 注入。
        {
            use std::collections::HashMap as _Hm;
            const PASS: &[&str] = &[
                "SystemRoot",
                "windir",
                "TEMP",
                "TMP",
                "PATH",
                "PATHEXT",
                "COMSPEC",
                "NUMBER_OF_PROCESSORS",
                "PROCESSOR_ARCHITECTURE",
                "OS",
                "USERNAME",
            ];
            let pass: _Hm<String, String> = std::env::vars()
                .filter(|(k, _)| PASS.iter().any(|p| p.eq_ignore_ascii_case(k)))
                .collect();
            c.env_clear();
            for (k, v) in pass {
                c.env(k, v);
            }
            for (k, v) in &m.env {
                c.env(k, v);
            }
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

    /// RA5-UX：插件 zip 上传安装——解压（防 zip slip）→ plugin.json 校验 →
    /// 落位 data/plugins/<id>/。已存在同 id 插件 → 409 拒绝（防覆盖）。
    /// 校验失败整包丢弃（不留半装状态）。
    pub fn install_zip(&self, st: &Shared, zip_bytes: &[u8]) -> Result<String, (u16, String)> {
        use std::io::Read;
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
            .map_err(|e| (400u16, format!("zip 解析失败: {e}")))?;
        // 顶层必须有 plugin.json
        if archive.by_name("plugin.json").is_err() {
            return Err((400, "zip 顶层缺少 plugin.json".into()));
        }
        // 先读 manifest 校验 id（落位目录名 = id）
        let manifest_text = {
            let mut mf = archive
                .by_name("plugin.json")
                .map_err(|e| (400u16, format!("plugin.json 读取失败: {e}")))?;
            let mut buf = Vec::new();
            mf.read_to_end(&mut buf)
                .map_err(|e| (400u16, format!("plugin.json 读取失败: {e}")))?;
            String::from_utf8_lossy(&buf).into_owned()
        };
        let m: PluginManifest = serde_json::from_str(&manifest_text)
            .map_err(|e| (400u16, format!("清单 JSON 非法: {e}")))?;
        m.validate(&m.id).map_err(|e| (400u16, e))?;
        let target = plugin_dir(st).join(&m.id);
        if target.exists() {
            return Err((409u16, format!("插件 {} 已存在", m.id)));
        }
        // 解压：逐 entry 防 zip slip（canonicalize 后必须落在 target 内）
        let target_c = target
            .canonicalize()
            .or_else(|_| std::fs::create_dir_all(&target).map(|_| target.clone()))
            .map_err(|e| (500u16, format!("创建目录失败: {e}")))?;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| (400u16, format!("zip 条目读取失败: {e}")))?;
            let name = entry.name().to_string();
            if name.ends_with('/') {
                continue; // 目录条目
            }
            let dest = match entry.enclosed_name() {
                Some(p) => target.join(p),
                None => return Err((400u16, format!("zip 条目路径非法: {name}"))),
            };
            let dest_c = dest.canonicalize().unwrap_or_else(|_| dest.clone());
            if !dest_c.starts_with(&target_c) {
                return Err((400u16, format!("zip 条目逃逸安装目录: {name}")));
            }
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut w =
                std::fs::File::create(&dest).map_err(|e| (500u16, format!("写文件失败: {e}")))?;
            std::io::copy(&mut entry, &mut w)
                .map_err(|e| (500u16, format!("解压写入失败: {e}")))?;
        }
        Ok(m.id)
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
