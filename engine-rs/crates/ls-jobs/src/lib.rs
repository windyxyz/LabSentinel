//! ls-jobs（L2/T-330）：后台作业的 Rust 实现与调度器。
//!
//! 三个作业与 Python 服务同口径（同一 SQLite、同一数据目录、同一环境变量）：
//! - retention：每 6h 清理（[`retention`]）
//! - backup：按 LS_BACKUP_INTERVAL_H 自动热备份，保留 LS_BACKUP_KEEP 份（[`backup`]）
//! - escalation：每 60s 检查超时未处置事件并提醒一次（[`escalation`]）
//!
//! 调度线程与 Python 一致：启动即跑首轮，之后按周期；`stop()` 后 ≤200ms 退出。
//! 发送侧复用 [`ls_dispatch`]：静默 / 渠道过滤(LS_ALERT_FILTERS) / 限流 / 投递记录 / 重试。

pub mod backup;
pub mod escalation;
pub mod paths;
pub mod retention;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use chrono::Local;
use ls_store::Store;
use serde_json::{json, Value};

pub use backup::{BackupConfig, BackupOutcome};
pub use escalation::{EscalationConfig, EscalationOutcome, EscalationRow};
pub use retention::{CleanupStats, RetentionConfig};

pub type JobResult<T> = Result<T, String>;

const RETENTION_INTERVAL_S: u64 = 6 * 3600;
const ESCALATION_INTERVAL_S: u64 = 60;
const BACKUP_CHECK_MAX_S: u64 = 3600;

#[derive(Clone, Debug)]
pub struct JobsConfig {
    pub retention: RetentionConfig,
    pub backup: BackupConfig,
    pub escalation: EscalationConfig,
}

impl JobsConfig {
    /// 默认配置（测试/嵌入式使用，不读环境变量）。
    pub fn default_for_db(db_path: &Path) -> Self {
        let data_dir = db_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        Self {
            retention: RetentionConfig::for_data_dir(&data_dir),
            backup: BackupConfig::for_data_dir(&data_dir),
            escalation: EscalationConfig::default(),
        }
    }

    /// 生产配置：环境变量与 Python settings 同名（LS_ 前缀），数据目录取数据库所在目录。
    pub fn from_env(db_path: &Path) -> Self {
        let mut cfg = Self::default_for_db(db_path);
        cfg.retention.retention_events_days = env_i64(
            "LS_RETENTION_EVENTS_DAYS",
            cfg.retention.retention_events_days,
        );
        cfg.retention.retention_pool_days =
            env_i64("LS_RETENTION_POOL_DAYS", cfg.retention.retention_pool_days);
        cfg.retention.retention_images_days = env_i64(
            "LS_RETENTION_IMAGES_DAYS",
            cfg.retention.retention_images_days,
        );
        cfg.retention.disk_min_free_gb =
            env_f64("LS_DISK_MIN_FREE_GB", cfg.retention.disk_min_free_gb);
        cfg.backup.interval_h = env_i64("LS_BACKUP_INTERVAL_H", cfg.backup.interval_h);
        cfg.backup.keep = env_i64("LS_BACKUP_KEEP", cfg.backup.keep as i64).max(1) as usize;
        cfg.backup.include_media = env_bool("LS_BACKUP_INCLUDE_MEDIA", cfg.backup.include_media);
        cfg.escalation.after_min = env_i64("LS_ESCALATE_AFTER_MIN", cfg.escalation.after_min);
        cfg.escalation.min_level = env_i64("LS_ESCALATE_MIN_LEVEL", cfg.escalation.min_level);
        cfg.escalation.batch = env_i64("LS_ESCALATE_BATCH", cfg.escalation.batch);
        if let Some(t) = env_raw("LS_ALERT_TEMPLATE_TEXT") {
            cfg.escalation.template_text = t;
        }
        cfg
    }
}

fn env_raw(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}
fn env_i64(key: &str, default: i64) -> i64 {
    env_raw(key)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}
fn env_f64(key: &str, default: f64) -> f64 {
    env_raw(key)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}
