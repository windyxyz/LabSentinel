//! 数据保留与清理（T-330 / Python `retention.py` 等价实现）。
//!
//! - 过期事件：先删证据文件（必须在 data_dir 内）再删行，分批 50 条；
//! - 误报标注池按修改时间过期；audit_log / alert_deliveries 与事件同保留口径；
//! - 图片识别记录只删派生证据副本与缩略图，原始截图永不删除；
//! - 孤儿媒体：无任何事件/图片记录引用、超过 1h 宽限、单轮上限 500；不递归子目录；
//! - 紧急清理三档（B-20）：已处置 video → 已处置 snapshot → 按 level 升序删行，
//!   未处置事件被删时打印 ERROR；磁盘下限由调用方注入，便于测试。
//!
//! 所有文件删除都经过 [`crate::paths::safe_within`] 真实路径校验。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::Local;
use ls_store::Store;
use serde::Serialize;
use serde_json::{json, Value};

use crate::paths::{disk_free_gb, disk_total_gb, norm_key, safe_within};
use crate::JobResult;

const BATCH: usize = 50;
const ORPHAN_GRACE_S: u64 = 3600;
const ORPHAN_MAX_PER_RUN: usize = 500;
const ORPHAN_EXTS: [&str; 9] = [
    "jpg", "jpeg", "png", "bmp", "webp", "mp4", "avi", "mkv", "mov",
];

#[derive(Clone, Debug)]
pub struct RetentionConfig {
    pub data_dir: PathBuf,
    pub snapshot_dir: PathBuf,
    pub video_dir: PathBuf,
    pub annotation_pool_dir: PathBuf,
    pub retention_events_days: i64,
    pub retention_pool_days: i64,
    pub retention_images_days: i64,
    pub disk_min_free_gb: f64,
}

