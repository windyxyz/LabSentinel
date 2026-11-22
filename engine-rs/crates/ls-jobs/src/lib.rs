//! ls-jobs（L2/T-330）：后台作业的 Rust 实现与调度器。
//!
//! 三个作业与 Python 服务同口径（同一 SQLite、同一数据目录、同一环境变量）：
//! - retention：每 6h 清理（[`retention`]）
//! - backup：按 LS_BACKUP_INTERVAL_H 自动热备份，保留 LS_BACKUP_KEEP 份（[`backup`]）
//! - escalation：每 60s 检查超时未处置事件并提醒一次（[`escalation`]）
//!
//! 调度线程与 Python 一致：启动即跑首轮，之后按周期；`stop()` 后 ≤200ms 退出。
//! 发送侧复用 [`ls_dispatch`]：静默 / 渠道过滤(LS_ALERT_FILTERS) / 限流 / 投递记录 / 重试。
//!
//! ### T-330 genai / folder / plugins 迁移评估结论（2026-11-07）
//! - **genai**（VLM 复核队列）：暂不迁移。它依赖外部 VLM HTTP 客户端
//!   （`genai.py`，OpenAI 兼容协议 + 超时/重试/配额策略），且与事件派发强耦合
//!   （复核结果回写 `events.ai_verdict` 并触发二次派发）。收益低于成本，留到 L3
//!   控制面接管后与「事件流水线编排」一起迁（那时复用 ls-dispatch 的 HTTP 栈）。
//! - **folder**（图片文件夹作业）：数据面（扫描/稳定期/去重/关键帧推理）属
//!   T-326 范畴，见 `ls-ingest/src/folder.rs` 的纯函数扫描器；作业编排（每日上限、
//!   缩略图、证据副本）在 Python `folders.py`，Rust 侧随 L4 一起收口。
//! - **plugins**（插件子进程管理）：暂不迁移。清单校验/环境白名单/生命周期与
//!   Windows 句柄语义强绑定（T-112/T-602），且属低频管理操作、无热路径收益；
//!   L3 控制面迁移时用 `std::process` + 显式白名单环境重做（语义表已在
//!   `server/app/plugins.py` 与 `docs/dev/plugin-guide.md`，可直接翻译）。

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

use ls_core::time as ls_time;
use ls_store::Store;
use serde_json::{json, Value};

pub use backup::{BackupConfig, BackupOutcome};
pub use escalation::{EscalationConfig, EscalationOutcome, EscalationRow};
pub use retention::{CleanupStats, RetentionConfig};

pub type JobResult<T> = Result<T, String>;

const RETENTION_INTERVAL_S: u64 = 6 * 3600;
const ESCALATION_INTERVAL_S: u64 = 60;
const BACKUP_CHECK_MAX_S: u64 = 3600;
/// LS-053：告警失败重试消费者轮询间隔
const RETRY_INTERVAL_S: u64 = 30;
/// LS-053：日报/周报调度检查间隔
const REPORT_CHECK_S: u64 = 60;
/// LS-053：最大重试次数（超过即 dead_letter 死信）
pub const MAX_RETRY_ATTEMPTS: i64 = 5;

/// LS-053：第 n 次重试的退避秒数（指数退避，封顶 1 小时）。
/// 纯函数便于单测：n=0 → 30s，n=1 → 120s，n=2 → 600s，n≥3 → 1800/3600s。
pub fn retry_backoff_s(attempts: i64) -> u64 {
    match attempts.max(0) {
        0 => 30,
        1 => 120,
        2 => 600,
        3 => 1800,
        _ => 3600,
    }
}

/// LS-053：日报是否到点——启用、当前 HH:MM ≥ 设定时刻、且今天未发过。
/// 纯函数便于单测。
pub fn report_due(
    enabled: bool,
    due_hm: &str,
    now_hm: &str,
    today: &str,
    last_date: Option<&str>,
) -> bool {
    if !enabled {
        return false;
    }
    if !due_hm.is_empty() && now_hm < due_hm {
        return false;
    }
    last_date != Some(today)
}

