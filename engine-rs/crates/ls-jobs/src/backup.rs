//! 在线热备份（T-330 / Python `scripts/backup_data.py` 同格式）。
//!
//! - `rusqlite::backup` 热快照（运行中可直接备份，SQLite 保证一致性）；
//! - zip 布局与 manifest 字段与 Python 完全一致（manifest.json + labsentinel.db +
//!   media/<相对 data_dir 路径>），Python `backup_data.py verify/restore` 可直接消费；
//! - 自动备份只管理 `auto-*.zip`，保留最近 N 份；升级路径与手动备份不受影响；
//! - verify 只读；restore 保守（不删目标中备份没有的文件），覆盖前转存 .bak-<ts>。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::Local;
use ls_store::Store;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::paths::walk_files;
use crate::retention::RetentionConfig;
use crate::JobResult;

const MEDIA_DIRS: [&str; 3] = ["snapshots", "videos", "annotation_pool"];
const TOOL_VERSION: &str = "backup-1.0";

#[derive(Clone, Debug)]
pub struct BackupConfig {
    pub data_dir: PathBuf,
    pub interval_h: i64,
    pub keep: usize,
    pub include_media: bool,
}

impl BackupConfig {
    pub fn for_data_dir(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            interval_h: 24,
            keep: 7,
            include_media: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BackupOutcome {
    pub path: String,
    pub name: String,
    pub zip_bytes: u64,
    pub entries: usize,
    pub db_bytes: usize,
    pub media_files: usize,
    pub media_bytes: u64,
    pub integrity_check: String,
}

impl BackupOutcome {
    pub fn message(&self) -> String {
        format!(
            "OK {} ({} 文件，zip {:.1}MB；db {:.1}MB + 媒体 {} 个/{:.1}MB，integrity={})",
            self.name,
            self.entries,
            self.zip_bytes as f64 / 1e6,
            self.db_bytes as f64 / 1e6,
            self.media_files,
            self.media_bytes as f64 / 1e6,
            self.integrity_check
        )
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VerifyReport {
    pub db_bytes: usize,
    pub db_sha256: String,
    pub media_entries: usize,
    pub integrity_check: String,
    pub created_at: String,
    pub tool_version: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RestoreReport {
    pub db_bytes: usize,
    pub media_files: usize,
    pub integrity_check: String,
}

pub fn backup_dir(cfg: &BackupConfig) -> PathBuf {
    cfg.data_dir.join("backups")
}

/// 按 mtime 新→旧排序的 auto-*.zip。
pub fn auto_files(cfg: &BackupConfig) -> Vec<PathBuf> {
    let dir = backup_dir(cfg);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, SystemTime)> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?;
            if !name.starts_with("auto-") || !name.ends_with(".zip") {
                return None;
            }
            let m = e.metadata().ok()?.modified().ok()?;
            Some((p, m))
        })
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.1));
    files.into_iter().map(|(p, _)| p).collect()
}

/// 距离上次备份是否已超过 interval（无备份=需要）。
pub fn needs_backup(files: &[PathBuf], now: SystemTime, interval_h: i64) -> bool {
    let Some(latest) = files.first() else {
        return true;
    };
    let Ok(m) = latest.metadata().and_then(|md| md.modified()) else {
        return true;
    };
    now.duration_since(m)
        .map(|d| d > Duration::from_secs(interval_h.max(0) as u64 * 3600))
        .unwrap_or(false)
}