impl RetentionConfig {
    pub fn for_data_dir(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            snapshot_dir: data_dir.join("snapshots"),
            video_dir: data_dir.join("videos"),
            annotation_pool_dir: data_dir.join("annotation_pool"),
            retention_events_days: 180,
            retention_pool_days: 90,
            retention_images_days: 30,
            disk_min_free_gb: 5.0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CleanupStats {
    pub deleted_events: usize,
    pub deleted_files: usize,
    pub freed_mb: f64,
    pub deleted_audit: usize,
    pub deleted_deliveries: usize,
    pub deleted_images: usize,
    pub deleted_orphans: usize,
    pub freed_orphan_mb: f64,
    pub orphans_limited: bool,
    pub ts: String,
    pub disk_free_gb: f64,
}

fn now_iso() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn cutoff_iso(days: i64) -> String {
    (Local::now() - chrono::TimeDelta::days(days.max(0)))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

/// 删除一个证据文件（真实路径必须在 data_dir 内；缺失/越界静默跳过）。
fn remove_file_counted(path: &str, cfg: &RetentionConfig, stats: &mut CleanupStats) {
    if path.is_empty() {
        return;
    }
    let p = Path::new(path);
    if !safe_within(p, &cfg.data_dir) {
        return;
    }
    let Ok(meta) = fs::metadata(p) else {
        return;
    };
    let size = meta.len();
    if fs::remove_file(p).is_ok() {
        stats.deleted_files += 1;
        stats.freed_mb += size as f64 / 1e6;
    }
}

fn referenced_media_paths(conn: &rusqlite::Connection) -> JobResult<HashSet<String>> {
    let mut stmt = conn
        .prepare(
            "SELECT snapshot FROM events WHERE snapshot IS NOT NULL \
             UNION SELECT video FROM events WHERE video IS NOT NULL \
             UNION SELECT snapshot FROM folder_images WHERE snapshot IS NOT NULL",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = HashSet::new();
    for row in rows {
        let p = row.map_err(|e| e.to_string())?;
        if !p.is_empty() {
            out.insert(norm_key(Path::new(&p)));
        }
    }
    Ok(out)
}

/// 无引用且已过宽限期的媒体文件（不递归；只扫 caption/video 两个目录）。
fn iter_orphan_media(cfg: &RetentionConfig, referenced: &HashSet<String>) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let now = SystemTime::now();
    for dir in [&cfg.snapshot_dir, &cfg.video_dir] {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let p = entry.path();
            let ext = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.to_ascii_lowercase())
                .unwrap_or_default();
            if !ORPHAN_EXTS.contains(&ext.as_str()) {
                continue;
            }
            if referenced.contains(&norm_key(&p)) {
                continue;
            }
            let age_s = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if age_s < ORPHAN_GRACE_S {
                continue; // 新文件/可能仍在写入
            }
            if !safe_within(&p, &cfg.data_dir) {
                continue;
            }
            out.push((p, meta.len()));
        }
    }
    out
}

fn pool_file_count(cfg: &RetentionConfig) -> (usize, u64) {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(
            (cfg.retention_pool_days.max(0) as u64) * 86400,
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut files = 0;
    let mut bytes = 0;
    if let Ok(entries) = fs::read_dir(&cfg.annotation_pool_dir) {
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let old = meta.modified().map(|m| m < cutoff).unwrap_or(false);
            if old {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    (files, bytes)
}

/// 只读预演（任务68）：与 run_cleanup 同一套阈值/路径判断，**绝不删除任何东西**。
pub fn preview_cleanup(store: &Store, cfg: &RetentionConfig) -> JobResult<Value> {
    preview_cleanup_with(store, cfg, &disk_free_gb)
}

pub fn preview_cleanup_with(
    store: &Store,
    cfg: &RetentionConfig,
    free_gb: &dyn Fn(&Path) -> f64,
) -> JobResult<Value> {
    let conn = store.conn();
    let cutoff = cutoff_iso(cfg.retention_events_days);
    let mut ev_count = 0usize;
    let mut ev_bytes = 0u64;
    let mut last_id = 0i64;
    loop {
        let rows = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, snapshot, video FROM events WHERE ts < ? AND id > ? \
                     ORDER BY id ASC LIMIT ?",
                )
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map(rusqlite::params![cutoff, last_id, BATCH as i64], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            it.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
        };
        if rows.is_empty() {
            break;
        }
        for (id, snap, vid) in &rows {
            last_id = *id;
            ev_count += 1;
            for p in [snap, vid].into_iter().flatten() {
                if p.is_empty() {
                    continue;
                }
                let path = Path::new(p);
                if safe_within(path, &cfg.data_dir) {
                    if let Ok(md) = fs::metadata(path) {
                        ev_bytes += md.len();
                    }
                }
            }
        }
        if rows.len() < BATCH {
            break;
        }
    }
    let (pool_files, pool_bytes) = pool_file_count(cfg);
    let audit_rows = if table_exists(conn, "audit_log") {
        conn.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE ts < ?",
            [&cutoff],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
    } else {
        0
    };
    let img_cutoff = cutoff_iso(cfg.retention_images_days);
    let img_rows = if table_exists(conn, "folder_images") {
        conn.query_row(
            "SELECT COUNT(*) FROM folder_images WHERE processed_ts < ?",
            [&img_cutoff],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
    } else {
        0
    };
    let (orph_files, orph_bytes) = match referenced_media_paths(conn) {
        Ok(refs) => {
            let items = iter_orphan_media(cfg, &refs);
            (items.len(), items.iter().map(|(_, s)| *s).sum::<u64>())
        }
        Err(_) => (0, 0),
    };
    let free = free_gb(&cfg.data_dir);
    Ok(json!({
        "cutoff": cutoff,
        "events": {"count": ev_count, "bytes": ev_bytes},
        "pool": {"files": pool_files, "bytes": pool_bytes},
        "audit": {"count": audit_rows},
        "images": {"count": img_rows, "cutoff": img_cutoff},
        "orphans": {"files": orph_files, "bytes": orph_bytes},
        "disk_free_gb": round2(free),
        "estimated_free_gb": round2(free + (ev_bytes + pool_bytes + orph_bytes) as f64 / 1e9),
        "would_emergency_clean": free < cfg.disk_min_free_gb,
    }))
}

/// 紧急清理档③行（id, status, level, snapshot, video）
type EmergencyRow = (i64, String, i64, Option<String>, Option<String>);

/// 执行一轮清理（幂等可重入）；返回统计。
pub fn run_cleanup(store: &Store, cfg: &RetentionConfig) -> JobResult<CleanupStats> {
    run_cleanup_with(store, cfg, &disk_free_gb)
}

pub fn run_cleanup_with(
    store: &Store,
    cfg: &RetentionConfig,
    free_gb: &dyn Fn(&Path) -> f64,
) -> JobResult<CleanupStats> {
    let conn = store.conn();
    let mut stats = CleanupStats::default();
    let cutoff = cutoff_iso(cfg.retention_events_days);

    // 1) 过期事件：先删文件再删行
    loop {
        let rows = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, snapshot, video FROM events WHERE ts < ? ORDER BY id ASC LIMIT ?",
                )
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map(rusqlite::params![cutoff, BATCH as i64], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            it.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
        };
        if rows.is_empty() {
            break;
        }
        for (id, snap, vid) in &rows {
            for p in [snap, vid].into_iter().flatten() {
                remove_file_counted(p, cfg, &mut stats);
            }
            conn.execute("DELETE FROM events WHERE id=?", [id])
                .map_err(|e| e.to_string())?;
            stats.deleted_events += 1;
        }
        if rows.len() < BATCH {
            break;
        }
    }

    // 2) 误报标注池过期文件（按修改时间）
    let pool_cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(
            (cfg.retention_pool_days.max(0) as u64) * 86400,
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if let Ok(entries) = fs::read_dir(&cfg.annotation_pool_dir) {
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if !meta.is_file() || meta.modified().map(|m| m >= pool_cutoff).unwrap_or(true) {
                continue;
            }
            let size = meta.len();
            if fs::remove_file(e.path()).is_ok() {
                stats.deleted_files += 1;
                stats.freed_mb += size as f64 / 1e6;
            }
        }
    }

    // 3) 审计日志与投递记录（旧库缺表时跳过）
    if table_exists(conn, "audit_log") {
        stats.deleted_audit = conn
            .execute("DELETE FROM audit_log WHERE ts < ?", [&cutoff])
            .unwrap_or(0);
    }
    if table_exists(conn, "alert_deliveries") {
        stats.deleted_deliveries = conn
            .execute(
                "DELETE FROM alert_deliveries WHERE updated_ts < ?",
                [&cutoff],
            )
            .unwrap_or(0);
    }

    // 3.6) 图片文件夹识别记录：只删派生证据副本与缩略图
    if table_exists(conn, "folder_images") {
        let img_cutoff = cutoff_iso(cfg.retention_images_days);
        loop {
            let rows = {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, snapshot FROM folder_images WHERE processed_ts < ? \
                         ORDER BY id LIMIT ?",
                    )
                    .map_err(|e| e.to_string())?;
                let it = stmt
                    .query_map(rusqlite::params![img_cutoff, BATCH as i64], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
                    })
                    .map_err(|e| e.to_string())?;
                it.collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|e| e.to_string())?
            };
            if rows.is_empty() {
                break;
            }
            for (id, snap) in &rows {
                if let Some(p) = snap.as_deref().filter(|p| !p.is_empty()) {
                    if safe_within(Path::new(p), &cfg.data_dir) {
                        let referenced = conn
                            .query_row("SELECT 1 FROM events WHERE snapshot=? LIMIT 1", [p], |_| {
                                Ok(())
                            })
                            .is_ok();
                        if !referenced {
                            remove_file_counted(p, cfg, &mut stats);
                        }
                    }
                }
                let thumb = cfg.data_dir.join("thumbs").join(format!("img_{id}.jpg"));
                let _ = fs::remove_file(thumb);
                conn.execute("DELETE FROM folder_images WHERE id=?", [id])
                    .map_err(|e| e.to_string())?;
                stats.deleted_images += 1;
            }
            if rows.len() < BATCH {
                break;
            }
        }
    }

