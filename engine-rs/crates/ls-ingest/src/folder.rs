//! T-326：图片文件夹源的候选扫描器（与 Python `folders.FolderWorker._candidates` 同口径）。
//!
//! 覆盖语义：扩展名白名单（小写比较）、大小上限、静置期（防读到半张截图）、
//! 真时钟异常的未来文件跳过、内部数据目录自激排除、新图优先排序、单次扫描上限。
//!
//! 与 Python 的差异（有意）：
//! - 本模块是**无状态纯函数**（每次全量枚举）；Python worker 额外维护目录 mtime
//!   增量缓存（`_dir_sig`/`_dir_children`）以省 IO。增量缓存属作业编排，随 L4
//!   迁移时在调度层实现，扫描语义不变。
//! - 符号链接目录不跟随（`follow_symlinks(false)` 同 Python）。

use std::path::{Path, PathBuf};

/// 默认扩展名（与 `LS_FOLDER_EXTS` 默认值一致，小写含点）。
pub const DEFAULT_EXTS: &[&str] = &[".jpg", ".jpeg", ".png", ".bmp", ".webp", ".tif", ".tiff"];

/// Python `_CANDIDATE_CAP`（单目录超过 5 万张只处理最新部分并告警）。
pub const CANDIDATE_CAP: usize = 50_000;

#[derive(Clone, Debug)]
pub struct FolderScanConfig {
    /// 扩展名白名单（调用方应传入小写；比较时统一 `to_ascii_lowercase`）。
    pub exts: Vec<String>,
    /// 超过此大小的图片跳过（`LS_FOLDER_MAX_FILE_MB`，防误指大文件目录）。
    pub max_file_bytes: u64,
    /// 文件修改后至少静置多久才读（`LS_FOLDER_STABLE_S`，防读到半张截图）。
    pub stable_s: f64,
    /// 是否递归子目录（`LS_FOLDER_RECURSIVE`）。
    pub recursive: bool,
}

impl Default for FolderScanConfig {
    fn default() -> Self {
        Self {
            exts: DEFAULT_EXTS.iter().map(|s| s.to_string()).collect(),
            max_file_bytes: 32 * 1024 * 1024,
            stable_s: 1.5,
            recursive: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_s: f64,
}

/// 去重键（与 Python `(camera, path, mtime_ns, size)` 同构；入库唯一约束用）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderKey {
    pub camera_id: i64,
    pub path: String,
    pub mtime_ns: i64,
    pub size: i64,
}

impl FolderKey {
    /// 稳定字符串形式（日志/持久化用；路径按原样保存，不做 normcase）。
    pub fn as_string(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.camera_id, self.path, self.mtime_ns, self.size
        )
    }
}

/// 目录是否位于内部数据目录内（防"证据图→再识别"自激循环）。
/// `internal` 应为已 resolve 的 data_dir；比较按平台规范化（Windows 大小写不敏感）。
fn is_internal(dir: &Path, internal: Option<&Path>) -> bool {
    let Some(internal) = internal else {
        return false;
    };
    let norm = |p: &Path| -> String {
        let s = p.to_string_lossy().replace('/', "\\");
        if cfg!(windows) {
            s.to_ascii_lowercase()
        } else {
            s
        }
    };
    let d = norm(dir);
    let i = norm(internal);
    d == i || d.starts_with(&(i + "\\"))
}

/// 单目录文件枚举（不含子目录；`.name().starts_with('.')` 的隐藏目录由调用方跳过）。
fn files_in_dir(
    dir: &Path,
    exts: &[String],
    max_bytes: u64,
    stable_s: f64,
    now: f64,
    out: &mut Vec<Candidate>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()))
            .unwrap_or_default();
        if !exts.iter().any(|e| e.eq_ignore_ascii_case(&ext)) {
            continue;
        }
        let Ok(md) = entry.metadata() else { continue };
        let size = md.len();
        if size == 0 {
            continue; // 可能刚创建还没写完：下轮再看
        }
        if size > max_bytes {
            continue;
        }
        let mtime_s = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        // 静置期：Windows 粗时钟可能让 age 为小负数 → clamp 0（与 Python 一致）；
        // 真时钟异常的未来文件（>5min）跳过。
        let age = (now - mtime_s).max(0.0);
        if age < stable_s {
            continue;
        }
        if now - mtime_s < -300.0 {
            continue;
        }
        out.push(Candidate {
            path,
            size,
            mtime_s,
        });
    }
}

