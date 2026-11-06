//! 路径与磁盘工具（retention/backup 共用）。
//!
//! 安全口径与 Python `retention._safe_within` 一致：**真实路径**包含判断
//! （canonicalize 后按路径分量比较，`data-evil` 不算 `data` 子目录；`..` 无法穿越），
//! 不存在的路径一律返回 false（只删确认存在的文件）。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// 路径必须在 parent 内（含 parent 本身）且真实存在。
pub fn safe_within(path: &Path, parent: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let (Ok(real), Ok(base)) = (fs::canonicalize(path), fs::canonicalize(parent)) else {
        return false;
    };
    real == base || real.starts_with(&base)
}

/// 与 Python `os.path.normcase(os.path.abspath(p))` 等价（Windows 小写 + 反斜杠）。
pub fn norm_key(path: &Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let s = abs.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        s.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        s
    }
}

/// 递归收集目录下的普通文件（不跟随符号链接目录，避免环）。
pub fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            walk_files(&entry.path(), out)?;
        } else if meta.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}

/// 目录递归大小（不可读项按 0 计，与 Python os.walk + getsize 忽略 OSError 同语义）。
pub fn dir_bytes(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            match entry.metadata() {
                Ok(m) if m.is_dir() => total += dir_bytes(&p),
                Ok(m) if m.is_file() => total += m.len(),
                _ => {}
            }
        }
    }
    total
}

/// 可用空间 GB（异常返回 +∞，与 Python `_disk_free_gb` 一致：宁可不紧急清理）。
pub fn disk_free_gb(data_dir: &Path) -> f64 {
    fs2::available_space(data_dir)
        .map(|b| b as f64 / 1e9)
        .unwrap_or(f64::INFINITY)
}

/// 磁盘总容量 GB（异常返回 None）。
pub fn disk_total_gb(data_dir: &Path) -> Option<f64> {
    fs2::total_space(data_dir).ok().map(|b| b as f64 / 1e9)
}