    // 3.7) 孤儿媒体
    if let Ok(refs) = referenced_media_paths(conn) {
        for (p, size) in iter_orphan_media(cfg, &refs) {
            if stats.deleted_orphans >= ORPHAN_MAX_PER_RUN {
                stats.orphans_limited = true;
                break;
            }
            match fs::remove_file(&p) {
                Ok(()) => {
                    stats.deleted_orphans += 1;
                    stats.freed_orphan_mb += size as f64 / 1e6;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
        }
    }
    stats.freed_orphan_mb = round3(stats.freed_orphan_mb);

    // 4) 磁盘紧急清理三档
    let mut rounds = 0;
    let mut warned_unhandled = false;
    while free_gb(&cfg.data_dir) < cfg.disk_min_free_gb && rounds < 200 {
        let vid_rows = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, video FROM events WHERE status!='new' AND video IS NOT NULL \
                     ORDER BY id ASC LIMIT ?",
                )
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map([BATCH as i64], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .map_err(|e| e.to_string())?;
            it.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
        };
        let mut snap_rows: Vec<(i64, Option<String>)> = Vec::new();
        let mut row_rows: Vec<EmergencyRow> = Vec::new();
        if vid_rows.is_empty() {
            let mut stmt = conn
                .prepare(
                    "SELECT id, snapshot FROM events WHERE status!='new' AND snapshot IS NOT NULL \
                     ORDER BY id ASC LIMIT ?",
                )
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map([BATCH as i64], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .map_err(|e| e.to_string())?;
            snap_rows = it
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
        }
        if vid_rows.is_empty() && snap_rows.is_empty() {
            let mut stmt = conn
                .prepare(
                    "SELECT id, COALESCE(status,'new'), level, snapshot, video FROM events \
                     ORDER BY level ASC, id ASC LIMIT ?",
                )
                .map_err(|e| e.to_string())?;
            let it = stmt
                .query_map([BATCH as i64], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            row_rows = it
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
        }
        if vid_rows.is_empty() && snap_rows.is_empty() && row_rows.is_empty() {
            println!("[retention] 磁盘仍不足但已无可删目标——请人工介入");
            break;
        }
        for (id, video) in &vid_rows {
            if let Some(p) = video {
                remove_file_counted(p, cfg, &mut stats);
            }
            conn.execute("UPDATE events SET video=NULL WHERE id=?", [id])
                .map_err(|e| e.to_string())?;
        }
        for (id, snap) in &snap_rows {
            if let Some(p) = snap {
                remove_file_counted(p, cfg, &mut stats);
            }
            conn.execute("UPDATE events SET snapshot=NULL WHERE id=?", [id])
                .map_err(|e| e.to_string())?;
        }
        for (id, status, _level, snap, video) in &row_rows {
            for p in [snap, video].into_iter().flatten() {
                remove_file_counted(p, cfg, &mut stats);
            }
            conn.execute("DELETE FROM events WHERE id=?", [id])
                .map_err(|e| e.to_string())?;
            stats.deleted_events += 1;
            if status == "new" && !warned_unhandled {
                warned_unhandled = true;
                println!("[retention] [ERROR] 磁盘不足导致未处置事件（id={id}）被清理");
            }
        }
        rounds += 1;
        println!(
            "[retention] 紧急清理第 {rounds} 批，剩余磁盘 {:.1}GB",
            free_gb(&cfg.data_dir)
        );
    }

    stats.ts = now_iso();
    stats.disk_free_gb = round2(free_gb(&cfg.data_dir));
    Ok(stats)
}

/// 维护状态（Python `/api/maintenance/status` 等价）：清理结果 + 磁盘水位 + 体量/增速估算。
pub fn maintenance_status(
    store: &Store,
    cfg: &RetentionConfig,
    last: Option<&CleanupStats>,
) -> JobResult<Value> {
    let conn = store.conn();
    let (data_bytes, capped) = data_bytes_capped(&cfg.data_dir, 20_000);
    let free = disk_free_gb(&cfg.data_dir);
    let total = disk_total_gb(&cfg.data_dir);
    let free_pct = total.filter(|t| *t > 0.0).map(|t| round1(free / t * 100.0));
    let week_cutoff = (Local::now() - chrono::TimeDelta::days(7))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let mut week_bytes = 0u64;
    if let Ok(mut stmt) =
        conn.prepare("SELECT snapshot, video FROM events WHERE ts >= ? LIMIT 5000")
    {
        if let Ok(rows) = stmt.query_map([&week_cutoff], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        }) {
            for row in rows.flatten() {
                for p in [row.0, row.1].into_iter().flatten() {
                    if let Ok(md) = fs::metadata(&p) {
                        if md.is_file() {
                            week_bytes += md.len();
                        }
                    }
                }
            }
        }
    }
    let forecast_days = if week_bytes > 0 {
        let per_day_gb = (week_bytes as f64 / 7.0) / 1e9;
        if per_day_gb > 0.0 {
            Some((free / per_day_gb).round() as i64)
        } else {
            None
        }
    } else {
        None
    };
    Ok(json!({
        "last_cleanup": last.map(|s| serde_json::to_value(s).unwrap_or(json!({}))).unwrap_or(json!({})),
        "disk_free_gb": round2(free),
        "disk_total_gb": total.map(round2),
        "disk_free_pct": free_pct,
        "data_bytes": data_bytes,
        "data_bytes_capped": capped,
        "low_disk": free < cfg.disk_min_free_gb,
        "forecast_days": forecast_days,
        "data_dir": cfg.data_dir.to_string_lossy(),
    }))
}

fn data_bytes_capped(dir: &Path, limit: usize) -> (u64, bool) {
    fn walk(dir: &Path, total: &mut u64, n: &mut usize, limit: usize) -> bool {
        let Ok(entries) = fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if walk(&entry.path(), total, n, limit) {
                    return true;
                }
            } else if meta.is_file() {
                *n += 1;
                if *n > limit {
                    return true;
                }
                *total += meta.len();
            }
        }
        false
    }
    let mut total = 0;
    let mut n = 0;
    let capped = walk(dir, &mut total, &mut n, limit);
    (total, capped)
}

/// 四核心目录递归大小（db + snapshots + videos + annotation_pool）。
pub fn core_bytes(cfg: &RetentionConfig) -> u64 {
    let db = cfg.data_dir.join("labsentinel.db");
    let mut total = fs::metadata(db).map(|m| m.len()).unwrap_or(0);
    for d in [&cfg.snapshot_dir, &cfg.video_dir, &cfg.annotation_pool_dir] {
        total += crate::paths::dir_bytes(d);
    }
    total
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}
fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}