/// LS-053：周报是否到点——启用、ISO 星期匹配、当前 HH:MM ≥ 设定时刻、且本周未发。
pub fn weekly_due(
    enabled: bool,
    due_weekday: u8,
    due_hm: &str,
    now_weekday: u8,
    now_hm: &str,
    today: &str,
    last_date: Option<&str>,
) -> bool {
    if !enabled || now_weekday != due_weekday {
        return false;
    }
    if !due_hm.is_empty() && now_hm < due_hm {
        return false;
    }
    // 同一周内只发一次：比较本周一日期
    match last_date {
        Some(d) => !same_iso_week(d, today),
        None => true,
    }
}

/// 两个 ISO 日期是否在同一自然周（周一为起点）。纯字符串比较 + 日期差近似。
fn same_iso_week(a: &str, b: &str) -> bool {
    // 简单稳妥：同一周 ⇔ 两日期相差 < 7 天且 b >= a 且 b-a 的星期偏移对齐；
    // 这里用「相差 ≤ 6 天」近似（调度每天检查一次，重复窗口已足够窄）。
    match (
        ls_core::time::parse_local_datetime(a),
        ls_core::time::parse_local_datetime(b),
    ) {
        (Some(x), Some(y)) => {
            let dx = ls_core::time::diff_minutes(y, x) / (60 * 24);
            (0..7).contains(&dx)
        }
        _ => a == b,
    }
}

