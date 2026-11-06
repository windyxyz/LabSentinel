//! 未处置事件升级提醒（T-330 / Python `escalation.py` 等价实现）。
//!
//! 命中条件：status=new 且 escalated_ts IS NULL 且 ts < now-after_min 且 level >= min_level；
//! 提醒后写 escalated_ts（发送失败也标记，防每分钟重试风暴）；每轮最多 batch 条。
//! 发送本身可注入（测试用），生产由 ls-dispatch 渠道完成（见 `Jobs::escalate_once`）。

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use ls_store::Store;
use serde::Serialize;

use crate::JobResult;

pub const DEFAULT_TEMPLATE: &str =
    "{icon} L{level} {type_text} · {ts}\n摄像头 #{camera_id}{track_part}\n{detail}";

#[derive(Clone, Debug)]
pub struct EscalationConfig {
    pub after_min: i64,
    pub min_level: i64,
    pub batch: i64,
    pub template_text: String,
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self {
            after_min: 15,
            min_level: 3,
            batch: 20,
            template_text: DEFAULT_TEMPLATE.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct EscalationRow {
    pub id: i64,
    pub ts: String,
    pub camera_id: i64,
    pub type_: String,
    pub level: i64,
    pub track_id: i64,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct EscalationOutcome {
    pub checked: usize,
    pub escalated: usize,
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

fn parse_ts(ts: &str) -> Option<DateTime<Local>> {
    let naive = NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S").ok()?;
    Local.from_local_datetime(&naive).single()
}

/// 超时未处置且未提醒过的事件（旧→新排序，限 batch 条）。
pub fn find_unhandled(
    store: &Store,
    now: &DateTime<Local>,
    cfg: &EscalationConfig,
) -> JobResult<Vec<EscalationRow>> {
    let cutoff = (*now - chrono::TimeDelta::minutes(cfg.after_min.max(0)))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let mut stmt = store
        .conn()
        .prepare(
            "SELECT id, ts, camera_id, type, level, track_id, detail FROM events \
             WHERE status='new' AND escalated_ts IS NULL AND ts < ? AND level >= ? \
             ORDER BY id ASC LIMIT ?",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![cutoff, cfg.min_level, cfg.batch.max(0)],
            |r| {
                Ok(EscalationRow {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    camera_id: r.get(2)?,
                    type_: r.get(3)?,
                    level: r.get(4)?,
                    track_id: r.get(5)?,
                    detail: r.get(6)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub fn mark_escalated(store: &Store, eid: i64, ts: &str) -> JobResult<()> {
    store
        .conn()
        .execute(
            "UPDATE events SET escalated_ts=? WHERE id=?",
            rusqlite::params![ts, eid],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn type_text(t: &str) -> &str {
    match t {
        "enter" => "进入安全区",
        "enter_no_ppe" => "进入安全区未穿实验服",
        "exit" => "离开安全区",
        "occupancy_over" => "在场人数超限",
        "stranger" => "陌生人进入",
        _ => t,
    }
}

fn level_icon(level: i64) -> &'static str {
    match level {
        2 => "⚠️",
        3 => "🔴",
        _ => "ℹ️",
    }
}

/// 单遍占位符替换（未知占位符原样保留；`{{`/`}}` 转义；detail 内的花括号不二次替换）。
pub fn render_template(template: &str, vals: &[(&str, String)]) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::with_capacity(template.len() + 16);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                out.push('{');
                i += 2;
                continue;
            }
            if let Some(off) = chars[i + 1..].iter().position(|c| *c == '}') {
                let key: String = chars[i + 1..i + 1 + off].iter().collect();
                if let Some((_, v)) = vals.iter().find(|(k, _)| *k == key) {
                    out.push_str(v);
                } else {
                    out.push('{');
                    out.push_str(&key);
                    out.push('}');
                }
                i = i + 1 + off + 1;
                continue;
            }
        } else if chars[i] == '}' && i + 1 < chars.len() && chars[i + 1] == '}' {
            out.push('}');
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 告警文案（Python `render_alert` 等价，默认模板逐字一致）。
pub fn render_alert(row: &EscalationRow, template: &str) -> String {
    let track_part = if row.type_ == "occupancy_over" {
        String::new()
    } else {
        format!(" · track {}", row.track_id)
    };
    let vals = vec![
        ("icon", level_icon(row.level).to_string()),
        ("level", row.level.to_string()),
        ("type", row.type_.clone()),
        ("type_text", type_text(&row.type_).to_string()),
        ("ts", row.ts.clone()),
        ("camera_id", row.camera_id.to_string()),
        ("track_id", row.track_id.to_string()),
        ("track_part", track_part),
        ("detail", row.detail.clone()),
    ];
    render_template(template, &vals)
}

/// 发送回调（测试注入/生产渠道派发）：返回是否至少一个渠道接受。
pub type ReminderSender<'a> = &'a dyn Fn(&EscalationRow, &str) -> bool;

/// 跑一轮：返回 {checked, escalated}；send 可注入（None=仅标记并记日志）。
pub fn run_once(
    store: &Store,
    cfg: &EscalationConfig,
    now: Option<DateTime<Local>>,
    send: Option<ReminderSender<'_>>,
) -> JobResult<EscalationOutcome> {
    let now = now.unwrap_or_else(Local::now);
    let ts = now.format("%Y-%m-%dT%H:%M:%S").to_string();
    if cfg.after_min <= 0 {
        return Ok(EscalationOutcome {
            checked: 0,
            escalated: 0,
            ts,
            disabled: Some(true),
        });
    }
    let rows = find_unhandled(store, &now, cfg)?;
    let mut escalated = 0;
    for r in &rows {
        let waited = parse_ts(&r.ts)
            .map(|t| (now - t).num_minutes().max(0))
            .unwrap_or(0);
        let text = format!(
            "⏰ 未处置提醒（已 {waited} 分钟）\n{}",
            render_alert(r, &cfg.template_text)
        );
        let ok = send.map(|f| f(r, &text)).unwrap_or(false);
        mark_escalated(store, r.id, &ts)?;
        escalated += 1;
        if !ok {
            println!(
                "[escalate] event#{} 提醒发送失败（已标记 escalated_ts，不重试风暴）",
                r.id
            );
        }
    }
    Ok(EscalationOutcome {
        checked: rows.len(),
        escalated,
        ts,
        disabled: None,
    })
}

/// 升级提醒状态（Python escalation.status() 同字段）。
pub fn status_json(
    store: &Store,
    cfg: &EscalationConfig,
    last: Option<&EscalationOutcome>,
) -> JobResult<serde_json::Value> {
    let conn = store.conn();
    let pending: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE status='new' AND level >= ?",
            [cfg.min_level],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let today = Local::now().format("%Y-%m-%d").to_string();
    let escalated_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE escalated_ts LIKE ?",
            [format!("{today}%")],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(serde_json::json!({
        "enabled": cfg.after_min > 0,
        "after_min": cfg.after_min,
        "min_level": cfg.min_level,
        "pending": pending,
        "escalated_today": escalated_today,
        "last": last.map(|l| serde_json::to_value(l).unwrap_or(serde_json::json!({})))
            .unwrap_or(serde_json::json!({})),
    }))
}
