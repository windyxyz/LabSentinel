//! UX3（2026-11-20）：日志文件化——无黑框 ≠ 黑盒。
//!
//! - `data/logs/server-YYYYMMDD.log` 按天滚动（本地日期），保留 14 天；
//! - 纯标准库实现（无 chrono/tracing 依赖；时间戳复用 util::iso_now 口径）；
//! - 增量收编：main/worker/关键错误路径先接（logger::error/warn/info），
//!   存量 println 分批迁移，不阻塞本轮（roadmap 非目标明示）；
//! - panic hook 同文件落盘（GUI 双击启动时崩溃现场可查）。
//!
//! 线程模型：worker/复核/派发线程并发追加——每次写入独立 open+append+
//! close（O_APPEND 原子追加语义，无锁；代价是小日志量下的打开开销，可接受）。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
/// 日志保留天数（UX3.2 口径：14 天）
const RETENTION_DAYS: i64 = 14;

/// 初始化：建目录 + 清理超期文件。失败不阻断启动（降级为仅控制台输出），
/// 返回日志文件路径供启动横幅展示。
pub fn init(data_dir: &Path) -> Option<PathBuf> {
    let dir = data_dir.join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let _ = LOG_DIR.set(dir.clone());
    prune_old(&dir);
    Some(today_file(&dir))
}

fn today_file(dir: &Path) -> PathBuf {
    dir.join(format!("server-{}.log", today_local()))
}

/// 本地日期（统一走 `ls_core::time` 本地时区，00:00 滚动而非 UTC 08:00）。
fn today_local() -> String {
    ls_core::time::local_today_compact()
}

/// 清理超期日志（>RETENTION_DAYS 天的 server-*.log）。
fn prune_old(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let today = today_local();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name
            .strip_prefix("server-")
            .and_then(|s| s.strip_suffix(".log"))
        {
            if rest < today.as_str() {
                // 按日期字符串比较：今天之前的都可能是超期文件，再按天数精确判
                if let Ok(d) = std::fs::metadata(e.path())
                    .and_then(|m| m.modified().map_err(|_| std::io::Error::other("")))
                {
                    let age = std::time::SystemTime::now()
                        .duration_since(d)
                        .unwrap_or_default()
                        .as_secs();
                    if age > RETENTION_DAYS as u64 * 86_400 {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
    }
}

fn append(level: &str, msg: &str) {
    let Some(dir) = LOG_DIR.get() else {
        return; // 未初始化（测试/极早启动路径）——仅调用方控制台输出
    };
    // 按天滚动：日期变更后新文件自动生效（文件名每天现算）
    let line = format!("{} [{level}] {}\n", crate::util::iso_now(), msg);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(today_file(dir))
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn info(msg: &str) {
    println!("{msg}");
    append("INFO", msg);
}

pub fn warn(msg: &str) {
    eprintln!("{msg}");
    append("WARN", msg);
}

pub fn error(msg: &str) {
    eprintln!("{msg}");
    append("ERROR", msg);
}

/// panic hook：崩溃现场同文件落盘（GUI 模式无控制台可看）。
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".into());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        append("PANIC", &format!("at {loc}: {msg}"));
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_local_format_is_yyyymmdd() {
        let s = today_local();
        assert_eq!(s.len(), 8);
        assert!(s.bytes().all(|b| b.is_ascii_digit()));
    }

    #[test]
    fn init_creates_dir_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let f = init(dir.path()).expect("init 成功");
        error("测试错误行");
        let content = std::fs::read_to_string(&f).unwrap();
        assert!(content.contains("[ERROR] 测试错误行"));
        assert!(!content.contains("[PANIC"));
    }
}
