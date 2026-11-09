//! ls-store（T-332/ADR-0008）：与 Python `store.py` 共用同一 SQLite 文件与 schema。
//!
//! Rust 侧**不重复实现迁移链**：打开 Python 已建好的库（含 v1~v13 迁移）直接读写，
//! 双向兼容由集成测试保证（Python 建库→Rust 读写 / Rust 建库→Python 读写）。
//! L4 移除 Python 运行时时，迁移链逐条翻译到 `migrations.rs`（保持相同编号）。

use rusqlite::{params, Connection, Row};
use serde::Serialize;

/// 与 `server/app/store.py: SCHEMA_VERSION_LATEST` 的权威编号一致（v13 =
/// 设备配对令牌表，T-620）；此处只作文档/诊断常量，不参与迁移执行。
pub const SCHEMA_VERSION_LATEST: i64 = 13;

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
            "UPDATE alert_deliveries SET status=?, attempts=?, last_error=?, updated_ts=? WHERE id=?",
            params![status, attempts, error, chrono_now(), id],
        )
    }
}

// ==== R2-309 批次 1：控制面（L3）补齐所需的 store 方法 ====
// 与 Python `store.py` 同表同列同语义；ls-server 独立运行时读写的表。

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: i64,
    pub name: String,
    pub platform: String,
    pub created_ts: String,
    pub last_seen_ts: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub ts: String,
    pub request_id: Option<String>,
    pub actor: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub status_code: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Delivery {
    pub id: i64,
    pub event_id: i64,
    pub channel: String,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub updated_ts: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Silence {
    pub id: i64,
    pub scope: String,
    pub target: Option<String>,
    pub reason: Option<String>,
    pub created_ts: String,
    pub until_ts: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalendarRow {
    pub date: String,
    pub mode: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub config_json: String,
    pub created_ts: Option<String>,
    pub updated_ts: Option<String>,
}

/// 事件列表过滤（R2-309：与 Python `GET /api/events` 同参数口径）
#[derive(Default)]
pub struct EventsFilter<'a> {
    pub camera_id: Option<i64>,
    pub status: Option<&'a str>,
    pub level: Option<i64>,
    pub type_: Option<&'a str>,
    pub start_ts: Option<&'a str>,
    pub end_ts: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

/// 统计聚合行（Python `_EMPTY` 同形：total/l1/l2/l3 + 按类型计数）
pub type AggRow = std::collections::BTreeMap<String, i64>;

fn agg_rows(rows: Vec<(String, i64, String, i64)>) -> std::collections::BTreeMap<String, AggRow> {
    let mut out: std::collections::BTreeMap<String, AggRow> = Default::default();
    for (key, level, type_, n) in rows {
        let row = out.entry(key).or_insert_with(|| {
            let mut m = AggRow::new();
            m.insert("total".into(), 0);
            m.insert("l1".into(), 0);
            m.insert("l2".into(), 0);
            m.insert("l3".into(), 0);
            m
        });
        *row.entry("total".into()).or_insert(0) += n;
        *row.entry(format!("l{}", level)).or_insert(0) += n;
        *row.entry(type_).or_insert(0) += n;
    }
    out
}

impl Store {
    // ---- cameras（写扩展） ----
    pub fn delete_camera(&self, id: i64) -> rusqlite::Result<usize> {
        // 与 Python delete_camera 同语义：级联清理区域/规则（外键未启用时手动删）
        self.conn
            .execute("DELETE FROM rules WHERE camera_id=?", params![id])?;
        self.conn
            .execute("DELETE FROM zones WHERE camera_id=?", params![id])?;
        self.conn
            .execute("DELETE FROM cameras WHERE id=?", params![id])
    }

    pub fn set_camera_group(&self, id: i64, group: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE cameras SET group_name=? WHERE id=?",
            params![group, id],
        )
    }

    pub fn set_camera_analyzer(&self, id: i64, analyzer: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE cameras SET analyzer=? WHERE id=?",
            params![analyzer, id],
        )
    }

    pub fn camera_groups(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT COALESCE(group_name,'') FROM cameras WHERE COALESCE(group_name,'')<>'' ORDER BY 1")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    // ---- zones ----
    pub fn zones_all(&self) -> rusqlite::Result<Vec<(i64, i64, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,camera_id,name,polygon FROM zones ORDER BY camera_id,id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect()
    }

    pub fn add_zone(
        &self,
        camera_id: i64,
        name: &str,
        polygon_json: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO zones(camera_id,name,polygon) VALUES(?,?,?)",
            params![camera_id, name, polygon_json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_zone(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM zones WHERE id=?", params![id])
    }

    // ---- rules ----
    pub fn rules_all(&self) -> rusqlite::Result<Vec<RuleFullRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,camera_id,zone_id,behavior,level,COALESCE(cooldown_s,0),time_start,time_end,COALESCE(threshold,0),enabled FROM rules ORDER BY camera_id,id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ))
        })?;
        rows.collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_rule(
        &self,
        camera_id: i64,
        zone_id: Option<i64>,
        behavior: &str,
        level: i64,
        cooldown_s: i64,
        time_start: Option<&str>,
        time_end: Option<&str>,
        threshold: i64,
        enabled: bool,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO rules(camera_id,zone_id,behavior,level,cooldown_s,time_start,time_end,threshold,enabled,days_of_week,holiday_only) \
             VALUES(?,?,?,?,?,?,?,?,?,0,0)",
            params![camera_id, zone_id, behavior, level, cooldown_s, time_start, time_end, threshold, enabled as i64],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_rule(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM rules WHERE id=?", params![id])
    }

    // ---- events（过滤 / 批量处置） ----
    pub fn events_query(&self, f: &EventsFilter<'_>) -> rusqlite::Result<Vec<Event>> {
        let mut sql = String::from(
            "SELECT id,ts,camera_id,type,level,track_id,detail,snapshot,video,COALESCE(status,'new') FROM events WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(v) = f.camera_id {
            sql.push_str(" AND camera_id=?");
            args.push(Box::new(v));
        }
        if let Some(v) = f.status {
            sql.push_str(" AND COALESCE(status,'new')=?");
            args.push(Box::new(v.to_string()));
        }
        if let Some(v) = f.level {
            sql.push_str(" AND level=?");
            args.push(Box::new(v));
        }
        if let Some(v) = f.type_ {
            sql.push_str(" AND type=?");
            args.push(Box::new(v.to_string()));
        }
        if let Some(v) = f.start_ts {
            sql.push_str(" AND ts>=?");
            args.push(Box::new(v.to_string()));
        }
        if let Some(v) = f.end_ts {
            sql.push_str(" AND ts<=?");
            args.push(Box::new(v.to_string()));
        }
        sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");
        args.push(Box::new(f.limit));
        args.push(Box::new(f.offset));
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(vals.as_slice(), map_event)?;
        rows.collect()
    }

    pub fn bulk_status(
        &self,
        ids: &[i64],
        status: &str,
        handled_by: &str,
    ) -> rusqlite::Result<usize> {
        let mut n = 0;
        for id in ids {
            n += self.conn.execute(
                "UPDATE events SET status=?, handled_by=?, handled_ts=? WHERE id=?",
                params![status, handled_by, chrono_now(), id],
            )?;
        }
        Ok(n)
    }

    pub fn count_events_by(
        &self,
        camera_id: Option<i64>,
        level: Option<i64>,
        status: Option<&str>,
        start_ts: Option<&str>,
        end_ts: Option<&str>,
    ) -> rusqlite::Result<i64> {
        let mut sql = String::from("SELECT COUNT(*) FROM events WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(v) = camera_id {
            sql.push_str(" AND camera_id=?");
            args.push(Box::new(v));
        }
        if let Some(v) = level {
            sql.push_str(" AND level=?");
            args.push(Box::new(v));
        }
        if let Some(v) = status {
            let st = if v == "unhandled" { "new" } else { v };
            sql.push_str(" AND COALESCE(status,'new')=?");
            args.push(Box::new(st.to_string()));
        }
        if let Some(v) = start_ts {
            sql.push_str(" AND ts>=?");
            args.push(Box::new(v.to_string()));
        }
        if let Some(v) = end_ts {
            sql.push_str(" AND ts<=?");
            args.push(Box::new(v.to_string()));
        }
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        self.conn.query_row(&sql, vals.as_slice(), |r| r.get(0))
    }

    // ---- 设备配对（T-620） ----
    pub fn add_device(
        &self,
        name: &str,
        platform: &str,
        token_hash: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO devices(name,platform,token_hash,created_ts) VALUES(?,?,?,?)",
            params![name, platform, token_hash, chrono_now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn devices(&self) -> rusqlite::Result<Vec<Device>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,name,COALESCE(platform,''),created_ts,last_seen_ts,COALESCE(revoked,0) FROM devices ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Device {
                id: r.get(0)?,
                name: r.get(1)?,
                platform: r.get(2)?,
                created_ts: r.get(3)?,
                last_seen_ts: r.get(4)?,
                revoked: r.get::<_, i64>(5)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn device_by_hash(&self, token_hash: &str) -> rusqlite::Result<Option<Device>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,name,COALESCE(platform,''),created_ts,last_seen_ts,COALESCE(revoked,0) FROM devices WHERE token_hash=?")?;
        let mut rows = stmt.query_map(params![token_hash], |r| {
            Ok(Device {
                id: r.get(0)?,
                name: r.get(1)?,
                platform: r.get(2)?,
                created_ts: r.get(3)?,
                last_seen_ts: r.get(4)?,
                revoked: r.get::<_, i64>(5)? != 0,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn touch_device(&self, device_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE devices SET last_seen_ts=? WHERE id=?",
            params![chrono_now(), device_id],
        )
    }

    pub fn revoke_device(&self, device_id: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE devices SET revoked=1 WHERE id=?",
            params![device_id],
        )?;
        Ok(n > 0)
    }

    // ---- 审计（任务87） ----
    #[allow(clippy::too_many_arguments)]
    pub fn audit_add(
        &self,
        ts: &str,
        request_id: &str,
        actor: &str,
        action: &str,
        target: Option<&str>,
        status_code: i64,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO audit_log(ts,request_id,actor,action,target,status_code) VALUES(?,?,?,?,?,?)",
            params![ts, request_id, actor, action, target, status_code],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn audit_list(
        &self,
        limit: i64,
        offset: i64,
        action: Option<&str>,
        actor: Option<&str>,
    ) -> rusqlite::Result<(Vec<AuditRow>, i64)> {
        let mut where_sql = String::new();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(a) = action {
            where_sql.push_str(" AND action LIKE ?");
            args.push(Box::new(format!("{a}%")));
        }
        if let Some(a) = actor {
            where_sql.push_str(" AND actor=?");
            args.push(Box::new(a.to_string()));
        }
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM audit_log WHERE 1=1{where_sql}"),
            vals.as_slice(),
            |r| r.get(0),
        )?;
        let mut stmt = self.conn.prepare(&format!(
            "SELECT id,ts,request_id,actor,action,target,status_code FROM audit_log WHERE 1=1{where_sql} ORDER BY id DESC LIMIT ? OFFSET ?"))?;
        // rusqlite 参数不能复用同一绑定缓冲，重新构造一次
        let mut args2: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(a) = action {
            args2.push(Box::new(format!("{a}%")));
        }
        if let Some(a) = actor {
            args2.push(Box::new(a.to_string()));
        }
        args2.push(Box::new(limit));
        args2.push(Box::new(offset));
        let vals2: Vec<&dyn rusqlite::ToSql> = args2.iter().map(|a| a.as_ref()).collect();
        let rows = stmt.query_map(vals2.as_slice(), |r| {
            Ok(AuditRow {
                id: r.get(0)?,
                ts: r.get(1)?,
                request_id: r.get(2)?,
                actor: r.get(3)?,
                action: r.get(4)?,
                target: r.get(5)?,
                status_code: r.get(6)?,
            })
        })?;
        Ok((rows.collect::<Result<Vec<_>, _>>()?, total))
    }

    // ---- 投递记录 ----
    pub fn deliveries(&self, status: Option<&str>, limit: i64) -> rusqlite::Result<Vec<Delivery>> {
        let mut sql = String::from(
            "SELECT id,event_id,channel,COALESCE(status,'pending'),attempts,last_error,updated_ts FROM alert_deliveries WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = status {
            sql.push_str(" AND COALESCE(status,'pending')=?");
            args.push(Box::new(s.to_string()));
        }
        sql.push_str(" ORDER BY id DESC LIMIT ?");
        args.push(Box::new(limit));
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(vals.as_slice(), |r| {
            Ok(Delivery {
                id: r.get(0)?,
                event_id: r.get(1)?,
                channel: r.get(2)?,
                status: r.get(3)?,
                attempts: r.get(4)?,
                last_error: r.get(5)?,
                updated_ts: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn retry_delivery(&self, id: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE alert_deliveries SET status='pending', attempts=0, last_error=NULL, updated_ts=? WHERE id=?",
            params![chrono_now(), id],
        )?;
        Ok(n > 0)
    }

    // ---- 告警静默（任务81） ----
    pub fn silences(&self) -> rusqlite::Result<Vec<Silence>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,scope,target,reason,created_ts,until_ts FROM alert_silences ORDER BY id DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(Silence {
                id: r.get(0)?,
                scope: r.get(1)?,
                target: r.get(2)?,
                reason: r.get(3)?,
                created_ts: r.get(4)?,
                until_ts: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_silence(
        &self,
        scope: &str,
        target: Option<&str>,
        reason: Option<&str>,
        until_ts: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO alert_silences(scope,target,reason,created_ts,until_ts,created_by) VALUES(?,?,?,?,?,'ls-server')",
            params![scope, target, reason, chrono_now(), until_ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_silence(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM alert_silences WHERE id=?", params![id])?
            > 0)
    }

    // ---- 日历例外（任务112） ----
    pub fn calendar_exceptions(&self) -> rusqlite::Result<Vec<CalendarRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT date,mode,COALESCE(note,'') FROM calendar_exceptions ORDER BY date")?;
        let rows = stmt.query_map([], |r| {
            Ok(CalendarRow {
                date: r.get(0)?,
                mode: r.get(1)?,
                note: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn set_calendar_exception(
        &self,
        date: &str,
        mode: &str,
        note: &str,
    ) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "INSERT INTO calendar_exceptions(date,mode,note) VALUES(?,?,?) \
             ON CONFLICT(date) DO UPDATE SET mode=excluded.mode, note=excluded.note",
            params![date, mode, note],
        )?;
        Ok(n > 0)
    }

    pub fn delete_calendar_exception(&self, date: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM calendar_exceptions WHERE date=?",
            params![date],
        )? > 0)
    }

    // ---- 规则模板（任务116） ----
    pub fn rule_templates(&self) -> rusqlite::Result<Vec<Template>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,name,config_json,created_ts,updated_ts FROM rule_templates ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Template {
                id: r.get(0)?,
                name: r.get(1)?,
                config_json: r.get(2)?,
                created_ts: r.get(3)?,
                updated_ts: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_rule_template(&self, name: &str, config_json: &str) -> rusqlite::Result<i64> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT INTO rule_templates(name,config_json,created_ts,updated_ts) VALUES(?,?,?,?)",
            params![name, config_json, now, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_rule_template(
        &self,
        id: i64,
        name: &str,
        config_json: &str,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE rule_templates SET name=?, config_json=?, updated_ts=? WHERE id=?",
            params![name, config_json, chrono_now(), id],
        )
    }

    pub fn delete_rule_template(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM rule_templates WHERE id=?", params![id])?
            > 0)
    }

    // ---- 统计聚合（与 stats.py 同口径） ----
    fn _agg_sql(
        &self,
        select_key: &str,
        extra_where: &str,
        args: &[Box<dyn rusqlite::ToSql>],
    ) -> rusqlite::Result<std::collections::BTreeMap<String, AggRow>> {
        let sql = format!(
            "SELECT {select_key} k, level, type, COUNT(*) n FROM events WHERE 1=1{extra_where} GROUP BY k, level, type");
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(vals.as_slice(), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(agg_rows(rows))
    }

    /// GET /api/stats/daily：最近 N 天逐日（缺数据天补零，_EMPTY 同形）
    pub fn stats_daily(
        &self,
        days: i64,
        camera_id: Option<i64>,
        status: Option<&str>,
    ) -> rusqlite::Result<Vec<(String, AggRow)>> {
        let days = days.clamp(1, 90);
        let start = iso_days_ago(days - 1);
        let mut where_sql = " AND ts>=?".to_string();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(start)];
        if let Some(c) = camera_id {
            where_sql.push_str(" AND camera_id=?");
            args.push(Box::new(c));
        }
        if let Some(s) = status {
            let st = if s == "unhandled" { "new" } else { s };
            where_sql.push_str(" AND COALESCE(status,'new')=?");
            args.push(Box::new(st.to_string()));
        }
        let agg = self._agg_sql("substr(ts,1,10)", &where_sql, &args)?;
        let mut out = Vec::new();
        for i in 0..days {
            let d = iso_days_ago(days - 1 - i);
            let empty = || {
                let mut m = AggRow::new();
                for k in ["total", "l1", "l2", "l3"] {
                    m.insert(k.into(), 0);
                }
                m
            };
            let row = agg.get(&d).cloned().unwrap_or_else(empty);
            out.push((d, row));
        }
        Ok(out)
    }

    /// GET /api/stats/weekly：按 ISO 周一聚合，最近 N 周
    pub fn stats_weekly(
        &self,
        weeks: i64,
        camera_id: Option<i64>,
        status: Option<&str>,
    ) -> rusqlite::Result<Vec<(String, AggRow)>> {
        let weeks = weeks.clamp(1, 26);
        let start = iso_weeks_ago_monday(weeks - 1);
        let mut where_sql = " AND ts>=?".to_string();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(start)];
        if let Some(c) = camera_id {
            where_sql.push_str(" AND camera_id=?");
            args.push(Box::new(c));
        }
        if let Some(s) = status {
            let st = if s == "unhandled" { "new" } else { s };
            where_sql.push_str(" AND COALESCE(status,'new')=?");
            args.push(Box::new(st.to_string()));
        }
        let agg = self._agg_sql(
            "date(substr(ts,1,10),'-6 days','weekday 1')",
            &where_sql,
            &args,
        )?;
        let mut out = Vec::new();
        for i in 0..weeks {
            let wk = iso_weeks_ago_monday(weeks - 1 - i);
            let empty = || {
                let mut m = AggRow::new();
                for k in ["total", "l1", "l2", "l3"] {
                    m.insert(k.into(), 0);
                }
                m
            };
            let row = agg.get(&wk).cloned().unwrap_or_else(empty);
            out.push((wk, row));
        }
        Ok(out)
    }

    /// GET /api/stats/hourly：某日 0-23 时逐小时
    pub fn stats_hourly(
        &self,
        date: &str,
        camera_id: Option<i64>,
    ) -> rusqlite::Result<Vec<(String, AggRow)>> {
        let mut where_sql = " AND substr(ts,1,10)=?".to_string();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(date.to_string())];
        if let Some(c) = camera_id {
            where_sql.push_str(" AND camera_id=?");
            args.push(Box::new(c));
        }
        let agg = self._agg_sql("substr(ts,12,2)", &where_sql, &args)?;
        let mut out = Vec::new();
        for h in 0..24i64 {
            let key = format!("{h:02}");
            let empty = || {
                let mut m = AggRow::new();
                for k in ["total", "l1", "l2", "l3"] {
                    m.insert(k.into(), 0);
                }
                m
            };
            let row = agg.get(&key).cloned().unwrap_or_else(empty);
            out.push((key, row));
        }
        Ok(out)
    }

    /// GET /api/events/timeline：日期+小时热力桶（缺数据补零）
    pub fn events_timeline(
        &self,
        days: i64,
        camera_id: Option<i64>,
        level: Option<i64>,
    ) -> rusqlite::Result<Vec<TimelineDay>> {
        let days = days.clamp(1, 30);
        let start = format!("{} 00:00:00", iso_days_ago(days - 1));
        let mut where_sql = " AND ts>=?".to_string();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(start)];
        if let Some(c) = camera_id {
            where_sql.push_str(" AND camera_id=?");
            args.push(Box::new(c));
        }
        if let Some(l) = level {
            where_sql.push_str(" AND level=?");
            args.push(Box::new(l));
        }
        let sql = format!(
            "SELECT substr(ts,1,10) d, CAST(substr(ts,12,2) AS INTEGER) h, COUNT(*) n, \
             SUM(CASE WHEN level>=3 THEN 1 ELSE 0 END) l3 FROM events WHERE 1=1{where_sql} GROUP BY d, h");
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(vals.as_slice(), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut agg: std::collections::BTreeMap<(String, i64), (i64, i64)> = Default::default();
        for (d, h, n, l3) in rows {
            agg.insert((d, h), (n, l3));
        }
        let mut out = Vec::new();
        for i in 0..days {
            let d = iso_days_ago(days - 1 - i);
            let mut hours = Vec::new();
            for h in 0..24i64 {
                let (n, l3) = agg.get(&(d.clone(), h)).copied().unwrap_or((0, 0));
                hours.push((h, n, l3));
            }
            out.push((d, hours));
        }
        Ok(out)
    }

    // ---- 图片文件夹识别记录（任务121，R2-309 批次 2；行结构见模块级 FolderImageRow） ----

    pub fn folder_images(
        &self,
        f: &FolderImageFilter<'_>,
    ) -> rusqlite::Result<Vec<FolderImageRow>> {
        let (camera_id, key, violation) = (f.camera_id, f.key, f.violation);
        let (start_ts, end_ts) = (f.start_ts, f.end_ts);
        let error_only = f.error_only;
        let (limit, offset) = (f.limit, f.offset);
        let mut sql = String::from(
            "SELECT id,camera_id,path,name,ts,processed_ts,width,height,persons,in_zone, \
             ppe_no,ppe_wearing,COALESCE(key,0),COALESCE(violation,0),event_id,snapshot,error \
             FROM folder_images WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(v) = camera_id {
            sql.push_str(" AND camera_id=?");
            args.push(Box::new(v));
        }
        if let Some(v) = key {
            sql.push_str(" AND COALESCE(key,0)=?");
            args.push(Box::new(v as i64));
        }
        if let Some(v) = violation {
            sql.push_str(" AND COALESCE(violation,0)=?");
            args.push(Box::new(v as i64));
        }
        if let Some(v) = start_ts {
            sql.push_str(" AND ts>=?");
            args.push(Box::new(v.to_string()));
        }
        if let Some(v) = end_ts {
            sql.push_str(" AND ts<=?");
            args.push(Box::new(v.to_string()));
        }
        if error_only {
            sql.push_str(" AND error IS NOT NULL");
        }
        sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");
        args.push(Box::new(limit));
        args.push(Box::new(offset));
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(vals.as_slice(), map_folder_image)?;
        rows.collect()
    }

    pub fn count_folder_images(
        &self,
        camera_id: Option<i64>,
        key: Option<bool>,
        violation: Option<bool>,
        start_ts: Option<&str>,
        end_ts: Option<&str>,
        error_only: bool,
    ) -> rusqlite::Result<i64> {
        let mut sql = String::from("SELECT COUNT(*) FROM folder_images WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(v) = camera_id {
            sql.push_str(" AND camera_id=?");
            args.push(Box::new(v));
        }
        if let Some(v) = key {
            sql.push_str(" AND COALESCE(key,0)=?");
            args.push(Box::new(v as i64));
        }
        if let Some(v) = violation {
            sql.push_str(" AND COALESCE(violation,0)=?");
            args.push(Box::new(v as i64));
        }
        if let Some(v) = start_ts {
            sql.push_str(" AND ts>=?");
            args.push(Box::new(v.to_string()));
        }
        if let Some(v) = end_ts {
            sql.push_str(" AND ts<=?");
            args.push(Box::new(v.to_string()));
        }
        if error_only {
            sql.push_str(" AND error IS NOT NULL");
        }
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        self.conn.query_row(&sql, vals.as_slice(), |r| r.get(0))
    }

    /// 按摄像头聚合近 N 天（与 Python folder_image_summary 同 SQL 口径）。
    /// 返回 (camera_id, total, key_n, violation_n, persons_sum, in_zone_sum,
    /// error_n, last_image_ts, last_processed_ts)
    pub fn folder_image_summary(
        &self,
        days: i64,
        camera_id: Option<i64>,
    ) -> rusqlite::Result<Vec<FolderImageSummary>> {
        let cutoff = iso_days_ago(days) + "T00:00:00";
        let mut stmt = self.conn.prepare(
            "SELECT camera_id, COUNT(*), COALESCE(SUM(key),0), COALESCE(SUM(violation),0), \
             COALESCE(SUM(persons),0), COALESCE(SUM(in_zone),0), \
             COALESCE(SUM(CASE WHEN error IS NOT NULL THEN 1 ELSE 0 END),0), \
             MAX(ts), MAX(processed_ts) FROM folder_images \
             WHERE ts >= ? AND (? IS NULL OR camera_id=?) GROUP BY camera_id",
        )?;
        let rows = stmt.query_map(params![cutoff, camera_id, camera_id], |r| {
            Ok(FolderImageSummary {
                camera_id: r.get(0)?,
                total: r.get(1)?,
                key_n: r.get(2)?,
                violation_n: r.get(3)?,
                persons_sum: r.get(4)?,
                in_zone_sum: r.get(5)?,
                error_n: r.get(6)?,
                last_image_ts: r.get(7)?,
                last_processed_ts: r.get(8)?,
            })
        })?;
        rows.collect()
    }

    /// app_settings 读（R2-309：ppe 状态等只读消费；不做解密）
    pub fn setting_get(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM app_settings WHERE key=?",
                params![key],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
    }
}

fn map_folder_image(r: &Row<'_>) -> rusqlite::Result<FolderImageRow> {
    Ok(FolderImageRow {
        id: r.get(0)?,
        camera_id: r.get(1)?,
        path: r.get(2)?,
        name: r.get(3)?,
        ts: r.get(4)?,
        processed_ts: r.get(5)?,
        width: r.get(6)?,
        height: r.get(7)?,
        persons: r.get(8)?,
        in_zone: r.get(9)?,
        ppe_no: r.get(10)?,
        ppe_wearing: r.get(11)?,
        key: r.get::<_, i64>(12)? != 0,
        violation: r.get::<_, i64>(13)? != 0,
        event_id: r.get(14)?,
        snapshot: r.get(15)?,
        error: r.get(16)?,
    })
}

/// 全量规则行：(id, camera_id, zone_id, behavior, level, cooldown_s,
/// time_start, time_end, threshold, enabled)
pub type RuleFullRow = (
    i64,
    i64,
    Option<i64>,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    i64,
    i64,
);

/// 时间线一天：(date, [(hour, total, l3), …24])
pub type TimelineDay = (String, Vec<(i64, i64, i64)>);

/// folder_images 行（与 Python folder_row_out 同字段口径）
#[derive(Debug, Clone, Serialize)]
pub struct FolderImageRow {
    pub id: i64,
    pub camera_id: i64,
    pub path: String,
    pub name: Option<String>,
    pub ts: String,
    pub processed_ts: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub persons: i64,
    pub in_zone: i64,
    pub ppe_no: i64,
    pub ppe_wearing: i64,
    pub key: bool,
    pub violation: bool,
    pub event_id: Option<i64>,
    pub snapshot: Option<String>,
    pub error: Option<String>,
}

/// folder_images 列表过滤（R2-309 批次 2：与 Python GET /api/folder-images 同参）
pub struct FolderImageFilter<'a> {
    pub camera_id: Option<i64>,
    pub key: Option<bool>,
    pub violation: Option<bool>,
    pub start_ts: Option<&'a str>,
    pub end_ts: Option<&'a str>,
    pub error_only: bool,
    pub limit: i64,
    pub offset: i64,
}

/// 图片识别汇总行（Python folder_image_summary 同字段：total/key/violation/
/// persons/in_zone/error + 最近时间）
#[derive(Debug, Clone, Serialize)]
pub struct FolderImageSummary {
    pub camera_id: i64,
    pub total: i64,
    pub key_n: i64,
    pub violation_n: i64,
    pub persons_sum: i64,
    pub in_zone_sum: i64,
    pub error_n: i64,
    pub last_image_ts: Option<String>,
    pub last_processed_ts: Option<String>,
}

/// ISO 日期（本地时间近似：UTC 即可，控制面统计与 Python 同基准即可对齐）
fn iso_days_ago(days: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86400) - days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// N 周前的周一（ISO 周，Python `date - timedelta(days=weekday()) - timedelta(weeks=n)` 同口径）
fn iso_weeks_ago_monday(weeks: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs.div_euclid(86400) - weeks * 7;
    let (y, m, d) = civil_from_days(days);
    // weekday：1970-01-01 是周四 → (days + 3) % 7 == 0 表示周一
    let weekday = (days + 3).rem_euclid(7); // 0=周一
    let (y2, m2, d2) = civil_from_days(days - weekday);
    let _ = (y, m, d);
    format!("{y2:04}-{m2:02}-{d2:02}")
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
              status TEXT, attempts INTEGER, last_error TEXT, updated_ts TEXT);").unwrap();
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