fn env_bool(key: &str, default: bool) -> bool {
    match env_raw(key) {
        Some(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        None => default,
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Jobs {
    cfg: JobsConfig,
    db_path: PathBuf,
    stop: AtomicBool,
    retention_last: Mutex<Option<CleanupStats>>,
    backup_last: Mutex<Option<BackupOutcome>>,
    backup_running: AtomicBool,
    escalation_last: Mutex<Option<EscalationOutcome>>,
    rate: Mutex<ls_dispatch::RateLimiter>,
    handles: Mutex<Vec<JoinHandle<()>>>,
}

impl Jobs {
    pub fn new(db_path: PathBuf, cfg: JobsConfig) -> Self {
        Self {
            cfg,
            db_path,
            stop: AtomicBool::new(false),
            retention_last: Mutex::new(None),
            backup_last: Mutex::new(None),
            backup_running: AtomicBool::new(false),
            escalation_last: Mutex::new(None),
            rate: Mutex::new(ls_dispatch::RateLimiter::new()),
            handles: Mutex::new(Vec::new()),
        }
    }

    pub fn config(&self) -> &JobsConfig {
        &self.cfg
    }

    fn store(&self) -> JobResult<Store> {
        Store::open(&self.db_path.to_string_lossy()).map_err(|e| e.to_string())
    }

    // ---- retention ----
    pub fn run_cleanup(&self) -> JobResult<CleanupStats> {
        let store = self.store()?;
        let stats = retention::run_cleanup(&store, &self.cfg.retention)?;
        println!(
            "[retention] 清理完成: {}",
            serde_json::to_string(&stats).unwrap_or_default()
        );
        *lock(&self.retention_last) = Some(stats.clone());
        Ok(stats)
    }

    pub fn retention_last(&self) -> Option<CleanupStats> {
        lock(&self.retention_last).clone()
    }

    pub fn preview_cleanup(&self) -> JobResult<Value> {
        let store = self.store()?;
        retention::preview_cleanup(&store, &self.cfg.retention)
    }

    pub fn maintenance_status(&self) -> JobResult<Value> {
        let store = self.store()?;
        let last = self.retention_last();
        retention::maintenance_status(&store, &self.cfg.retention, last.as_ref())
    }

    // ---- backup ----
    pub fn backup_status(&self) -> Value {
        backup::status_json(&self.cfg.backup, self.backup_running.load(Ordering::SeqCst))
    }

    pub fn backup_last(&self) -> Option<BackupOutcome> {
        lock(&self.backup_last).clone()
    }

    /// 原子触发（手动/定时共用）；已在跑返回 false（API 据此 409）。
    pub fn trigger_backup(self: &Arc<Self>) -> bool {
        if self
            .backup_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let me = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("backup-trigger".into())
            .spawn(move || {
                let outcome = backup::run_auto_backup(&me.cfg.backup);
                match outcome {
                    Ok(o) => {
                        println!("[auto-backup] ✓ {}", o.message());
                        *lock(&me.backup_last) = Some(o);
                    }
                    Err(e) => println!("[auto-backup] ✗ 备份异常: {e}"),
                }
                me.backup_running.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.backup_running.store(false, Ordering::SeqCst);
            return false;
        }
        true
    }

    fn backup_tick(self: &Arc<Self>) {
        if self.cfg.backup.interval_h <= 0 {
            return;
        }
        let files = backup::auto_files(&self.cfg.backup);
        if backup::needs_backup(&files, SystemTime::now(), self.cfg.backup.interval_h)
            && self.trigger_backup()
        {
            println!("[auto-backup] 已触发");
        }
    }

    // ---- escalation ----
    pub fn escalation_status(&self) -> JobResult<Value> {
        let store = self.store()?;
        let last = lock(&self.escalation_last).clone();
        escalation::status_json(&store, &self.cfg.escalation, last.as_ref())
    }

    pub fn escalate_once(&self) -> JobResult<EscalationOutcome> {
        let store = self.store()?;
        let channels = ls_dispatch::channels_from_env();
        let filters = parse_alert_filters();
        let silences = active_silences(&store);
        let now_iso = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let sender = |row: &EscalationRow, text: &str| -> bool {
            let ev = ls_dispatch::DispatchEvent {
                id: row.id,
                camera_id: row.camera_id,
                type_: row.type_.clone(),
                level: row.level,
                detail: row.detail.clone(),
                ts: row.ts.clone(),
                camera_name: String::new(),
            };
            if let Some(idx) = ls_dispatch::match_silence(&silences, &ev, &now_iso) {
                println!(
                    "[alert] event#{} 升级提醒命中静默 #{}——外部渠道不推送",
                    row.id, silences[idx].target
                );
                return false;
            }
            let mut submitted = 0;
            for (ch, ch_cfg) in &channels {
                if let Some(reason) = ls_dispatch::channel_filtered(*ch, &ev, &filters) {
                    println!(
                        "[alert] event#{} 渠道 {} 被过滤（{reason}）——不建投递行",
                        row.id,
                        ch.name()
                    );
                    continue;
                }
                let (allowed, pending) = lock(&self.rate).gate(*ch);
                if !allowed {
                    continue; // 超限：计数留待下一窗口摘要
                }
                let mut send_text = text.to_string();
                if pending > 0 {
                    send_text.push_str(&format!("\n（上一窗口合并 {pending} 条）"));
                }
                let did = store.add_delivery(row.id, ch.name()).ok();
                let d = ls_dispatch::deliver(*ch, ch_cfg, &send_text);
                if let Some(id) = did {
                    let _ = store.update_delivery(
                        id,
                        if d.ok { "ok" } else { "failed" },
                        d.attempts as i64,
                        d.error.as_deref(),
                    );
                }
                if d.ok {
                    submitted += 1;
                }
            }
            submitted > 0
        };
        let outcome = escalation::run_once(&store, &self.cfg.escalation, None, Some(&sender))?;
        if outcome.disabled != Some(true) {
            *lock(&self.escalation_last) = Some(outcome.clone());
        }
        Ok(outcome)
    }

    // ---- scheduler ----
    /// 启动三个后台线程（幂等）。首轮立即执行，之后按周期。
    pub fn start(self: &Arc<Self>) {
        let mut handles = lock(&self.handles);
        if !handles.is_empty() {
            return;
        }
        self.stop.store(false, Ordering::SeqCst);
        handles.push(spawn_loop(
            "retention",
            Arc::clone(self),
            RETENTION_INTERVAL_S,
            |j| {
                let _ = j.run_cleanup();
            },
        ));
        let backup_check = if self.cfg.backup.interval_h > 0 {
            BACKUP_CHECK_MAX_S
                .min(60.max(self.cfg.backup.interval_h.saturating_mul(3600) / 4) as u64)
        } else {
            BACKUP_CHECK_MAX_S
        };
        handles.push(spawn_loop(
            "auto-backup",
            Arc::clone(self),
            backup_check,
            |j| j.backup_tick(),
        ));
        handles.push(spawn_loop(
            "escalation",
            Arc::clone(self),
            ESCALATION_INTERVAL_S,
            |j| {
                let _ = j.escalate_once();
            },
        ));
        println!(
            "[jobs] 后台作业已启动（retention 每 {}h / backup 每 {}h 保留 {} 份 / escalation 每 {}s）",
            RETENTION_INTERVAL_S / 3600,
            self.cfg.backup.interval_h,
            self.cfg.backup.keep,
            ESCALATION_INTERVAL_S
        );
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let handles = std::mem::take(&mut *lock(&self.handles));
        for h in handles {
            let _ = h.join(); // 线程每 200ms 检查停止标志，正常 ≤200ms 返回
        }
    }
}

fn spawn_loop(
    name: &str,
    jobs: Arc<Jobs>,
    interval_s: u64,
    task: fn(&Arc<Jobs>),
) -> JoinHandle<()> {
    let thread_name = format!("ls-jobs-{name}");
    thread::Builder::new()
        .name(thread_name)
        .spawn(move || loop {
            if jobs.stop.load(Ordering::SeqCst) {
                return;
            }
            task(&jobs);
            let deadline = Instant::now() + Duration::from_secs(interval_s);
            while Instant::now() < deadline {
                if jobs.stop.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(Duration::from_millis(200));
            }
        })
        .expect("无法创建后台作业线程")
}

/// LS_ALERT_FILTERS 解析（Python `alert_filters()` 等价：非法条目忽略并告警）。
pub fn parse_alert_filters() -> HashMap<String, Value> {
    let Some(raw) = env_raw("LS_ALERT_FILTERS") else {
        return HashMap::new();
    };
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&raw) else {
        println!("[alert-filters] 配置非法（忽略全部）");
        return HashMap::new();
    };
    let known: Vec<&str> = ls_dispatch::Channel::all()
        .iter()
        .map(|c| c.name())
        .collect();
    let mut out = HashMap::new();
    for (name, f) in obj {
        if !known.contains(&name.as_str()) {
            println!("[alert-filters] 未知渠道 {name:?}（忽略该条）");
            continue;
        }
        let Some(f) = f.as_object() else {
            println!("[alert-filters] {name} 配置必须是对象（忽略该条）");
            continue;
        };
        let mut entry = serde_json::Map::new();
        if let Some(ml) = f.get("min_level") {
            match ml.as_i64() {
                Some(v) if (1..=3).contains(&v) => {
                    entry.insert("min_level".into(), json!(v));
                }
                _ => println!("[alert-filters] {name} min_level 非法（忽略该字段）: {ml}"),
            }
        }
        if let Some(types) = f.get("types") {
            match types.as_array() {
                Some(arr) if arr.iter().all(|x| x.is_string()) => {
                    entry.insert("types".into(), types.clone());
                }
                _ => println!("[alert-filters] {name} types 必须为字符串数组（忽略该字段）"),
            }
        }
        if !entry.is_empty() {
            out.insert(name, Value::Object(entry));
        }
    }
    out
}

/// 活跃静默（alert_silences 表，until_ts > now）。
fn active_silences(store: &Store) -> Vec<ls_dispatch::Silence> {
    let now = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let Ok(mut stmt) = store.conn().prepare(
        "SELECT COALESCE(scope,'global'), COALESCE(target,''), until_ts FROM alert_silences \
         WHERE until_ts > ?",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([now], |r| {
        Ok(ls_dispatch::Silence {
            scope: r.get(0)?,
            target: r.get(1)?,
            until_ts: r.get(2)?,
        })
    }) else {
        return Vec::new();
    };
    rows.flatten().collect()
}