fn prune(files: &[PathBuf], keep: usize) -> usize {
    let mut removed = 0;
    for old in files.iter().skip(keep) {
        if fs::remove_file(old).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn safe_entry(name: &str) -> bool {
    let n = name.replace('\\', "/");
    if n.starts_with('/') || (n.len() > 1 && n.as_bytes()[1] == b':') {
        return false;
    }
    !n.split('/').any(|c| c == "..")
}

fn posix_rel(p: &Path, base: &Path) -> Option<String> {
    let rel = p.strip_prefix(base).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

fn temp_db_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("ls-jobs-{tag}-{}-{nanos}.db", std::process::id()))
}

fn snapshot_db(src: &Path, dst: &Path) -> JobResult<String> {
    let src_conn =
        rusqlite::Connection::open_with_flags(src, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("打开源库失败: {e}"))?;
    let mut dst_conn = rusqlite::Connection::open(dst).map_err(|e| e.to_string())?;
    {
        let backup = rusqlite::backup::Backup::new(&src_conn, &mut dst_conn)
            .map_err(|e| format!("快照失败: {e}"))?;
        backup
            .run_to_completion(5, Duration::from_millis(50), None)
            .map_err(|e| format!("快照失败: {e}"))?;
    }
    let detail: String = dst_conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(detail)
}

/// 执行一次备份；out 由调用方给定（自动备份用 auto-<时间戳>.zip）。
pub fn run_backup(cfg: &BackupConfig, out: &Path) -> JobResult<BackupOutcome> {
    let db = cfg.data_dir.join("labsentinel.db");
    if !db.is_file() {
        return Err(format!("数据库不存在: {}", db.display()));
    }
    if out.exists() {
        return Err(format!("{} 已存在", out.display()));
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let snap = temp_db_path("snapshot");
    let result = (|| -> JobResult<BackupOutcome> {
        let integrity = snapshot_db(&db, &snap)?;
        if integrity != "ok" {
            return Err(format!("快照 integrity_check 未过: {integrity}"));
        }
        let db_bytes = fs::read(&snap).map_err(|e| e.to_string())?;
        let db_sha = hex_sha256(&db_bytes);
        let mut media: Vec<(PathBuf, String)> = Vec::new();
        let mut media_bytes = 0u64;
        if cfg.include_media {
            for d in MEDIA_DIRS {
                let base = cfg.data_dir.join(d);
                if !base.is_dir() {
                    continue;
                }
                let mut files = Vec::new();
                walk_files(&base, &mut files).map_err(|e| e.to_string())?;
                for f in files {
                    if let Some(rel) = posix_rel(&f, &cfg.data_dir) {
                        media_bytes += fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                        media.push((f, rel));
                    }
                }
            }
        }
        let manifest = json!({
            "created_at": Local::now().format("%Y-%m-%dT%H:%M:%S%z").to_string(),
            "source_data_dir": cfg.data_dir.to_string_lossy(),
            "db_bytes": db_bytes.len(),
            "db_sha256": db_sha,
            "media_files": media.len(),
            "media_bytes": media_bytes,
            "integrity_check": integrity,
            "tool_version": TOOL_VERSION,
        });
        let file = File::create(out).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        zip.start_file("manifest.json", opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(
            serde_json::to_string_pretty(&manifest)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        zip.start_file("labsentinel.db", opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(&db_bytes).map_err(|e| e.to_string())?;
        for (path, rel) in &media {
            zip.start_file(format!("media/{rel}"), opts)
                .map_err(|e| e.to_string())?;
            let mut f = File::open(path).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        let zip_bytes = fs::metadata(out).map(|m| m.len()).unwrap_or(0);
        Ok(BackupOutcome {
            path: out.to_string_lossy().to_string(),
            name: out
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            zip_bytes,
            entries: 2 + media.len(),
            db_bytes: db_bytes.len(),
            media_files: media.len(),
            media_bytes,
            integrity_check: integrity,
        })
    })();
    let _ = fs::remove_file(&snap);
    result
}

/// 自动备份入口：生成 auto-<时间戳>.zip，成功后按 keep 清理旧 auto 备份。
pub fn run_auto_backup(cfg: &BackupConfig) -> JobResult<BackupOutcome> {
    let name = format!("auto-{}.zip", Local::now().format("%Y%m%d-%H%M%S"));
    let out = backup_dir(cfg).join(name);
    let outcome = run_backup(cfg, &out)?;
    prune(&auto_files(cfg), cfg.keep);
    Ok(outcome)
}

/// 校验备份 zip（不落盘 data）：manifest 可解析、db SHA256 匹配、integrity_check 通过、
/// 条目路径安全。
pub fn verify_zip(zip_path: &Path) -> JobResult<VerifyReport> {
    let file = File::open(zip_path).map_err(|e| format!("zip 不存在: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip 损坏: {e}"))?;
    let names: Vec<String> = archive
        .file_names()
        .filter_map(Result::ok)
        .map(|s| s.to_string())
        .collect();
    if let Some(bad) = names.iter().find(|n| !safe_entry(n)) {
        return Err(format!("发现不安全条目路径: {bad}"));
    }
    if !names.iter().any(|n| n == "manifest.json") || !names.iter().any(|n| n == "labsentinel.db") {
        return Err("缺少 manifest.json 或 labsentinel.db".into());
    }
    let mut mtext = String::new();
    archive
        .by_name("manifest.json")
        .map_err(|e| e.to_string())?
        .read_to_string(&mut mtext)
        .map_err(|e| e.to_string())?;
    let m: serde_json::Value = serde_json::from_str(&mtext).map_err(|e| e.to_string())?;
    for k in ["db_bytes", "db_sha256", "tool_version"] {
        if m.get(k).is_none() {
            return Err(format!("manifest 缺字段: {k}"));
        }
    }
    let mut db_bytes = Vec::new();
    archive
        .by_name("labsentinel.db")
        .map_err(|e| e.to_string())?
        .read_to_end(&mut db_bytes)
        .map_err(|e| e.to_string())?;
    let expect_len = m.get("db_bytes").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let expect_sha = m
        .get("db_sha256")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if db_bytes.len() != expect_len || hex_sha256(&db_bytes) != expect_sha {
        return Err("db 大小/SHA256 与 manifest 不一致（zip 损坏或被篡改）".into());
    }
    let tmp = temp_db_path("verify");
    fs::write(&tmp, &db_bytes).map_err(|e| e.to_string())?;
    let integrity = match Store::open(&tmp.to_string_lossy()) {
        Ok(s) => s.integrity_check().unwrap_or_else(|e| e.to_string()),
        Err(e) => e.to_string(),
    };
    let _ = fs::remove_file(&tmp);
    if integrity != "ok" {
        return Err(format!("db integrity_check 失败: {integrity}"));
    }
    Ok(VerifyReport {
        db_bytes: db_bytes.len(),
        db_sha256: expect_sha,
        media_entries: names.iter().filter(|n| n.starts_with("media/")).count(),
        integrity_check: integrity,
        created_at: m
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        tool_version: m
            .get("tool_version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

fn prune_baks(db: &Path, keep: usize) -> usize {
    let Some(dir) = db.parent() else {
        return 0;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut baks: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("labsentinel.db.bak-"))
        })
        .collect();
    baks.sort();
    let mut removed = 0;
    let excess = baks.len().saturating_sub(keep);
    for old in baks.into_iter().take(excess) {
        if fs::remove_file(old).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn checkpoint(src: &Path) -> JobResult<()> {
    let conn = rusqlite::Connection::open(src).map_err(|e| format!("数据库无法打开: {e}"))?;
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .map_err(|e| format!("checkpoint 失败: {e}"))?;
    if busy != 0 {
        return Err(
            "数据库被占用（checkpoint busy）——LabSentinel 正在运行？先停止服务再恢复".into(),
        );
    }
    Ok(())
}

/// 恢复（先 verify；目标已有 db 未 force 拒绝；force 先转存 .bak-<ts>）。
pub fn restore_zip(
    cfg: &BackupConfig,
    zip_path: &Path,
    force: bool,
    no_media: bool,
) -> JobResult<RestoreReport> {
    let report = verify_zip(zip_path)?;
    let data_dir = &cfg.data_dir;
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let db = data_dir.join("labsentinel.db");
    let wal = data_dir.join("labsentinel.db-wal");
    let shm = data_dir.join("labsentinel.db-shm");
    if db.exists() {
        if !force {
            return Err(format!(
                "目标已有数据库 {}（--force 覆盖，现有 db 会转存 .bak-<时间戳>）——未修改任何文件",
                db.display()
            ));
        }
        checkpoint(&db)?;
        let bak = data_dir.join(format!(
            "labsentinel.db.bak-{}",
            Local::now().format("%Y%m%d-%H%M%S")
        ));
        fs::rename(&db, &bak).map_err(|e| format!("现有 db 转存失败（被占用/只读）: {e}"))?;
        prune_baks(&db, 3);
        let _ = fs::remove_file(&wal);
        let _ = fs::remove_file(&shm);
    } else {
        let _ = fs::remove_file(&wal);
        let _ = fs::remove_file(&shm);
    }
    let mut archive = zip::ZipArchive::new(File::open(zip_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut db_bytes = Vec::new();
    archive
        .by_name("labsentinel.db")
        .map_err(|e| e.to_string())?
        .read_to_end(&mut db_bytes)
        .map_err(|e| e.to_string())?;
    fs::write(&db, &db_bytes).map_err(|e| format!("无法写入目标 db: {e}"))?;
    let integrity = Store::open(&db.to_string_lossy())
        .and_then(|s| s.integrity_check())
        .unwrap_or_else(|e| e.to_string());
    if integrity != "ok" {
        return Err(format!("恢复后 integrity_check 失败: {integrity}"));
    }
    let mut restored = 0;
    if !no_media {
        for name in archive
            .file_names()
            .filter_map(Result::ok)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
        {
            let Some(rel) = name.strip_prefix("media/") else {
                continue;
            };
            if name.ends_with('/') || rel.is_empty() {
                continue;
            }
            if !safe_entry(&name) {
                continue;
            }
            let target = data_dir.join(rel);
            if !safe_within_parent(&target, data_dir) {
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut buf = Vec::new();
            archive
                .by_name(&name)
                .map_err(|e| e.to_string())?
                .read_to_end(&mut buf)
                .map_err(|e| e.to_string())?;
            fs::write(&target, &buf).map_err(|e| e.to_string())?;
            restored += 1;
        }
    }
    Ok(RestoreReport {
        db_bytes: report.db_bytes,
        media_files: restored,
        integrity_check: integrity,
    })
}

/// 目标路径的父目录必须解析在 data_dir 内（新文件不存在，不能 canonicalize 目标本身）。
fn safe_within_parent(target: &Path, data_dir: &Path) -> bool {
    let Some(parent) = target.parent() else {
        return false;
    };
    // 逐级向上找到已存在的祖先再 canonicalize，防 `..`/符号链接逃逸。
    let mut probe = parent;
    loop {
        if probe.exists() {
            return fs::canonicalize(probe)
                .map(|p| {
                    fs::canonicalize(data_dir)
                        .map(|b| p == b || p.starts_with(&b))
                        .unwrap_or(false)
                })
                .unwrap_or(false);
        }
        match probe.parent() {
            Some(p) => probe = p,
            None => return false,
        }
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// 自动备份状态（Python backups.status() 同字段）。
pub fn status_json(cfg: &BackupConfig, running: bool) -> serde_json::Value {
    let files = auto_files(cfg);
    let now = SystemTime::now();
    let last = files.first().and_then(|p| {
        let md = fs::metadata(p).ok()?;
        let mtime = md.modified().ok()?;
        let age_h = now
            .duration_since(mtime)
            .map(|d| (d.as_secs_f64() / 3600.0 * 10.0).round() / 10.0)
            .unwrap_or(0.0);
        Some(json!({
            "name": p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            "size": md.len(),
            "mtime": chrono::DateTime::<Local>::from(mtime).format("%Y-%m-%dT%H:%M:%S").to_string(),
            "age_h": age_h,
        }))
    });
    let total: u64 = files
        .iter()
        .filter_map(|p| fs::metadata(p).ok().map(|m| m.len()))
        .sum();
    json!({
        "enabled": cfg.interval_h > 0,
        "interval_h": cfg.interval_h,
        "keep": cfg.keep,
        "include_media": cfg.include_media,
        "last": last,
        "count": files.len(),
        "total_bytes": total,
        "running": running,
    })
}

/// 媒体目录候选（与 retention 同三目录）。
pub fn media_dirs(data_dir: &Path) -> Vec<PathBuf> {
    MEDIA_DIRS.iter().map(|d| data_dir.join(d)).collect()
}

/// 供测试/诊断：备份配置对应的 RetentionConfig（media 目录一致）。
pub fn retention_view(cfg: &BackupConfig) -> RetentionConfig {
    RetentionConfig::for_data_dir(&cfg.data_dir)
}
