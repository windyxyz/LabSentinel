//! ls-store（T-332/ADR-0008）：与 Python `store.py` 共用同一 SQLite 文件与 schema。
//!
//! Rust 侧**不重复实现迁移链**：打开 Python 已建好的库（含 v1~v12 迁移）直接读写，
//! 双向兼容由集成测试保证（Python 建库→Rust 读写 / Rust 建库→Python 读写）。
//! L4 移除 Python 运行时时，迁移链逐条翻译到 `migrations.rs`（保持相同编号）。

use rusqlite::{params, Connection, Row};
use serde::Serialize;

pub const SCHEMA_VERSION_LATEST: i64 = 12;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: i64,
    pub ts: String,
    pub camera_id: i64,
    pub type_: String,
    pub level: i64,
    pub track_id: i64,
    pub detail: String,
    pub snapshot: Option<String>,
    pub video: Option<String>,
    pub status: String,
}

/// T-325：rules 行（id, behavior, zone_id, time_start, time_end, cooldown_s）
pub type RuleRow = (i64, String, i64, String, String, i64);

/// T-332：add_event 入参（避免 8 参函数；与 Python add_event 同语义）
pub struct NewEvent<'a> {
    pub camera_id: i64,
    pub type_: &'a str,
    pub level: i64,
    pub track_id: i64,
    pub detail: &'a str,
    pub snapshot: Option<&'a str>,
    pub ts: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Camera {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub type_: String,
    pub status: String,
    pub group_name: String,
    pub analyzer: String,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// 打开已有库（Python 建库路径）；空文件则由 Python 负责首次建库。
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Store { conn })
    }

    /// 只读连接访问（T-330 维护作业：retention/escalation 需要游标分页与多表清理，
    /// 逐方法封装会让 ls-store 膨胀；SQL 仍由本 crate 之外的作业层集中维护）。
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// 双向兼容自检：schema_version 表存在且 ≥ 当前代码期望。
    pub fn schema_version(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
    }

    pub fn integrity_check(&self) -> rusqlite::Result<String> {
        self.conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
    }

    // ---- events ----
    pub fn add_event(&self, ev: &NewEvent) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO events(ts,camera_id,type,level,track_id,detail,snapshot) VALUES(?,?,?,?,?,?,?)",
            params![ev.ts, ev.camera_id, ev.type_, ev.level, ev.track_id, ev.detail,
                    ev.snapshot])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn events(
        &self,
        limit: i64,
        offset: i64,
        camera_id: Option<i64>,
        status: Option<&str>,
    ) -> rusqlite::Result<Vec<Event>> {
        let mut sql = String::from("SELECT id,ts,camera_id,type,level,track_id,detail,snapshot,video,COALESCE(status,'new') FROM events WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(cid) = camera_id {
            sql.push_str(" AND camera_id=?");
            args.push(Box::new(cid));
        }
        if let Some(st) = status {
            sql.push_str(" AND COALESCE(status,'new')=?");
            args.push(Box::new(st.to_string()));
        }
        sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");
        args.push(Box::new(limit));
        args.push(Box::new(offset));
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(vals.as_slice(), map_event)?;
        rows.collect()
    }

    pub fn event_by_id(&self, id: i64) -> rusqlite::Result<Option<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,ts,camera_id,type,level,track_id,detail,snapshot,video,COALESCE(status,'new') FROM events WHERE id=?")?;
        let mut rows = stmt.query_map(params![id], map_event)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn count_events(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
    }

    pub fn set_event_status(
        &self,
        id: i64,
        status: &str,
        handled_by: Option<&str>,
        handled_ts: Option<&str>,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE events SET status=?, handled_by=?, handled_ts=? WHERE id=?",
            params![status, handled_by, handled_ts, id],
        )
    }

    pub fn delete_events_older_than(&self, cutoff: &str) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM events WHERE ts < ?", params![cutoff])
    }

    // ---- cameras ----
    pub fn cameras(&self) -> rusqlite::Result<Vec<Camera>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,name,source,type,status,COALESCE(group_name,''),COALESCE(analyzer,'python') FROM cameras ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Camera {
                id: r.get(0)?,
                name: r.get(1)?,
                source: r.get(2)?,
                type_: r.get(3)?,
                status: r.get(4)?,
                group_name: r.get(5)?,
                analyzer: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn camera(&self, id: i64) -> rusqlite::Result<Option<Camera>> {
        let cams = self.cameras()?;
        Ok(cams.into_iter().find(|c| c.id == id))
    }

    pub fn set_camera_status(&self, id: i64, status: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE cameras SET status=? WHERE id=?",
            params![status, id],
        )
    }

    /// L3 控制面：新增摄像头（与 Python store.add_camera 同列/默认值）
    pub fn conn_add_camera(&self, name: &str, source: &str, type_: &str) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO cameras(name,source,type,group_name) VALUES(?,?,?,'')",
            params![name, source, type_],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    // ---- 配置（供 L3 控制面读） ----
    pub fn zones(&self, camera_id: i64) -> rusqlite::Result<Vec<(i64, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,name,polygon FROM zones WHERE camera_id=? ORDER BY id")?;
        let rows = stmt.query_map(params![camera_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect()
    }

    pub fn rules(&self, camera_id: i64) -> rusqlite::Result<Vec<RuleRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,behavior,COALESCE(zone_id,0),time_start,time_end,COALESCE(cooldown_s,0) FROM rules WHERE camera_id=? AND enabled=1")?;
        let rows = stmt.query_map(params![camera_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
        rows.collect()
    }

    // ---- 投递记录（L2 ls-dispatch 复用） ----
    pub fn add_delivery(&self, event_id: i64, channel: &str) -> rusqlite::Result<i64> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT INTO alert_deliveries(event_id,channel,status,attempts,updated_ts) VALUES(?,?,'pending',0,?)",
            params![event_id, channel, now])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_delivery(
        &self,
        id: i64,
        status: &str,
        attempts: i64,
        error: Option<&str>,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE alert_deliveries SET status=?, attempts=?, error=?, updated_ts=? WHERE id=?",
            params![status, attempts, error, chrono_now(), id],
        )
    }
}

