//! 统一本地时区时间模块（替代已停更的 `chrono` / RUSTSEC-2026-0543，
//! 同时消灭全仓 5 处手写 UTC `civil_from_days` 与 `Local::now()` 之间的 8 小时撕裂）。

use std::time::SystemTime;

pub use jiff::civil::DateTime;
use jiff::{SignedDuration, Timestamp, Zoned};

/// 当前本地时间（Zoned，自动读取系统时区）。
pub fn local_now() -> Zoned {
    Zoned::now()
}

/// 当前本地民用时间（无时区偏移包装，便于统一比较与格式化）。
pub fn local_now_civil() -> DateTime {
    Zoned::now().datetime()
}

/// 格式化民用时间为 `YYYY-MM-DDTHH:MM:SS`（与 Python `datetime.now().isoformat(timespec="seconds")` 同口径）。
pub fn format_iso(dt: DateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

/// 格式化民用日期为 `YYYY-MM-DD`。
pub fn format_date(dt: DateTime) -> String {
    format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day())
}

/// 格式化民用时分为 `HH:MM`。
pub fn format_hm(dt: DateTime) -> String {
    format!("{:02}:{:02}", dt.hour(), dt.minute())
}

/// ISO 星期编号（1=周一 .. 7=周日）。
pub fn weekday_iso(dt: DateTime) -> u8 {
    dt.weekday().to_monday_one_offset() as u8
}

/// 当前本地 ISO 秒时间戳 `YYYY-MM-DDTHH:MM:SS`。
pub fn local_now_iso() -> String {
    format_iso(local_now_civil())
}

/// 当前本地 ISO 时间戳带时区偏移 `YYYY-MM-DDTHH:MM:SS+0800`（备份 manifest 用）。
pub fn local_now_iso_offset() -> String {
    let z = local_now();
    let off = z.offset().seconds();
    let sign = if off < 0 { '-' } else { '+' };
    let abs = off.abs();
    let oh = abs / 3600;
    let om = (abs % 3600) / 60;
    format!("{}{sign}{oh:02}{om:02}", format_iso(z.datetime()))
}

/// 当前本地紧凑时间戳 `YYYYMMDD-HHMMSS`（备份文件名用）。
pub fn local_now_compact() -> String {
    let dt = local_now_civil();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

/// 当前本地日期 `YYYY-MM-DD`。
pub fn local_today_iso() -> String {
    format_date(local_now_civil())
}

/// 当前本地紧凑日期 `YYYYMMDD`（日志文件滚动名用）。
pub fn local_today_compact() -> String {
    let dt = local_now_civil();
    format!("{:04}{:02}{:02}", dt.year(), dt.month(), dt.day())
}

/// 当前本地时间偏移指定秒数后的 `DateTime`。
pub fn local_offset_secs(delta_secs: i64) -> DateTime {
    let z = local_now();
    let shifted = z
        .timestamp()
        .checked_add(SignedDuration::from_secs(delta_secs))
        .unwrap_or_else(|_| z.timestamp());
    shifted.to_zoned(z.time_zone().clone()).datetime()
}

/// `days` 天前的本地 ISO 秒时间戳 `YYYY-MM-DDTHH:MM:SS`。
pub fn local_days_ago_iso(days: i64) -> String {
    format_iso(local_offset_secs(-days.saturating_mul(86_400)))
}

/// `days` 天前的本地日期 `YYYY-MM-DD`。
pub fn local_days_ago_date(days: i64) -> String {
    format_date(local_offset_secs(-days.saturating_mul(86_400)))
}

/// `mins` 分钟前的本地 ISO 秒时间戳 `YYYY-MM-DDTHH:MM:SS`。
pub fn local_minutes_ago_iso(mins: i64) -> String {
    format_iso(local_offset_secs(-mins.saturating_mul(60)))
}

/// `hours` 小时前的本地 ISO 秒时间戳 `YYYY-MM-DDTHH:MM:SS`。
pub fn local_hours_ago_iso(hours: i64) -> String {
    format_iso(local_offset_secs(-hours.saturating_mul(3600)))
}

/// 将 `SystemTime` 转为本地 ISO 秒时间戳。
pub fn format_system_time_local_iso(st: SystemTime) -> String {
    match Timestamp::try_from(st) {
        Ok(ts) => {
            let tz = local_now().time_zone().clone();
            format_iso(ts.to_zoned(tz).datetime())
        }
        Err(_) => local_now_iso(),
    }
}

/// 解析事件时间戳前缀（兼容 `YYYY-MM-DDTHH:MM:SS`、`YYYY-MM-DD HH:MM:SS`、小数秒或带偏移）。
pub fn parse_local_datetime(s: &str) -> Option<DateTime> {
    let s = s.trim();
    if s.len() < 19 || !s.is_char_boundary(19) {
        return None;
    }
    let head = &s[..19];
    let normalized = if head.as_bytes()[10] == b' ' {
        format!("{}T{}", &head[..10], &head[11..])
    } else {
        head.to_string()
    };
    normalized.parse::<DateTime>().ok()
}

/// 计算两个本地民用时间之间的整分钟差 `(later - earlier)`。
pub fn diff_minutes(later: DateTime, earlier: DateTime) -> i64 {
    let span = earlier
        .until(later)
        .unwrap_or_default()
        .total(jiff::Unit::Minute)
        .unwrap_or(0.0);
    span.trunc() as i64
}

/// 给 `DateTime` 加上指定毫秒数（用于离线黄金语料确定性时间戳）。
pub fn add_millis(dt: DateTime, ms: i64) -> DateTime {
    let dur = SignedDuration::from_millis(ms);
    dt.checked_add(dur).unwrap_or(dt)
}

/// 从固定 ISO 起点加毫秒并输出 `YYYY-MM-DDTHH:MM:SS`。
pub fn add_ms_iso(base_iso: &str, ms: i64) -> Option<String> {
    let dt = parse_local_datetime(base_iso)?;
    Some(format_iso(add_millis(dt, ms)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_roundtrip() {
        let dt = parse_local_datetime("2026-11-22T08:30:45.123+08:00").unwrap();
        assert_eq!(format_iso(dt), "2026-11-22T08:30:45");
        assert_eq!(format_date(dt), "2026-11-22");
        assert_eq!(format_hm(dt), "08:30");
        assert_eq!(weekday_iso(dt), 7); // 2026-11-22 是周日
        assert_eq!(
            add_ms_iso("2026-01-01T00:00:00", 1500).as_deref(),
            Some("2026-01-01T00:00:01")
        );
    }

    #[test]
    fn diff_minutes_and_local_helpers() {
        let a = parse_local_datetime("2026-11-22 08:00:00").unwrap();
        let b = parse_local_datetime("2026-11-22T08:15:30").unwrap();
        assert_eq!(diff_minutes(b, a), 15);
        assert_eq!(local_now_iso().len(), 19);
        assert_eq!(local_today_iso().len(), 10);
        assert_eq!(local_today_compact().len(), 8);
    }
}
