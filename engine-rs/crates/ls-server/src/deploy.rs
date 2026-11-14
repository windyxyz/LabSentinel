//! RA4.2（ADR-0011）：版本归档与回滚——Python `install_portable.py` 的
//! `finalize --archive` / `rollback` 语义在 Rust 侧的等价实现（无 Python 运行时依赖）。
//!
//! 布局：安装根（portable 为 `LabSentinel/`，Inno 为安装目录）
//! - `app/VERSION`（优先）或 `VERSION` 记录当前版本
//! - `versions/<ver>/` 存放升级前归档树（排除 data/versions/缓存）
//! - `current.txt` 记录当前已安装版本（回滚时跳过它，选择次新归档）
//!
//! 安全锚点：版本名必须是单一路径分量（拒绝 `/ \ ..`），归档目标必须是
//! `<install>/versions` 的直接子目录；复制不跟随符号链接，避免归档逃逸。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const CURRENT_MARK: &str = "current.txt";
pub const VERSIONS_DIRNAME: &str = "versions";
/// 归档时跳过的目录：数据、历史归档、字节码缓存、待删除标记。
const IGNORED_DIRS: [&str; 4] = ["data", VERSIONS_DIRNAME, "__pycache__", "PENDING_DELETE"];

/// 安装根默认值：`LS_INSTALL_DIR` → 可执行文件所在目录（`app/` 下则取上一级）。
pub fn default_install_root() -> PathBuf {
    if let Ok(v) = std::env::var("LS_INSTALL_DIR") {
        if !v.trim().is_empty() {
            return PathBuf::from(v);
        }
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let dir = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    if dir.file_name().and_then(|s| s.to_str()) == Some("app") {
        return dir.parent().map(Path::to_path_buf).unwrap_or(dir);
    }
    dir
}

/// 当前安装版本：`app/VERSION` → `VERSION` → `LS_VERSION` → "0.0.0"。
pub fn read_version(install: &Path) -> String {
    for rel in ["app/VERSION", "VERSION"] {
        if let Ok(s) = fs::read_to_string(install.join(rel)) {
            let v = s.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    std::env::var("LS_VERSION").unwrap_or_else(|_| "0.0.0".into())
}

/// 版本名只允许单一路径分量（防穿越）。
fn safe_component(v: &str) -> Option<&str> {
    let v = v.trim();
    if v.is_empty() || v == "." || v == ".." || v.contains('/') || v.contains('\\') {
        return None;
    }
    Some(v)
}

/// 把当前安装树归档到 `<install>/versions/<version>/`；返回归档目录。
/// 失败不删除任何东西（调用方按「仅失去回滚点」处理，不阻断升级）。
pub fn archive(install: &Path) -> Result<PathBuf, String> {
    if !install.is_dir() {
        return Err(format!("安装目录不存在: {}", install.display()));
    }
    let version = read_version(install);
    let name = safe_component(&version)
        .ok_or_else(|| format!("非法版本号（不允许路径分隔/穿越）: {version:?}"))?;
    let versions = install.join(VERSIONS_DIRNAME);
    let dest = versions.join(name);
    if dest.parent() != Some(versions.as_path()) {
        return Err("归档目标越出 versions/ 目录".into());
    }
    fs::create_dir_all(&versions).map_err(|e| format!("创建 versions/ 失败: {e}"))?;
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧归档失败: {e}"))?;
    }
    copy_tree(install, &dest, false).map_err(|e| format!("归档复制失败: {e}"))?;
    Ok(dest)
}

/// 回滚到 `versions/` 中除 `current.txt` 之外的最新一版（数据目录不动）。
/// 返回已恢复的归档目录。
pub fn rollback(install: &Path) -> Result<PathBuf, String> {
    if !install.is_dir() {
        return Err(format!("安装目录不存在: {}", install.display()));
    }
    let versions = install.join(VERSIONS_DIRNAME);
    if !versions.is_dir() {
        return Err("无版本归档（versions/ 不存在），无法回滚".into());
    }
    let current = fs::read_to_string(install.join(CURRENT_MARK))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let mut cands: Vec<String> = fs::read_dir(&versions)
        .map_err(|e| format!("读取 versions/ 失败: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| safe_component(n).is_some() && n != &current)
        .collect();
    cands.sort();
    let target_name = cands
        .pop()
        .ok_or_else(|| "没有可回滚的旧版本".to_string())?;
    let target = versions.join(&target_name);
    copy_tree(&target, install, true).map_err(|e| format!("回滚复制失败: {e}"))?;
    fs::write(install.join(CURRENT_MARK), &target_name)
        .map_err(|e| format!("写 current.txt 失败: {e}"))?;
    Ok(target)
}

/// 递归复制；`overwrite=true` 时允许目标已存在（回滚覆盖安装根）。
/// 跳过归档排除目录、`*.pyc` 与符号链接（不跟随，防逃逸）。
fn copy_tree(src: &Path, dst: &Path, overwrite: bool) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let from = entry.path();
        let to = dst.join(&name);
        let meta = entry.metadata()?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if IGNORED_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            copy_tree(&from, &to, overwrite)?;
        } else if meta.is_file() {
            if name_str.ends_with(".pyc") {
                continue;
            }
            if !overwrite && to.exists() {
                continue;
            }
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("ls-deploy-{tag}-{nanos}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(p: &Path, s: &str) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, s).unwrap();
    }

    #[test]
    fn archive_copies_tree_and_skips_data() {
        let root = tmp_root("arch");
        write(&root.join("app/VERSION"), "1.2.3\n");
        write(&root.join("app/server/main.py"), "print(1)");
        write(&root.join("data/labsentinel.db"), "db");
        write(&root.join("versions/0.0.1/old.txt"), "old");
        let dest = archive(&root).expect("archive ok");
        assert_eq!(dest, root.join("versions").join("1.2.3"));
        assert!(dest.join("app/server/main.py").is_file());
        assert!(!dest.join("data").exists(), "data/ 不应进入归档");
        assert!(!dest.join("versions").exists(), "versions/ 不应递归归档");
        // 版本读取优先级：app/VERSION 优先
        assert_eq!(read_version(&root), "1.2.3");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_rejects_traversal_version() {
        let root = tmp_root("trav");
        write(&root.join("VERSION"), "../evil");
        assert!(archive(&root).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_restores_latest_archive_and_writes_current() {
        let root = tmp_root("rb");
        // 当前版本 2.0.0；归档里有两版，应回滚到 1.1.0（最新旧版）
        write(&root.join("app/VERSION"), "2.0.0");
        write(&root.join("app/server/main.py"), "new");
        write(&root.join("versions/1.0.0/app/server/main.py"), "old-1.0");
        write(&root.join("versions/1.1.0/app/server/main.py"), "old-1.1");
        write(&root.join("current.txt"), "2.0.0");
        let restored = rollback(&root).expect("rollback ok");
        assert!(restored.ends_with("1.1.0"));
        assert_eq!(
            fs::read_to_string(root.join("app/server/main.py")).unwrap(),
            "old-1.1"
        );
        assert_eq!(
            fs::read_to_string(root.join("current.txt")).unwrap(),
            "1.1.0"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rollback_without_versions_fails() {
        let root = tmp_root("nov");
        write(&root.join("app/VERSION"), "1.0.0");
        assert!(rollback(&root).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
