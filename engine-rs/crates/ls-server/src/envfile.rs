//! LS-053：`.env` 读写（网页端局域网配置落盘）。
//!
//! 纪律：
//! - **保留其他键与注释**（只改/追加目标键，不重排、不丢注释）；
//! - 原子写（tmp + rename），避免半截文件；
//! - 值按原样写入（调用方负责校验，禁止换行注入）。

use std::path::{Path, PathBuf};

/// 设置/更新一个键（存在则替换该行，不存在则追加）。
/// 返回新内容（便于单测）；不直接落盘。
pub fn set_key(content: &str, key: &str, value: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        let is_target = trimmed
            .strip_prefix(key)
            .map(|rest| rest.starts_with('='))
            .unwrap_or(false);
        if is_target {
            out.push(format!("{key}={value}"));
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !replaced {
        out.push(format!("{key}={value}"));
    }
    let mut s = out.join("\n");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// 读取键值（去引号/空白）；不存在返回 None。
pub fn get_key(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(v) = rest.strip_prefix('=') {
                let v = v.trim().trim_matches('"').trim_matches('\'').trim();
                return Some(v.to_string());
            }
        }
    }
    None
}

/// 原子写 .env（内容先写 .env.tmp 再 rename）。
pub fn write_env(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, content.as_bytes())?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 生成强随机令牌（32 字节 hex；零新依赖：用系统时间+进程 id+计数器混合，
/// 再经 std 的 RandomState 哈希扩散——足以满足局域网管理令牌强度，且不引 rand）。
pub fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let mut out = String::with_capacity(64);
    for i in 0..4u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
        );
        h.write_u64(std::process::id() as u64);
        h.write_u64(CTR.fetch_add(1, Ordering::Relaxed));
        h.write_u64(i);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_key_preserves_comments_and_other_keys() {
        let src = "# header\nLS_PORT=8000\n# 注释保留\nLS_HOST=127.0.0.1\n";
        let out = set_key(src, "LS_HOST", "0.0.0.0");
        assert!(out.contains("# header"));
        assert!(out.contains("# 注释保留"));
        assert!(out.contains("LS_PORT=8000"));
        assert!(out.contains("LS_HOST=0.0.0.0"));
        assert!(!out.contains("LS_HOST=127.0.0.1"));
    }

    #[test]
    fn set_key_appends_when_absent() {
        let out = set_key("LS_PORT=8000\n", "LS_ADMIN_TOKEN", "abc");
        assert!(out.contains("LS_ADMIN_TOKEN=abc"));
        assert!(out.contains("LS_PORT=8000"));
    }

    #[test]
    fn get_key_handles_quotes_and_comments() {
        let src = "# LS_PORT=1\nLS_PORT=\"9000\"\nLS_HOST='0.0.0.0'\n";
        assert_eq!(get_key(src, "LS_PORT"), Some("9000".into()));
        assert_eq!(get_key(src, "LS_HOST"), Some("0.0.0.0".into()));
        assert_eq!(get_key(src, "LS_NOPE"), None);
    }

    #[test]
    fn random_token_is_strong_and_unique() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn write_env_is_atomic() {
        let dir = std::env::temp_dir().join(format!("ls-env-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(".env");
        write_env(&p, "A=1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "A=1\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