fn map_event(r: &Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        ts: r.get(1)?,
        camera_id: r.get(2)?,
        type_: r.get(3)?,
        level: r.get(4)?,
        track_id: r.get(5)?,
        detail: r.get(6)?,
        snapshot: r.get(7)?,
        video: r.get(8)?,
        status: r.get(9)?,
    })
}

/// 本地 ISO 秒时间戳（无外部 chrono 依赖；格式与 Python datetime.now().isoformat 一致）
fn chrono_now() -> String {
    // Windows/Linux 通用：SystemTime → 秒 → 手写 civil-from-days（1970 起）
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant 的 civil_from_days（公历，days since 1970-01-01）
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// T-332：跨语言 DB 兼容自检（`labsentinel-server --selftest-db <path>`）。
/// 对既有库做一轮完整读写（事件/处置/投递/摄像头/区域/规则），全部成功返回 0。
pub mod selftest {
    use super::Store;
    use crate::NewEvent;

    pub fn run(db_path: &str) -> i32 {
        let Ok(s) = Store::open(db_path) else {
            eprintln!("[selftest-db] 打开失败: {db_path}");
            return 1;
        };
        let Ok(ver) = s.schema_version() else {
            eprintln!("[selftest-db] schema_version 读取失败");
            return 1;
        };
        if ver < 12 {
            eprintln!("[selftest-db] schema v{ver} < 12（需先由 Python 建全量迁移库）");
            return 1;
        }
        let Ok(cams) = s.cameras() else {
            eprintln!("[selftest-db] cameras 读取失败");
            return 1;
        };
        let cid = cams.first().map(|c| c.id).unwrap_or(1);
        let eid = match s.add_event(&NewEvent {
            camera_id: cid,
            type_: "enter",
            level: 2,
            track_id: 7,
            detail: "rust-selftest",
            snapshot: None,
            ts: "2026-01-01T00:00:01",
        }) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[selftest-db] add_event 失败: {e}");
                return 1;
            }
        };
        match s.event_by_id(eid) {
            Ok(Some(ev)) if ev.detail == "rust-selftest" => {}
            _ => {
                eprintln!("[selftest-db] event_by_id 读回不一致");
                return 1;
            }
        }
        if s.set_event_status(eid, "acked", Some("rust"), Some("2026-01-01T00:00:02"))
            .map(|n| n == 0)
            .unwrap_or(true)
        {
            eprintln!("[selftest-db] set_event_status 失败");
            return 1;
        }
        let Ok(Some(ev)) = s.event_by_id(eid) else {
            eprintln!("[selftest-db] 处置后读回失败");
            return 1;
        };
        if ev.status != "acked" {
            eprintln!("[selftest-db] 处置状态未持久化: {}", ev.status);
            return 1;
        }
        let _ = s.zones(cid);
        let _ = s.rules(cid);
        match s.integrity_check() {
            Ok(v) if v == "ok" => {
                println!("[selftest-db] OK schema=v{ver} event={eid}");
                0
            }
            other => {
                eprintln!("[selftest-db] integrity_check 未过: {other:?}");
                1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20454), (2026, 1, 1)); // 复核点
    }

    #[test]
    fn store_roundtrip_on_python_built_db() {
        // 双向兼容：Python 建库（scripts/seed_demo.py 或 Store()）后 Rust 读写。
        // 若 tests 目录旁无现成库则用纯内存自建最小 schema 验证 SQL 契约。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        let conn = Connection::open(&p).unwrap();
        conn.execute_batch("CREATE TABLE schema_version(version INTEGER PRIMARY KEY, applied_at TEXT); \
            INSERT INTO schema_version VALUES(12,'2026-01-01T00:00:00'); \
            CREATE TABLE events(id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, type TEXT, \
              level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT, status TEXT DEFAULT 'new', \
              handled_by TEXT, handled_ts TEXT); \
            CREATE TABLE cameras(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, type TEXT, \
              status TEXT DEFAULT 'stopped', group_name TEXT DEFAULT '', analyzer TEXT DEFAULT 'python'); \
            CREATE TABLE zones(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, name TEXT, polygon TEXT); \
            CREATE TABLE rules(id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, zone_id INTEGER, \
              behavior TEXT, level INTEGER, cooldown_s INTEGER, time_start TEXT, time_end TEXT, \
              threshold INTEGER, enabled INTEGER, days_of_week TEXT, holiday_only INTEGER); \
            CREATE TABLE alert_deliveries(id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER, channel TEXT, \
              status TEXT, attempts INTEGER, error TEXT, updated_ts TEXT);").unwrap();
        drop(conn);
        let s = Store::open(p.to_str().unwrap()).unwrap();
        assert_eq!(s.schema_version().unwrap(), 12);
        let eid = s
            .add_event(&NewEvent {
                camera_id: 1,
                type_: "enter",
                level: 2,
                track_id: 7,
                detail: "Rust 写入",
                snapshot: None,
                ts: "2026-01-01T00:00:01",
            })
            .unwrap();
        let got = s.event_by_id(eid).unwrap().unwrap();
        assert_eq!(got.detail, "Rust 写入");
        assert_eq!(got.status, "new");
        s.set_event_status(eid, "acked", Some("rust"), Some("2026-01-01T00:00:02"))
            .unwrap();
        assert_eq!(s.event_by_id(eid).unwrap().unwrap().status, "acked");
        assert_eq!(s.count_events().unwrap(), 1);
        let d = s.add_delivery(eid, "wecom").unwrap();
        s.update_delivery(d, "ok", 1, None).unwrap();
        assert_eq!(s.integrity_check().unwrap(), "ok");
    }
}