/// 扫描候选（新图优先：mtime 降序、路径升序；超上限截断由调用方告警）。
pub fn scan_candidates(
    root: &Path,
    config: &FolderScanConfig,
    now_s: f64,
    internal_dir: Option<&Path>,
) -> Vec<Candidate> {
    let exts: Vec<String> = config
        .exts
        .iter()
        .map(|e| {
            let l = e.trim().to_ascii_lowercase();
            if l.starts_with('.') {
                l
            } else {
                format!(".{l}")
            }
        })
        .collect();
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if is_internal(&dir, internal_dir) {
            continue; // 数据目录（截图/证据/缩略图）一律跳过
        }
        files_in_dir(
            &dir,
            &exts,
            config.max_file_bytes,
            config.stable_s,
            now_s,
            &mut out,
        );
        if !config.recursive {
            break;
        }
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let Ok(ft) = entry.file_type() else { continue };
                if !ft.is_dir() {
                    continue; // 不跟随符号链接目录（follow_symlinks(false) 同 Python）
                }
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue; // 隐藏目录（.git/.cache 等）
                }
                stack.push(entry.path());
            }
        }
    }
    out.sort_by(|a, b| {
        b.mtime_s
            .partial_cmp(&a.mtime_s)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.to_string_lossy().cmp(&b.path.to_string_lossy()))
    });
    if out.len() > CANDIDATE_CAP {
        out.truncate(CANDIDATE_CAP);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(dir: &Path, name: &str, bytes: usize, age_s: u64) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, vec![7u8; bytes]).unwrap();
        if age_s > 0 {
            let t = SystemTime::now() - Duration::from_secs(age_s);
            let ft = std::fs::File::options().write(true).open(&p).unwrap();
            ft.set_modified(t).unwrap();
        }
        p
    }

    fn now() -> f64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    }

    #[test]
    fn filters_ext_size_stable_and_sorts_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let old = touch(d, "old.jpg", 10, 60);
        let mid = touch(d, "mid.png", 10, 30);
        touch(d, "note.txt", 10, 60); // 扩展名不符
        touch(d, "empty.jpg", 0, 60); // 大小 0
        touch(d, "huge.jpg", 40 * 1024 * 1024, 60); // 超上限
        let fresh = touch(d, "fresh.jpg", 10, 0); // 未静置
        let cfg = FolderScanConfig::default();
        let out = scan_candidates(d, &cfg, now(), None);
        let names: Vec<String> = out
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["mid.png", "old.jpg"], "新图优先且过滤生效");
        assert!(out.iter().any(|c| c.path == mid));
        let _ = (old, fresh);
    }

    #[test]
    fn skips_internal_data_dir_and_hidden_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("input");
        std::fs::create_dir_all(&root).unwrap();
        let sub = root.join("sub");
        let hidden = root.join(".hidden");
        let internal = root.join("data");
        for p in [&sub, &hidden, &internal] {
            std::fs::create_dir_all(p).unwrap();
        }
        touch(&sub, "a.jpg", 10, 60);
        touch(&hidden, "b.jpg", 10, 60);
        touch(&internal, "c.jpg", 10, 60);
        let cfg = FolderScanConfig::default();
        let out = scan_candidates(&root, &cfg, now(), Some(&internal));
        let names: Vec<String> = out
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.jpg"], "只保留普通子目录的图片");
    }

    #[test]
    fn non_recursive_only_root() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        touch(tmp.path(), "top.jpg", 10, 60);
        touch(&sub, "deep.jpg", 10, 60);
        let cfg = FolderScanConfig {
            recursive: false,
            ..Default::default()
        };
        let out = scan_candidates(tmp.path(), &cfg, now(), None);
        assert_eq!(out.len(), 1);
        assert!(out[0].path.ends_with("top.jpg"));
    }

    #[test]
    fn dedup_key_shape() {
        let k = FolderKey {
            camera_id: 3,
            path: r"D:\shots\a.jpg".into(),
            mtime_ns: 1_700_000_000_123_456_789,
            size: 4096,
        };
        assert_eq!(k.as_string(), r"3|D:\shots\a.jpg|1700000000123456789|4096");
        assert_ne!(
            k,
            FolderKey {
                size: 4097,
                ..k.clone()
            }
        );
    }

    #[test]
    fn future_file_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let p = touch(tmp.path(), "future.jpg", 10, 0);
        // 未来 10 分钟：真时钟异常 → 跳过（>5min）
        let ft = std::fs::File::options().write(true).open(&p).unwrap();
        ft.set_modified(SystemTime::now() + Duration::from_secs(600))
            .unwrap();
        let cfg = FolderScanConfig::default();
        assert!(scan_candidates(tmp.path(), &cfg, now(), None).is_empty());
    }

    #[test]
    fn candidate_cap_constant() {
        assert_eq!(CANDIDATE_CAP, 50_000);
    }
}