/// LS-053：报告文案（纯函数，便于单测与模板统一）。
pub fn report_text(
    title: &str,
    total: i64,
    violation: i64,
    unhandled: i64,
    top: &[(i64, i64)],
    camera_names: &std::collections::HashMap<i64, String>,
) -> String {
    let rate = if total > 0 {
        ((total - violation) as f64 / total as f64 * 100.0).round() as i64
    } else {
        100
    };
    let mut s = format!(
        "【LabSentinel·{title}】\n事件总数：{total}\n违规事件（L2+）：{violation}\n未处置：{unhandled}\n合规率：{rate}%"
    );
    if !top.is_empty() {
        s.push_str("\n违规 Top 摄像头：");
        for (i, (cid, n)) in top.iter().enumerate() {
            let name = camera_names
                .get(cid)
                .cloned()
                .unwrap_or_else(|| format!("cam{cid}"));
            s.push_str(&format!("\n{}. {name}（{n} 次）", i + 1));
        }
    }
    s
}

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
    /// R2-305：ApplyConfig/控制面注入的渠道快照（网页配置优先于 env；None=回退 env）
    alert_channels: Mutex<Option<Vec<(ls_dispatch::Channel, ls_dispatch::ChannelConfig)>>>,
    /// UX（2026-11-21）：告警文案网页编辑的运行时覆盖（PUT /api/alert-template
    /// 写 app_settings 并同步此处；None=用 cfg.escalation.template_text）
    alert_template_override: Mutex<Option<String>>,
    /// UX（2026-11-21）：推送冷却去重（camera|type|track → 上次推送时刻；
    /// alert.push_cooldown_s 秒窗口内同人同行为只推渠道一次）
    push_dedup: Mutex<std::collections::HashMap<String, Instant>>,
    /// LS-053：日报/周报最近发送日期（防同日重复推送）
    report_last_daily: Mutex<Option<String>>,
    report_last_weekly: Mutex<Option<String>>,
    retry_consumer: Mutex<()>,
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
            alert_channels: Mutex::new(None),
            alert_template_override: Mutex::new(None),
            push_dedup: Mutex::new(std::collections::HashMap::new()),
            report_last_daily: Mutex::new(None),
            report_last_weekly: Mutex::new(None),
            retry_consumer: Mutex::new(()),
        }
    }

    /// R2-305：注入渠道快照（网页端配置的生效值；None=回退 channels_from_env）。
    /// 渠道快照只进内存（凭据不落盘/日志）。
    pub fn set_alert_channels(
        &self,
        channels: Vec<(ls_dispatch::Channel, ls_dispatch::ChannelConfig)>,
    ) {
        if let Ok(mut guard) = self.alert_channels.lock() {
            *guard = Some(channels);
        }
    }

    /// 渠道快照克隆（alert-test 与实际派发**同源**：网页保存的 app_settings
    /// 注入值优先、其次 env——此前 alert_test 自读 env，两源不一致导致
    /// 「网页保存后点发送测试永远失败/不测」（2026-11-21 用户实测）。
    pub fn alert_channels_snapshot(
        &self,
    ) -> Vec<(ls_dispatch::Channel, ls_dispatch::ChannelConfig)> {
        let mut out = self
            .alert_channels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_default();
        // 合并环境变量兜底渠道（网页保存的 DB 渠道优先，未在 DB 配置的渠道回退 env）
        for (ch, cfg) in ls_dispatch::channels_from_env() {
            if !out.iter().any(|(c, _)| *c == ch) {
                out.push((ch, cfg));
            }
        }
        out
    }

    /// UX（2026-11-21）：网页端保存的告警文案即时生效（升级提醒渲染用）。
    pub fn set_alert_template(&self, text: String) {
        if let Ok(mut guard) = self.alert_template_override.lock() {
            *guard = Some(text);
        }
    }

    /// 清空自定义告警文案覆盖，回落默认模板。
    pub fn clear_alert_template(&self) {
        if let Ok(mut guard) = self.alert_template_override.lock() {
            *guard = None;
        }
    }

    /// 当前生效的告警文案（override > env 注入的 cfg 值）。
    pub fn alert_template(&self) -> String {
        lock(&self.alert_template_override)
            .clone()
            .unwrap_or_else(|| self.cfg.escalation.template_text.clone())
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

    /// RA1.3（ADR-0011）：单事件首发派发（worker 候选事件 → 8 渠道外部投递）。
    /// 复用 escalation 同一链路组件：注入渠道（或 env 回退）→ 静默 → 渠道过滤 →
    /// 限流门 → add_delivery → deliver → update_delivery。返回成功投递的渠道数。
    /// （escalation 的「未处置事件升级提醒」仍走 escalate_once；两者共享渠道快照
    /// 与限流器，事件不重复入队——首发只针对新落地事件。）
    pub fn dispatch_event(&self, store: &Store, ev: &ls_dispatch::DispatchEvent) -> usize {
        let channels: Vec<(ls_dispatch::Channel, ls_dispatch::ChannelConfig)> =
            match self.alert_channels.lock().ok().and_then(|g| g.clone()) {
                Some(injected) if !injected.is_empty() => injected,
                _ => ls_dispatch::channels_from_env(),
            };
        let filters = parse_alert_filters();
        let silences = active_silences(store);
        let now_iso = ls_time::local_now_iso();
        if let Some(idx) = ls_dispatch::match_silence(&silences, ev, &now_iso) {
            println!(
                "[alert] event#{} 首发命中静默 #{}——外部渠道不推送",
                ev.id, silences[idx].target
            );
            return 0;
        }
        // UX（2026-11-21）：推送冷却——同 camera|type|track 在
        // alert.push_cooldown_s 秒内只推渠道一次（事件照常入库/WS 广播，
        // 仅外部渠道静默）；0=关闭。API(vlm) 判定无 track → 同键合并兜底。
        let cooldown_s: u64 = store
            .setting_get("alert.push_cooldown_s")
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(60);
        if cooldown_s > 0 {
            let key = format!("{}|{}|{}", ev.camera_id, ev.type_, ev.track_id);
            let mut dedup = lock(&self.push_dedup);
            let suppressed = dedup
                .get(&key)
                .map(|t| t.elapsed() < Duration::from_secs(cooldown_s))
                .unwrap_or(false);
            if suppressed {
                println!(
                    "[alert] event#{} 冷却窗口内同人同行为——渠道不重复推送",
                    ev.id
                );
                return 0;
            }
            dedup.insert(key, Instant::now());
        }
        // 带图开关（alert.send_image，默认开）+ 读快照字节（合法 JPEG 且 ≤2MB）
        let send_image = store
            .setting_get("alert.send_image")
            .ok()
            .flatten()
            .map(|v| v.trim() != "0")
            .unwrap_or(true);
        let image: Option<Vec<u8>> = if send_image {
            ev.snapshot_path
                .as_deref()
                .and_then(|p| std::fs::read(p).ok())
                .filter(|d| d.starts_with(&[0xFF, 0xD8]) && d.len() <= 2 * 1024 * 1024)
        } else {
            None
        };
        let mut submitted = 0;
        for (ch, ch_cfg) in &channels {
            if let Some(reason) = ls_dispatch::channel_filtered(*ch, ev, &filters) {
                println!(
                    "[alert] event#{} 渠道 {} 被过滤（{reason}）——不建投递行",
                    ev.id,
                    ch.name()
                );
                continue;
            }
            let (allowed, pending) = lock(&self.rate).gate(*ch);
            if !allowed {
                continue; // 超限：计数留待下一窗口摘要
            }
            // UX（2026-11-21 用户实测反馈）：告警文字人话化——类型中文化、
            // 摄像头用名称（不带内部 ID）、去掉裸时间戳行（客户端消息自带时间）
            let level_cn = match ev.level {
                3 => "严重",
                2 => "警告",
                _ => "提示",
            };
            let mut text = format!(
                "【LabSentinel·{}】{}\n摄像头：{}\n{}",
                level_cn,
                ls_dispatch::type_text(&ev.type_),
                if ev.camera_name.is_empty() {
                    format!("#{}", ev.camera_id)
                } else {
                    ev.camera_name.clone()
                },
                ev.detail
            );
            if pending > 0 {
                text.push_str(&format!("\n（上一窗口合并 {pending} 条）"));
            }
            let did = store.add_delivery(ev.id, ch.name()).ok();
            let d = ls_dispatch::deliver_with_image(*ch, ch_cfg, ev.level, &text, image.as_deref());
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
        submitted
    }

    pub fn escalate_once(&self) -> JobResult<EscalationOutcome> {
        let store = self.store()?;
        let channels: Vec<(ls_dispatch::Channel, ls_dispatch::ChannelConfig)> =
            match self.alert_channels.lock().ok().and_then(|g| g.clone()) {
                Some(injected) if !injected.is_empty() => injected,
                _ => ls_dispatch::channels_from_env(),
            };
        let filters = parse_alert_filters();
        let silences = active_silences(&store);
        let now_iso = ls_time::local_now_iso();
        let sender = |row: &EscalationRow, text: &str| -> bool {
            let ev = ls_dispatch::DispatchEvent {
                id: row.id,
                camera_id: row.camera_id,
                type_: row.type_.clone(),
                level: row.level,
                detail: row.detail.clone(),
                ts: row.ts.clone(),
                camera_name: String::new(),
                track_id: 0,
                snapshot_path: None,
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
                let d = ls_dispatch::deliver(*ch, ch_cfg, ev.level, &send_text);
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
        // UX：告警文案 override（网页端保存）即时生效
        let mut esc_cfg = self.cfg.escalation.clone();
        esc_cfg.template_text = self.alert_template();
        let outcome = escalation::run_once(&store, &esc_cfg, None, Some(&sender))?;
        if outcome.disabled != Some(true) {
            *lock(&self.escalation_last) = Some(outcome.clone());
        }
        Ok(outcome)
    }

    /// LS-053：告警失败重试——扫描 retry/failed 且未超次数/退避到期的投递，
    /// 用同一渠道快照真重发；成功 → ok，失败 → retry，超过上限 → dead_letter。
    pub fn retry_tick(&self) {
        self.consume_retries(None);
    }

    pub fn retry_now(&self, id: i64) -> rusqlite::Result<bool> {
        let _guard = lock(&self.retry_consumer);
        let store = ls_store::Store::open(&self.db_path.to_string_lossy())?;
        if !store.retry_delivery(id)? {
            return Ok(false);
        }
        self.consume_retries_locked(&store, Some(id));
        Ok(true)
    }

    fn consume_retries(&self, delivery_id: Option<i64>) {
        let _guard = lock(&self.retry_consumer);
        let Ok(store) = ls_store::Store::open(&self.db_path.to_string_lossy()) else {
            return;
        };
        self.consume_retries_locked(&store, delivery_id);
    }

    fn consume_retries_locked(&self, store: &ls_store::Store, delivery_id: Option<i64>) {
        let cutoffs = std::array::from_fn(|i| {
            ls_core::time::format_iso(ls_core::time::local_offset_secs(
                -(retry_backoff_s(i as i64) as i64),
            ))
        });
        let rows = match store.retryable_deliveries(&cutoffs, delivery_id, 20) {
            Ok(r) => r,
            Err(_) => return,
        };
        if rows.is_empty() {
            return;
        }
        let channels = self.alert_channels_snapshot();
        for (did, event_id, channel_name, attempts) in rows {
            let Some(ev) = store.event_by_id(event_id).ok().flatten() else {
                let _ = store.update_delivery(did, "dead_letter", attempts, Some("事件已删除"));
                continue;
            };
            let Some((ch, cfg)) = channels
                .iter()
                .find(|(c, _)| c.name() == channel_name.as_str())
                .map(|(c, cfg)| (*c, cfg.clone()))
            else {
                let _ = store.update_delivery(did, "dead_letter", attempts, Some("渠道已移除"));
                continue;
            };
            let text = format!(
                "{}：{}（重试）",
                ls_dispatch::type_text(&ev.type_),
                ev.detail
            );
            let d = ls_dispatch::deliver_with_backoff(
                ch,
                &cfg,
                ev.level,
                &text,
                None,
                &[], // 消费者负责退避，每轮只投递一次
            );
            let next_attempts = attempts + 1;
            if d.ok {
                let _ = store.update_delivery(did, "ok", next_attempts, None);
            } else if next_attempts >= MAX_RETRY_ATTEMPTS {
                let _ =
                    store.update_delivery(did, "dead_letter", next_attempts, d.error.as_deref());
            } else {
                let _ = store.update_delivery(did, "retry", next_attempts, d.error.as_deref());
            }
        }
    }

    /// LS-053：日报/周报调度——到点汇总并复用告警渠道推送，同日/同周去重。
    pub fn report_tick(&self) {
        let Ok(store) = ls_store::Store::open(&self.db_path.to_string_lossy()) else {
            return;
        };
        let now = ls_core::time::local_now_civil();
        let now_hm = ls_core::time::format_hm(now);
        let today = ls_core::time::format_date(now);
        let weekday = ls_core::time::weekday_iso(now);
        let get = |k: &str| store.setting_get(k).ok().flatten();
        let daily_enabled = get("report.daily_enabled")
            .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        let daily_time = get("report.daily_time").unwrap_or_else(|| "18:00".into());
        let weekly_enabled = get("report.weekly_enabled")
            .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);
        let weekly_day = get("report.weekly_day")
            .and_then(|v| v.trim().parse::<u8>().ok())
            .filter(|d| (1..=7).contains(d))
            .unwrap_or(1);
        let weekly_time = get("report.weekly_time").unwrap_or_else(|| daily_time.clone());
        let last_daily = get("report.last_daily_date");
        let last_weekly = get("report.last_weekly_date");

        let mut sends = Vec::new();
        if report_due(
            daily_enabled,
            &daily_time,
            &now_hm,
            &today,
            last_daily.as_deref(),
        ) {
            sends.extend(
                store
                    .report_summary(&format!("{today}T00:00:00"), &format!("{today}T23:59:59"))
                    .map(|summary| ("日报", summary)),
            );
        }
        if weekly_due(
            weekly_enabled,
            weekly_day,
            &weekly_time,
            weekday,
            &now_hm,
            &today,
            last_weekly.as_deref(),
        ) {
            sends.extend(
                store
                    .report_summary(
                        &ls_core::time::local_days_ago_iso(6),
                        &format!("{today}T23:59:59"),
                    )
                    .map(|summary| ("周报", summary)),
            );
        }
        if sends.is_empty() {
            return;
        }
        let channels = self.alert_channels_snapshot();
        let names: std::collections::HashMap<i64, String> = store
            .cameras()
            .unwrap_or_default()
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect();
        for (title, summary) in sends {
            let text = report_text(
                title,
                summary.total,
                summary.violation,
                summary.unhandled,
                &summary.top,
                &names,
            );
            for (ch, cfg) in &channels {
                let _ = ls_dispatch::deliver_with_backoff(*ch, cfg, 2, &text, None, &[0, 0, 0]);
            }
            if title == "日报" {
                let _ = store.setting_set("report.last_daily_date", &today);
                if let Ok(mut g) = self.report_last_daily.lock() {
                    *g = Some(today.clone());
                }
            } else {
                let _ = store.setting_set("report.last_weekly_date", &today);
                if let Ok(mut g) = self.report_last_weekly.lock() {
                    *g = Some(today.clone());
                }
            }
        }
    }

    // ---- scheduler ----
    /// 启动后台线程（幂等）。首轮立即执行，之后按周期。
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
        // LS-053：告警失败重试消费者 + 日报/周报调度
        handles.push(spawn_loop(
            "alert-retry",
            Arc::clone(self),
            RETRY_INTERVAL_S,
            |j| j.retry_tick(),
        ));
        handles.push(spawn_loop(
            "reports",
            Arc::clone(self),
            REPORT_CHECK_S,
            |j| j.report_tick(),
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

/// T-329：从环境构造 MQTT 事件总线（LS_MQTT_*，与 Python settings 同名）。
///
/// 返回 `None` = 未启用/启动失败（事件总线属只读外发，任何故障不得影响主链路）。
/// 注意：**首发派发编排仍在 Python**（`mqtt_bridge.py` 为当前生产路径）；
/// 本入口供 L3 控制面接管首发派发时直接使用，payload/topic 语义已对齐。
pub fn mqtt_from_env() -> Option<ls_dispatch::mqtt::MqttBus> {
    if !env_bool("LS_MQTT_ENABLED", false) {
        return None;
    }
    let config = ls_dispatch::mqtt::MqttConfig {
        host: env_raw("LS_MQTT_HOST").unwrap_or_else(|| "127.0.0.1".into()),
        port: env_i64("LS_MQTT_PORT", 1883) as u16,
        username: env_raw("LS_MQTT_USERNAME").unwrap_or_default(),
        password: env_raw("LS_MQTT_PASSWORD").unwrap_or_default(),
        client_id: env_raw("LS_MQTT_CLIENT_ID").unwrap_or_default(),
        topic_prefix: env_raw("LS_MQTT_TOPIC_PREFIX").unwrap_or_else(|| "labsentinel".into()),
        qos: env_i64("LS_MQTT_QOS", 0) as u8,
        retain: env_bool("LS_MQTT_RETAIN", false),
        tls: env_bool("LS_MQTT_TLS", false),
    };
    match ls_dispatch::mqtt::MqttBus::start(config) {
        Ok(bus) => Some(bus),
        Err(e) => {
            println!("[mqtt] 事件总线启动失败（降级关闭）: {e}");
            None
        }
    }
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
    let now = ls_time::local_now_iso();
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
