//! R2-301/N-22：统一的外部二进制查找（ffmpeg/ffprobe/go2rtc）。
//!
//! Python 侧口径见 `server/app/platform/binaries.py::find_binary`。便携包把
//! ffmpeg 放在 `tools/ffmpeg/` 且不在 PATH，旧 Rust 代码直接 `Command::new("ffmpeg")`
//! 会导致取流/存证在便携包里整体失败。
//!
//! 查找顺序：
//! 1. `LS_<NAME>` 环境变量（大写，如 `LS_FFMPEG`）——由 Python 父进程显式下发；
//! 2. `<exe 同级>/tools/<os>-<arch>/<name>[.exe]`；
//! 3. `<exe 同级>/tools/<name>[.exe]` 与 `<exe 同级>/tools/ffmpeg/<name>[.exe]`；
//! 4. PATH（`which` 语义，用 `std::process::Command` 探测版本即可判定可用）。

use std::env;
use std::path::{Path, PathBuf};

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn os_arch() -> String {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    };
    format!("{os}-{arch}")
}

/// 返回第一个存在的候选路径，找不到时返回 `name` 本身（交给 PATH 解析）。
pub fn find(name: &str) -> PathBuf {
    let exe = exe_name(name);
    let env_key = format!("LS_{}", name.to_ascii_uppercase());
    if let Some(v) = env::var_os(&env_key) {
        let p = PathBuf::from(v);
        if p.is_file() {
            return p;
        }
    }
    if let Ok(current) = env::current_exe() {
        if let Some(base) = current.parent() {
            let tools = base.join("tools");
            let candidates = [
                tools.join(os_arch()).join(&exe),
                tools.join(&exe),
                tools.join("ffmpeg").join(&exe),
                base.parent()
                    .unwrap_or(base)
                    .join("tools")
                    .join("ffmpeg")
                    .join(&exe),
            ];
            for c in candidates {
                if c.is_file() {
                    return c;
                }
            }
        }
    }
    PathBuf::from(name)
}

/// 便携包布局下把 `tools/` 目录加入进程 PATH（子进程继承），
/// 这样即使调用方仍用裸 `Command::new("ffmpeg")` 也能找到随包二进制。
pub fn ensure_tools_on_path() {
    if let Ok(current) = env::current_exe() {
        if let Some(base) = current.parent() {
            let candidates = [base.join("tools").join("ffmpeg"), base.join("tools")];
            let mut add: Vec<PathBuf> = candidates.iter().filter(|p| p.is_dir()).cloned().collect();
            if !add.is_empty() {
                let mut paths: Vec<PathBuf> =
                    env::split_paths(&env::var_os("PATH").unwrap_or_default()).collect();
                for p in add.drain(..) {
                    if !paths.contains(&p) {
                        paths.insert(0, p);
                    }
                }
                if let Ok(joined) = env::join_paths(paths) {
                    env::set_var("PATH", joined);
                }
            }
        }
    }
}

/// 显式检查某个二进制是否可用（用于测试 skip 判定）。
pub fn available(name: &str) -> bool {
    std::process::Command::new(find(name))
        .arg("-version")
        .output()
        .map(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        .unwrap_or(false)
}

/// 与 Python `platform.binaries.find_binary` 等价的目录前缀（供诊断输出）。
pub fn tools_dir() -> Option<PathBuf> {
    let current = env::current_exe().ok()?;
    let base = current.parent()?;
    let d: &Path = base;
    Some(d.join("tools"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_name_windows_suffix() {
        let e = exe_name("ffmpeg");
        if cfg!(windows) {
            assert_eq!(e, "ffmpeg.exe");
        } else {
            assert_eq!(e, "ffmpeg");
        }
    }

    #[test]
    fn env_override_wins() {
        let dir = env::temp_dir().join("ls-bin-test");
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("ls-fake-bin.exe");
        std::fs::write(&fake, b"x").unwrap();
        env::set_var("LS_LSFAKEBIN", &fake);
        assert_eq!(find("lsfakebin"), fake);
        env::remove_var("LS_LSFAKEBIN");
        let _ = std::fs::remove_file(fake);
    }

    #[test]
    fn missing_binary_falls_back_to_name() {
        assert_eq!(
            find("definitely-not-a-real-binary-xyz"),
            PathBuf::from("definitely-not-a-real-binary-xyz")
        );
    }
}
