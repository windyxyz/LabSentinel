//! ls-store（T-332/ADR-0008）：与 Python `store.py` 共用同一 SQLite 文件与 schema。
//!
//! Python 运行时退役（2026-11-14）后迁移链归 Rust：`ensure_schema` 自举全新库，
//! `migrate` 幂等推进存量库（v13→v14 起）。v14 = UX2 cascade（events 加
//! vlm_verdict/vlm_ts/vlm_suppressed 三列）。

use rusqlite::{params, Connection, Row};
use serde::Serialize;

mod faces;
mod operations;
pub use faces::{FacePerson, FaceTemplate};
pub use operations::{ComplianceCounts, DeliveryWithHistory, EventNote};

/// schema 权威编号（v16 = LS-054 dataset_samples + share_links，2026-11-22）；
/// 文档/诊断常量，实际迁移执行见 ensure_schema/migrate。
pub const SCHEMA_VERSION_LATEST: i64 = 22;

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
    pub handled_by: Option<String>,
    pub handled_ts: Option<String>,
    pub ai_review: Option<String>,
    pub ai_review_ts: Option<String>,
    pub ai_review_model: Option<String>,
    /// UX2 cascade：VLM 复核判定（完整 JSON 文本；None=未复核）
    pub vlm_verdict: Option<String>,
    /// UX2 cascade：复核时间
    pub vlm_ts: Option<String>,
    /// UX2 cascade：VLM 否定且策略=suppress → 1（不投递告警渠道）
    pub vlm_suppressed: i64,
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

const EVENT_COLUMNS: &str = "id,ts,camera_id,type,level,track_id,detail,snapshot,video,\
    COALESCE(status,'new'),vlm_verdict,vlm_ts,COALESCE(vlm_suppressed,0),handled_by,handled_ts,\
    ai_review,ai_review_ts,ai_review_model";

pub const ALERT_EVENTS_SQL: &str = "type NOT IN ('photo_check','video_check')";

pub const AI_VERDICT_SQL: &str = "COALESCE(
    CASE WHEN json_valid(ai_review) THEN CASE json_extract(ai_review,'$.verdict')
        WHEN 'likely_true' THEN 'likely_true' WHEN 'likely_false' THEN 'likely_false'
        WHEN 'uncertain' THEN 'uncertain' END END,
    CASE WHEN json_valid(vlm_verdict) THEN CASE json_extract(vlm_verdict,'$.verdict')
        WHEN 'likely_true' THEN 'likely_true' WHEN 'likely_false' THEN 'likely_false'
        WHEN 'uncertain' THEN 'uncertain' END END,
    'none')";

#[derive(Debug, Clone, Serialize)]
pub struct Camera {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub type_: String,
    pub status: String,
    pub group_name: String,
    pub analyzer: String,
    pub analysis_source: String,
    pub analysis_shared: bool,
}

impl Camera {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            source: row.get(2)?,
            type_: row.get(3)?,
            status: row.get(4)?,
            group_name: row.get(5)?,
            analyzer: row.get(6)?,
            analysis_source: row.get(7)?,
            analysis_shared: row.get(8)?,
        })
    }
}

pub struct Store {
    conn: Connection,
}

pub struct ReportSummary {
    pub total: i64,
    pub violation: i64,
    pub unhandled: i64,
    pub top: Vec<(i64, i64)>,
}

impl Store {
    /// 打开已有库（Python 建库路径）；空文件则由 Python 负责首次建库。
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // LS-041（存储安全）：必须真正开启 WAL 模式才能与 synchronous=NORMAL 配对
        // （DELETE 模式下 synchronous=NORMAL 断电不安全且并发读写互锁 SQLITE_BUSY）。
        // 内存库 (:memory:) 不支持 WAL，忽略返回值不报错。
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(Store { conn })
    }

    /// 只读连接访问（T-330 维护作业：retention/escalation 需要游标分页与多表清理，
    /// 逐方法封装会让 ls-store 膨胀；SQL 仍由本 crate 之外的作业层集中维护）。
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// RA5（ADR-0011 独立部署缺口）：Rust 形态自举建表——v13 全量 schema
    /// 的幂等 CREATE（IF NOT EXISTS；与 Python 已建库共存安全，已有表不动）。
    /// 迁移链仍归 Python（ADR-0008）——本方法只覆盖"零 Python 部署"的初次
    /// 自举，ALTER 类列迁移不做（老库由 Python 升级）。
    /// 返回 true = 本次执行了建表（全新库）。
    pub fn ensure_schema(&self) -> rusqlite::Result<bool> {
        let fresh = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='events'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n == 0)
            .unwrap_or(true);
        if !fresh {
            return Ok(false);
        }
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version(version INTEGER PRIMARY KEY, applied_at TEXT);
             INSERT OR IGNORE INTO schema_version VALUES(22, 'rust-bootstrap');
             CREATE TABLE IF NOT EXISTS cameras(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, type TEXT, \
               status TEXT DEFAULT 'stopped', group_name TEXT DEFAULT '', \
               analyzer TEXT DEFAULT 'default', analysis_source TEXT NOT NULL DEFAULT '', \
               analysis_shared INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS zones(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, name TEXT, polygon TEXT);
             CREATE TABLE IF NOT EXISTS rules(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, zone_id INTEGER, \
               behavior TEXT, level INTEGER, cooldown_s INTEGER, \
               time_start TEXT, time_end TEXT, threshold INTEGER, enabled INTEGER, \
               days_of_week TEXT DEFAULT '1,2,3,4,5,6,7', holiday_only INTEGER DEFAULT 0);
             CREATE TABLE IF NOT EXISTS calendar_exceptions(\
               date TEXT PRIMARY KEY, mode TEXT NOT NULL, note TEXT);
             CREATE TABLE IF NOT EXISTS rule_templates(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL, \
               config_json TEXT NOT NULL, created_ts TEXT, updated_ts TEXT);
             CREATE TABLE IF NOT EXISTS events(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, type TEXT, \
               level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT, \
               status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT, \
               escalated_ts TEXT, ai_review TEXT, ai_review_ts TEXT, ai_review_model TEXT, \
               vlm_verdict TEXT, vlm_ts TEXT, vlm_suppressed INTEGER DEFAULT 0);
             CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
             CREATE INDEX IF NOT EXISTS idx_events_camera ON events(camera_id);
             CREATE TABLE IF NOT EXISTS face_consents(subject TEXT PRIMARY KEY, ts TEXT);
             CREATE TABLE IF NOT EXISTS alert_deliveries(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER, channel TEXT, \
               status TEXT DEFAULT 'pending', attempts INTEGER DEFAULT 0, \
               last_error TEXT, updated_ts TEXT);
             CREATE TABLE IF NOT EXISTS alert_silences(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, scope TEXT NOT NULL, target TEXT, \
               reason TEXT, created_ts TEXT NOT NULL, until_ts TEXT NOT NULL, created_by TEXT);
             CREATE TABLE IF NOT EXISTS audit_log(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, request_id TEXT, \
               actor TEXT, action TEXT NOT NULL, target TEXT, status_code INTEGER, detail TEXT);
             CREATE TABLE IF NOT EXISTS folder_images(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER NOT NULL, path TEXT NOT NULL, \
               name TEXT, mtime_ns INTEGER NOT NULL DEFAULT 0, size INTEGER NOT NULL DEFAULT 0, \
               ts TEXT NOT NULL, processed_ts TEXT NOT NULL, width INTEGER, height INTEGER, \
               persons INTEGER DEFAULT 0, in_zone INTEGER DEFAULT 0, ppe_no INTEGER DEFAULT 0, \
               ppe_wearing INTEGER DEFAULT 0, zones TEXT, violations TEXT, key INTEGER DEFAULT 0, \
               violation INTEGER DEFAULT 0, event_id INTEGER, snapshot TEXT, error TEXT);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_folder_images_uniq \
               ON folder_images(camera_id, path, mtime_ns, size);
             CREATE TABLE IF NOT EXISTS app_settings(\
               key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_ts TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS devices(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
               platform TEXT DEFAULT '', token_hash TEXT NOT NULL UNIQUE, \
               created_ts TEXT NOT NULL, last_seen_ts TEXT, revoked INTEGER DEFAULT 0);
             CREATE TABLE IF NOT EXISTS device_access(\
               device_id INTEGER PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE, \
               policy_json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS dataset_samples(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, source_type TEXT NOT NULL, \
               source_id TEXT NOT NULL, camera_id INTEGER, image_path TEXT NOT NULL, \
               width INTEGER NOT NULL, height INTEGER NOT NULL, \
               status TEXT NOT NULL DEFAULT 'pending', boxes_json TEXT NOT NULL DEFAULT '[]', \
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS share_links(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
               token_hash TEXT NOT NULL UNIQUE, ttl_hours INTEGER NOT NULL, \
               created_ts TEXT NOT NULL, expires_ts TEXT NOT NULL, \
               revoked INTEGER DEFAULT 0);",
        )?;
        self.conn.execute_batch(faces::SCHEMA)?;
        self.conn.execute_batch(operations::SCHEMA)?;
        Ok(true)
    }

    /// 双向兼容自检：schema_version 表存在且 ≥ 当前代码期望。
    pub fn schema_version(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
    }

    /// UX2（2026-11-20）：存量库列迁移——Python 退役后迁移链归 Rust（ADR-0008
    /// 口径随 Python 退役失效）。v13→v14：events 加 cascade 判定三列。幂等：
    /// 按 PRAGMA table_info 探列，已存在则跳过（多实例/重复启动安全）。
    /// 返回应用后的 schema 版本。
    pub fn migrate(&self) -> rusqlite::Result<i64> {
        let has_vlm_verdict: bool = self
            .conn
            .prepare("PRAGMA table_info(events)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|c| c == "vlm_verdict");
        if !has_vlm_verdict {
            self.conn.execute_batch(
                "ALTER TABLE events ADD COLUMN vlm_verdict TEXT;\
                 ALTER TABLE events ADD COLUMN vlm_ts TEXT;\
                 ALTER TABLE events ADD COLUMN vlm_suppressed INTEGER DEFAULT 0;",
            )?;
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_version(version, applied_at) VALUES(14, 'rust-ux2-cascade')",
            [],
        )?;
        // v15（LS-041）：修复历史 add_rule 硬编码写入 days_of_week='0' 导致规则永不触发的存量坏数据
        let _ = self.conn.execute(
            "UPDATE rules SET days_of_week='1,2,3,4,5,6,7' WHERE days_of_week='0' OR days_of_week IS NULL",
            [],
        );
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_version(version, applied_at) VALUES(15, 'rust-ls041-days-of-week')",
            [],
        )?;
        // v16（LS-054）：数据飞轮样本池 + 只读值守分享链接（幂等建表）
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS dataset_samples(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, source_type TEXT NOT NULL, \
               source_id TEXT NOT NULL, camera_id INTEGER, image_path TEXT NOT NULL, \
               width INTEGER NOT NULL, height INTEGER NOT NULL, \
               status TEXT NOT NULL DEFAULT 'pending', boxes_json TEXT NOT NULL DEFAULT '[]', \
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS idx_dataset_samples_status ON dataset_samples(status);
             CREATE TABLE IF NOT EXISTS share_links(\
               id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
               token_hash TEXT NOT NULL UNIQUE, ttl_hours INTEGER NOT NULL, \
               created_ts TEXT NOT NULL, expires_ts TEXT NOT NULL, \
               revoked INTEGER DEFAULT 0);",
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_version(version, applied_at) VALUES(16, 'rust-ls054-dataset-share')",
            [],
        )?;
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS device_access(\
               device_id INTEGER PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE, \
               policy_json TEXT NOT NULL); \
             INSERT OR IGNORE INTO schema_version(version, applied_at) VALUES(17, 'rust-ls063-device-access');",
        )?;
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let has_analysis_source = transaction
            .prepare("PRAGMA table_info(cameras)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|column| column == "analysis_source");
        if !has_analysis_source {
            transaction.execute_batch(
                "ALTER TABLE cameras ADD COLUMN analysis_source TEXT NOT NULL DEFAULT ''; \
                 ALTER TABLE cameras ADD COLUMN analysis_shared INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        transaction.execute(
            "INSERT OR IGNORE INTO schema_version VALUES(18, 'rust-ls065-analysis-source')",
            [],
        )?;
        transaction.commit()?;
        self.conn.execute_batch(faces::SCHEMA)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_version VALUES(19, 'rust-ls066-face-models')",
            [],
        )?;
        self.conn.execute_batch(operations::SCHEMA)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_version VALUES(20, 'rust-ls067-operations')",
            [],
        )?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let event_columns = tx
            .prepare("PRAGMA table_info(events)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
        for column in ["ai_review", "ai_review_ts", "ai_review_model"] {
            if !event_columns.contains(column) {
                tx.execute(&format!("ALTER TABLE events ADD COLUMN {column} TEXT"), [])?;
            }
        }
        let has_folder_results: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='folder_images')",
            [], |row| row.get(0),
        )?;
        if tx.execute(
            "INSERT OR IGNORE INTO schema_version VALUES(21, 'rust-ls071-file-checks')",
            [],
        )? > 0
            && has_folder_results
        {
            tx.execute(
                    "INSERT INTO events(ts,camera_id,type,level,track_id,detail,status)
                     SELECT f.processed_ts,f.camera_id,'photo_check',0,0,
                     CASE WHEN COALESCE(f.error,'')<>'' THEN
                       printf('补录图片「%s」检查失败。%s', COALESCE(f.name,'图片'), f.error)
                     ELSE
                       printf('补录图片「%s」检查完成。检出 %d 人，已穿实验服 %d、未穿 %d。%s',
                         COALESCE(f.name,'图片'), f.persons, f.ppe_wearing, f.ppe_no,
                         CASE WHEN f.violation<>0 THEN '有违规分类，详见图片识别结果。' ELSE '未检出明确违规。' END)
                     END, 'recorded'
                     FROM folder_images f JOIN cameras c ON c.id=f.camera_id
                     WHERE f.width IS NOT NULL AND COALESCE(f.processed_ts,'')<>''",
                    [],
                )?;
        }
        if tx.execute(
            "INSERT OR IGNORE INTO schema_version VALUES(22, 'rust-ls072-check-media')",
            [],
        )? > 0
            && has_folder_results
        {
            tx.execute(
                "INSERT OR IGNORE INTO photo_check_images(event_id,image_id)
                 SELECT e.id,MIN(f.id) FROM events e JOIN folder_images f
                      ON f.camera_id=e.camera_id AND f.processed_ts=e.ts
                 WHERE e.type='photo_check' AND
                      (instr(e.detail,printf('图片「%s」检查',COALESCE(f.name,'图片')))=1 OR
                       instr(e.detail,printf('补录图片「%s」检查',COALESCE(f.name,'图片')))=1)
                 GROUP BY e.id HAVING COUNT(*)=1",
                [],
            )?;
        }
        tx.commit()?;
        self.schema_version()
    }

    /// UX3/UX4 优雅关停：WAL checkpoint 回主库（托盘退出/shutdown 语义——
    /// 停 workers 后调用，保证 -wal 文件合并后再退进程）。
    pub fn checkpoint_wal(&self) -> rusqlite::Result<()> {
        self.conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                r.get::<_, i64>(0)
            })?;
        Ok(())
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

    pub fn add_check_event(
        &self,
        camera_id: i64,
        kind: &str,
        detail: &str,
        ts: &str,
        snapshot: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO events(ts,camera_id,type,level,track_id,detail,status,snapshot)
             VALUES(?,?,?,0,0,?,'recorded',?)",
            params![ts, camera_id, kind, detail, snapshot],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn check_image_path(&self, event_id: i64) -> rusqlite::Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row(
                "SELECT f.path FROM photo_check_images p JOIN folder_images f ON f.id=p.image_id
             JOIN events e ON e.id=p.event_id AND e.camera_id=f.camera_id
             WHERE e.id=? AND e.type='photo_check'",
                [event_id],
                |row| row.get(0),
            )
            .optional()
    }

    /// 今日事件按级别计数（RA2 对齐 Python today_event_counts：本地日 ts 前缀
    /// 匹配——SQLite `localtime` 取 OS 时区，与 Python datetime.now() 同口径）。
    pub fn today_event_counts(&self) -> rusqlite::Result<Vec<(i64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT level, COUNT(*) n FROM events \
             WHERE ts LIKE date('now','localtime') || '%' GROUP BY level",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// 今日已处置告警数。
    pub fn count_events_today_handled(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM events \
             WHERE ts LIKE date('now','localtime') || '%' AND status NOT IN ('new','recorded')",
            [],
            |r| r.get(0),
        )
    }

    /// 投递行计数（RA2 对齐 Python delivery_counts）：(pending, failed, ok)。
    pub fn delivery_counts(&self) -> rusqlite::Result<(i64, i64, i64)> {
        let mut stmt = self
            .conn
            .prepare("SELECT status, COUNT(*) FROM alert_deliveries GROUP BY status")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut d = std::collections::BTreeMap::new();
        for r in rows {
            let (k, v) = r?;
            d.insert(k, v);
        }
        Ok((
            d.get("pending").copied().unwrap_or(0),
            d.get("failed").copied().unwrap_or(0),
            d.get("ok").copied().unwrap_or(0),
        ))
    }

    /// 事件视频回填（RA1.1，对齐 Python store.set_event_video：存 data_dir
    /// 相对路径，serve_event_media 用 data_dir.join 解析）。
    pub fn set_event_video(&self, eid: i64, path: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE events SET video=? WHERE id=?", params![path, eid])?;
        Ok(())
    }

    /// UX2 cascade：VLM 复核判定回填（verdict 为完整判定 JSON；verdict=NULL
    /// 语义=未复核）。vlm_ts ISO 时间由调用方生成。
    pub fn set_event_vlm(&self, eid: i64, verdict_json: &str, ts: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE events SET vlm_verdict=?, vlm_ts=? WHERE id=?",
            params![verdict_json, ts, eid],
        )?;
        Ok(())
    }

    /// UX2 cascade：VLM 明确否定且 on_negative=suppress → 标记抑制（不投递）。
    pub fn set_event_vlm_suppressed(&self, eid: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE events SET vlm_suppressed=1 WHERE id=?", [eid])?;
        Ok(())
    }

    pub fn events(
        &self,
        limit: i64,
        offset: i64,
        camera_id: Option<i64>,
        status: Option<&str>,
    ) -> rusqlite::Result<Vec<Event>> {
        self.events_scoped(limit, offset, camera_id, status, None)
    }

    pub fn events_scoped(
        &self,
        limit: i64,
        offset: i64,
        camera_id: Option<i64>,
        status: Option<&str>,
        allowed_cameras: Option<&[i64]>,
    ) -> rusqlite::Result<Vec<Event>> {
        self.events_query(&EventsFilter {
            limit,
            offset,
            camera_id,
            status,
            allowed_cameras,
            ..Default::default()
        })
    }

    pub fn event_by_id(&self, id: i64) -> rusqlite::Result<Option<Event>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {EVENT_COLUMNS} FROM events WHERE id=?"))?;
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
    fn camera_query(&self, suffix: &str) -> rusqlite::Result<rusqlite::Statement<'_>> {
        let fields =
            "id,name,source,type,status,COALESCE(group_name,''),COALESCE(analyzer,'python')";
        self.conn
            .prepare(&format!(
                "SELECT {fields},analysis_source,analysis_shared FROM cameras {suffix}"
            ))
            .or_else(|error| {
                if error.to_string().starts_with("no such column: analysis_") {
                    self.conn
                        .prepare(&format!("SELECT {fields},'',0 FROM cameras {suffix}"))
                } else {
                    Err(error)
                }
            })
    }

    pub fn cameras(&self) -> rusqlite::Result<Vec<Camera>> {
        let mut stmt = self.camera_query("ORDER BY id")?;
        let rows = stmt.query_map([], Camera::from_row)?;
        rows.collect()
    }

    pub fn camera(&self, id: i64) -> rusqlite::Result<Option<Camera>> {
        let mut stmt = self.camera_query("WHERE id=?")?;
        use rusqlite::OptionalExtension;
        stmt.query_row([id], Camera::from_row).optional()
    }

    pub fn set_camera_analysis_source(
        &self,
        id: i64,
        source: &str,
        shared: bool,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE cameras SET analysis_source=?,analysis_shared=? WHERE id=? AND type='rtsp'",
            params![source, shared, id],
        )
    }

    /// RA2-S6：人脸同意记录（合规留痕，只存 subject/ts，不落人脸图片）。
    pub fn add_face_consent(&self, subject: &str, ts: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO face_consents(subject,ts) VALUES(?,?)",
            params![subject, ts],
        )?;
        Ok(())
    }

    pub fn face_consents(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT subject,ts FROM face_consents ORDER BY ts")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    pub fn face_consent_ts(&self, subject: &str) -> rusqlite::Result<Option<String>> {
        match self.conn.query_row(
            "SELECT ts FROM face_consents WHERE subject=?",
            params![subject],
            |r| r.get(0),
        ) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn clear_face_consents(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM face_consents", [])?;
        Ok(())
    }

    pub fn delete_face_consent(&self, subject: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM face_consents WHERE subject=?", [subject])?;
        Ok(())
    }

    /// RA2-S5：事件 AI 复核标记回写（对齐 Python store.set_ai_review：
    /// 只写 ai_review/ai_review_ts/ai_review_model，绝不改 status）。
    pub fn set_ai_review(
        &self,
        eid: i64,
        review_json: &str,
        ts: &str,
        model: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE events SET ai_review=?, ai_review_ts=?, ai_review_model=? WHERE id=?",
            params![review_json, ts, model, eid],
        )?;
        Ok(())
    }

    pub fn set_camera_status(&self, id: i64, status: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE cameras SET status=? WHERE id=?",
            params![status, id],
        )
    }

    /// L3 控制面：新增摄像头（列/默认值与历史 schema 兼容；新相机默认本地检测）
    pub fn conn_add_camera(&self, name: &str, source: &str, type_: &str) -> rusqlite::Result<i64> {
        self.add_camera_with_sources(name, source, type_, "", "")
    }

    pub fn add_camera_with_sources(
        &self,
        name: &str,
        source: &str,
        type_: &str,
        group: &str,
        analysis_source: &str,
    ) -> rusqlite::Result<i64> {
        if analysis_source.is_empty() {
            self.conn.execute(
                "INSERT INTO cameras(name,source,type,group_name,analyzer) VALUES(?,?,?,?,'default')",
                params![name, source, type_, group],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO cameras(name,source,type,group_name,analysis_source,analyzer)
                 VALUES(?,?,?,?,?,'default')",
                params![name, source, type_, group, analysis_source],
            )?;
        }
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
    #[serde(skip)]
    pub policy_json: Option<String>,
}

fn map_device(row: &rusqlite::Row<'_>) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get(0)?,
        name: row.get(1)?,
        platform: row.get(2)?,
        created_ts: row.get(3)?,
        last_seen_ts: row.get(4)?,
        revoked: row.get::<_, i64>(5)? != 0,
        policy_json: row.get(6)?,
    })
}

fn append_camera_scope(
    sql: &mut String,
    args: &mut Vec<Box<dyn rusqlite::ToSql>>,
    scope: Option<&[i64]>,
) {
    if let Some(ids) = scope {
        if ids.is_empty() {
            sql.push_str(" AND 0");
            return;
        }
        sql.push_str(" AND camera_id IN (");
        sql.push_str(&vec!["?"; ids.len()].join(","));
        sql.push(')');
        args.extend(
            ids.iter()
                .map(|id| Box::new(*id) as Box<dyn rusqlite::ToSql>),
        );
    }
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

/// 待重试投递：(delivery_id, event_id, channel, attempts)
pub type RetryableDelivery = (i64, i64, String, i64);
/// 单条投递：(event_id, status, attempts, last_error)
pub type DeliveryRow = (i64, String, i64, Option<String>);

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

/// LS-054：数据飞轮样本（人工标注工作台）。
#[derive(Debug, Clone, Serialize)]
pub struct DatasetSample {
    pub id: i64,
    pub source_type: String,
    pub source_id: String,
    pub camera_id: Option<i64>,
    pub image_path: String,
    pub width: i64,
    pub height: i64,
    pub status: String,
    pub boxes_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// LS-054：只读值守分享链接。
#[derive(Debug, Clone, Serialize)]
pub struct ShareLink {
    pub id: i64,
    pub name: String,
    pub ttl_hours: i64,
    pub created_ts: String,
    pub expires_ts: String,
    pub revoked: bool,
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
    pub category: Option<&'a str>,
    pub start_ts: Option<&'a str>,
    pub end_ts: Option<&'a str>,
    pub search: Option<&'a str>,
    pub ai_verdict: Option<&'a str>,
    pub allowed_cameras: Option<&'a [i64]>,
    pub limit: i64,
    pub offset: i64,
}

impl EventsFilter<'_> {
    fn conditions(&self) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
        let mut sql = String::from(" WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        match self.category {
            Some("check") => sql.push_str(" AND type IN ('photo_check','video_check')"),
            Some("alert") => {
                sql.push_str(" AND ");
                sql.push_str(ALERT_EVENTS_SQL);
            }
            _ => {}
        }
        if let Some(v) = self.camera_id {
            sql.push_str(" AND camera_id=?");
            args.push(Box::new(v));
        }
        if let Some(v) = self.level {
            sql.push_str(" AND level=?");
            args.push(Box::new(v));
        }
        for (condition, value) in [
            (" AND COALESCE(status,'new')=?", self.status),
            (" AND type=?", self.type_),
            (" AND ts>=?", self.start_ts),
        ] {
            if let Some(value) = value {
                sql.push_str(condition);
                args.push(Box::new(value.to_owned()));
            }
        }
        if let Some(v) = self.end_ts {
            if v.len() == 10 {
                sql.push_str(" AND ts < date(?, '+1 day')");
                args.push(Box::new(v.to_owned()));
            } else {
                sql.push_str(" AND ts<=?");
                args.push(Box::new(v.to_owned()));
            }
        }
        if let Some(search) = self.search {
            sql.push_str(
                " AND (instr(lower(detail), lower(?))>0 OR instr(lower(type), lower(?))>0)",
            );
            args.extend([
                Box::new(search.to_owned()) as Box<dyn rusqlite::ToSql>,
                Box::new(search.to_owned()),
            ]);
        }
        if let Some(verdict) = self.ai_verdict {
            sql.push_str(" AND ");
            sql.push_str(AI_VERDICT_SQL);
            sql.push_str("=?");
            args.push(Box::new(verdict.to_owned()));
        }
        append_camera_scope(&mut sql, &mut args, self.allowed_cameras);
        (sql, args)
    }
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

    /// LS-052：更新安全区多边形（拖拽顶点后保存；名称不变）。
    pub fn update_zone_polygon(&self, id: i64, polygon_json: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE zones SET polygon=? WHERE id=?",
            params![polygon_json, id],
        )
    }

    // ---- rules ----
    pub fn rules_all(&self) -> rusqlite::Result<Vec<RuleFullRow>> {
        // RA1.1（ADR-0011）：补选 days_of_week——Python store.rules 读该列
        // （weekday/时段过滤依赖它），原缺口会让时段规则周末误触发（静默差异）。
        // RA4.2（退役批）：补选 holiday_only——pipeline.holiday_allowed 消费。
        let mut stmt = self.conn.prepare(
            "SELECT id,camera_id,zone_id,behavior,level,COALESCE(cooldown_s,0),time_start,time_end,COALESCE(threshold,0),enabled,days_of_week,holiday_only FROM rules ORDER BY camera_id,id")?;
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
                r.get(10)?,
                r.get::<_, i64>(11)? != 0,
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
        self.add_rule_with_schedule(
            camera_id,
            zone_id,
            behavior,
            level,
            cooldown_s,
            time_start,
            time_end,
            threshold,
            enabled,
            Some("1,2,3,4,5,6,7"),
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_rule_with_schedule(
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
        days_of_week: Option<&str>,
        holiday_only: bool,
    ) -> rusqlite::Result<i64> {
        let dow = days_of_week
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "0")
            .unwrap_or("1,2,3,4,5,6,7");
        self.conn.execute(
            "INSERT INTO rules(camera_id,zone_id,behavior,level,cooldown_s,time_start,time_end,threshold,enabled,days_of_week,holiday_only) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                camera_id,
                zone_id,
                behavior,
                level,
                cooldown_s,
                time_start,
                time_end,
                threshold,
                enabled as i64,
                dow,
                holiday_only as i64
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_rule(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM rules WHERE id=?", params![id])
    }

    // ---- events（过滤 / 批量处置） ----
    pub fn events_query(&self, f: &EventsFilter<'_>) -> rusqlite::Result<Vec<Event>> {
        let (conditions, mut args) = f.conditions();
        args.push(Box::new(f.limit));
        args.push(Box::new(f.offset));
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM events{conditions} ORDER BY id DESC LIMIT ? OFFSET ?"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), map_event)?;
        rows.collect()
    }

    pub fn events_query_count(&self, filter: &EventsFilter<'_>) -> rusqlite::Result<i64> {
        let (conditions, args) = filter.conditions();
        self.conn.query_row(
            &format!("SELECT COUNT(*) FROM events{conditions}"),
            rusqlite::params_from_iter(args),
            |row| row.get(0),
        )
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
            "SELECT d.id,d.name,COALESCE(d.platform,''),d.created_ts,d.last_seen_ts,COALESCE(d.revoked,0),a.policy_json \
             FROM devices d LEFT JOIN device_access a ON d.id=a.device_id ORDER BY d.id")?;
        let rows = stmt.query_map([], map_device)?;
        rows.collect()
    }

    pub fn device_by_hash(&self, token_hash: &str) -> rusqlite::Result<Option<Device>> {
        use rusqlite::OptionalExtension;
        self.conn.query_row(
            "SELECT d.id,d.name,COALESCE(d.platform,''),d.created_ts,d.last_seen_ts,COALESCE(d.revoked,0),a.policy_json \
             FROM devices d LEFT JOIN device_access a ON d.id=a.device_id WHERE d.token_hash=?",
            [token_hash], map_device,
        ).optional()
    }

    pub fn add_browser_device(
        &self,
        name: &str,
        token_hash: &str,
        policy: &str,
        previous: Option<i64>,
    ) -> rusqlite::Result<i64> {
        let transaction = self.conn.unchecked_transaction()?;
        let id = self.add_device(name, "web", token_hash)?;
        transaction.execute(
            "INSERT INTO device_access(device_id,policy_json) VALUES(?,?)",
            params![id, policy],
        )?;
        if let Some(previous) = previous {
            self.revoke_device(previous)?;
        }
        transaction.commit()?;
        Ok(id)
    }

    pub fn update_browser_device(
        &self,
        id: i64,
        name: &str,
        policy: &str,
    ) -> rusqlite::Result<bool> {
        let transaction = self.conn.unchecked_transaction()?;
        let updated = transaction.execute(
            "UPDATE devices SET name=? WHERE id=? AND COALESCE(revoked,0)=0 AND EXISTS(SELECT 1 FROM device_access WHERE device_id=?)",
            params![name, id, id],
        )?;
        if updated == 0 {
            return Ok(false);
        }
        transaction.execute(
            "UPDATE device_access SET policy_json=? WHERE device_id=?",
            params![policy, id],
        )?;
        transaction.commit()?;
        Ok(true)
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
        self.list_deliveries(status, None, limit)
    }

    fn list_deliveries(
        &self,
        status: Option<&str>,
        event_id: Option<i64>,
        limit: i64,
    ) -> rusqlite::Result<Vec<Delivery>> {
        let mut sql = String::from(
            "SELECT id,event_id,channel,COALESCE(status,'pending'),attempts,last_error,updated_ts FROM alert_deliveries WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(id) = event_id {
            sql.push_str(" AND event_id=?");
            args.push(Box::new(id));
        }
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

    /// 回收上次进程退出时留下的首发记录，沿用原始时间与重试退避。
    pub fn recover_interrupted_deliveries(&self) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE alert_deliveries SET status='retry', attempts=COALESCE(attempts,0),
             updated_ts=COALESCE(NULLIF(updated_ts,''),?),
             last_error=COALESCE(NULLIF(last_error,''),'上次进程退出时尚未记录发送结果')
             WHERE COALESCE(status,'pending')='pending'",
            [chrono_now()],
        )
    }

    pub fn retry_delivery(&self, id: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE alert_deliveries SET status='retry', attempts=0, last_error=NULL, updated_ts=? WHERE id=? AND status IN ('failed','retry','dead_letter')",
            params!["1970-01-01T00:00:00", id],
        )?;
        Ok(n > 0)
    }

    /// LS-053：取待重试投递（status IN retry/failed、attempts < max、且
    /// updated_ts 早于 cutoff）。返回 (delivery_id, event_id, channel, attempts)。
    pub fn retryable_deliveries(
        &self,
        cutoffs: &[String; 5],
        delivery_id: Option<i64>,
        limit: i64,
    ) -> rusqlite::Result<Vec<RetryableDelivery>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,event_id,channel,attempts FROM alert_deliveries \
             WHERE COALESCE(status,'pending') IN ('retry','failed') \
               AND (?6 IS NULL OR id=?6)
               AND updated_ts <= CASE attempts WHEN 0 THEN ?1 WHEN 1 THEN ?2
                   WHEN 2 THEN ?3 WHEN 3 THEN ?4 ELSE ?5 END
               ORDER BY updated_ts ASC, id ASC LIMIT ?7",
        )?;
        let rows = stmt.query_map(
            params![
                cutoffs[0],
                cutoffs[1],
                cutoffs[2],
                cutoffs[3],
                cutoffs[4],
                delivery_id,
                limit
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        rows.collect()
    }

    /// LS-053：单条投递详情（重试消费者回写用）。
    pub fn delivery_by_id(&self, id: i64) -> rusqlite::Result<Option<DeliveryRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, COALESCE(status,'pending'), attempts, last_error \
             FROM alert_deliveries WHERE id=?",
        )?;
        let mut rows = stmt.query_map(params![id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// LS-053：日报汇总——指定时间窗内事件总数/违规数/未处置数/Top3 摄像头。
    pub fn report_summary(&self, start_ts: &str, end_ts: &str) -> rusqlite::Result<ReportSummary> {
        let total: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE ts >= ? AND ts <= ?",
            params![start_ts, end_ts],
            |r| r.get(0),
        )?;
        let violation: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE ts >= ? AND ts <= ? AND level >= 2",
            params![start_ts, end_ts],
            |r| r.get(0),
        )?;
        let unhandled: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE ts >= ? AND ts <= ? AND COALESCE(status,'new')='new'",
            params![start_ts, end_ts],
            |r| r.get(0),
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT camera_id, COUNT(*) AS n FROM events WHERE ts >= ? AND ts <= ? \
             AND level >= 2 GROUP BY camera_id ORDER BY n DESC LIMIT 3",
        )?;
        let rows = stmt.query_map(params![start_ts, end_ts], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let top: Vec<(i64, i64)> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ReportSummary {
            total,
            violation,
            unhandled,
            top,
        })
    }

    // ---- LS-054：数据飞轮样本池 ----

    #[allow(clippy::too_many_arguments)]
    pub fn add_dataset_sample(
        &self,
        source_type: &str,
        source_id: &str,
        camera_id: Option<i64>,
        image_path: &str,
        width: i64,
        height: i64,
        boxes_json: &str,
    ) -> rusqlite::Result<i64> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT INTO dataset_samples(source_type,source_id,camera_id,image_path,width,height,status,boxes_json,created_at,updated_at) \
             VALUES(?,?,?,?,?,?,'pending',?,?,?)",
            params![source_type, source_id, camera_id, image_path, width, height, boxes_json, now, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn dataset_samples(
        &self,
        status: Option<&str>,
        limit: i64,
    ) -> rusqlite::Result<Vec<DatasetSample>> {
        let mut sql = String::from(
            "SELECT id,source_type,source_id,camera_id,image_path,width,height,COALESCE(status,'pending'),boxes_json,created_at,updated_at \
             FROM dataset_samples WHERE 1=1");
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
            Ok(DatasetSample {
                id: r.get(0)?,
                source_type: r.get(1)?,
                source_id: r.get(2)?,
                camera_id: r.get(3)?,
                image_path: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                status: r.get(7)?,
                boxes_json: r.get(8)?,
                created_at: r.get(9)?,
                updated_at: r.get(10)?,
            })
        })?;
        rows.collect()
    }

    pub fn dataset_sample_by_id(&self, id: i64) -> rusqlite::Result<Option<DatasetSample>> {
        rusqlite::OptionalExtension::optional(self.conn.query_row(
            "SELECT id,source_type,source_id,camera_id,image_path,width,height,status,boxes_json,created_at,updated_at \
             FROM dataset_samples WHERE id=?",
            [id],
            |r| {
                Ok(DatasetSample {
                    id: r.get(0)?,
                    source_type: r.get(1)?,
                    source_id: r.get(2)?,
                    camera_id: r.get(3)?,
                    image_path: r.get(4)?,
                    width: r.get(5)?,
                    height: r.get(6)?,
                    status: r.get(7)?,
                    boxes_json: r.get(8)?,
                    created_at: r.get(9)?,
                    updated_at: r.get(10)?,
                })
            },
        ))
    }

    pub fn update_dataset_sample(
        &self,
        id: i64,
        boxes_json: &str,
        status: &str,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE dataset_samples SET boxes_json=?, status=?, updated_at=? WHERE id=?",
            params![boxes_json, status, chrono_now(), id],
        )
    }

    pub fn delete_dataset_sample(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM dataset_samples WHERE id=?", params![id])
    }

    /// 各类别样本数（均衡度展示）：从 boxes_json 统计 class 名。
    /// 返回 [(class_name, count)]，按数量降序。
    pub fn dataset_class_counts(&self) -> rusqlite::Result<Vec<(String, i64)>> {
        let rows = self.dataset_samples(None, 100_000)?;
        let mut m: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        for s in rows {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s.boxes_json) {
                if let Some(arr) = v.as_array() {
                    for b in arr {
                        let cls = b.get("class").and_then(|c| c.as_str()).unwrap_or("unknown");
                        *m.entry(cls.to_string()).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut out: Vec<(String, i64)> = m.into_iter().collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.1));
        Ok(out)
    }

    // ---- LS-054：只读值守分享链接 ----

    pub fn add_share_link(
        &self,
        name: &str,
        token_hash: &str,
        ttl_hours: i64,
        created_ts: &str,
        expires_ts: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO share_links(name,token_hash,ttl_hours,created_ts,expires_ts,revoked) \
             VALUES(?,?,?,?,?,0)",
            params![name, token_hash, ttl_hours, created_ts, expires_ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn share_links(&self) -> rusqlite::Result<Vec<ShareLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,name,ttl_hours,created_ts,expires_ts,COALESCE(revoked,0) \
             FROM share_links ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ShareLink {
                id: r.get(0)?,
                name: r.get(1)?,
                ttl_hours: r.get(2)?,
                created_ts: r.get(3)?,
                expires_ts: r.get(4)?,
                revoked: r.get::<_, i64>(5)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn revoke_share_link(&self, id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("UPDATE share_links SET revoked=1 WHERE id=?", params![id])
    }

    /// 按 token hash 查有效链接（未吊销且未过期）。
    pub fn share_link_valid(&self, token_hash: &str, now_ts: &str) -> rusqlite::Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM share_links WHERE token_hash=? AND COALESCE(revoked,0)=0 AND expires_ts > ?",
            params![token_hash, now_ts],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// 中间件用：若该 hash 属于分享链接则要求未吊销未过期；不属于则放行（普通设备令牌）。
    pub fn share_link_allows(&self, token_hash: &str, now_ts: &str) -> rusqlite::Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM share_links WHERE token_hash=?",
            params![token_hash],
            |r| r.get(0),
        )?;
        if n == 0 {
            return Ok(true);
        }
        self.share_link_valid(token_hash, now_ts)
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
        self.stats_daily_scoped(days, camera_id, status, None)
    }

    pub fn stats_daily_scoped(
        &self,
        days: i64,
        camera_id: Option<i64>,
        status: Option<&str>,
        allowed_cameras: Option<&[i64]>,
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
        append_camera_scope(&mut where_sql, &mut args, allowed_cameras);
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
             ppe_no,ppe_wearing,COALESCE(key,0),COALESCE(violation,0),event_id,snapshot,error,zones,violations \
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

    /// app_settings 写（upsert；R2-309 批次 3：ppe_download 写 ppe_model 路径）
    pub fn setting_set(&self, key: &str, value: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "INSERT INTO app_settings(key,value,updated_ts) VALUES(?,?,?) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_ts=excluded.updated_ts",
            params![key, value, chrono_now()],
        )
    }

    pub fn settings_set(&self, values: &[(&str, &str)]) -> rusqlite::Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        for (key, value) in values {
            self.setting_set(key, value)?;
        }
        transaction.commit()
    }

    /// app_settings 前缀查询（R2-305：ls-server 读 alert.* 渠道覆盖）
    pub fn settings_by_prefix(&self, prefix: &str) -> rusqlite::Result<Vec<(String, String)>> {
        let pattern = format!("{prefix}%");
        let mut stmt = self
            .conn
            .prepare("SELECT key,value FROM app_settings WHERE key LIKE ? ORDER BY key")?;
        let rows = stmt.query_map(params![pattern], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    /// app_settings 按键删除（R2-305：alerts/config/reset 真实语义——Python
    /// delete_app_settings 同行为；置空串会残留"空 DB 覆盖"，这里彻底移除）
    pub fn settings_delete(&self, keys: &[String]) -> rusqlite::Result<usize> {
        let mut n = 0;
        for k in keys {
            n += self
                .conn
                .execute("DELETE FROM app_settings WHERE key=?", params![k])?;
        }
        Ok(n)
    }

    /// R2-309 批次 3：config/import replace 模式（Python store.clear_config 同语义：
    /// 清空摄像头/区域/规则；事件保留但 camera_id 失去对应摄像头）
    pub fn clear_config(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM rules", [])?;
        self.conn.execute("DELETE FROM zones", [])?;
        self.conn.execute("DELETE FROM cameras", [])?;
        Ok(())
    }

    /// 带分组的新增摄像头（Python add_camera(name, source, type, group_name) 同列）
    pub fn add_camera_full(
        &self,
        name: &str,
        source: &str,
        type_: &str,
        group_name: &str,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO cameras(name,source,type,group_name,analyzer) VALUES(?,?,?,?,'default')",
            params![name, source, type_, group_name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// folder_images 单行（Python store.folder_image 同口径）
    pub fn folder_image(&self, iid: i64) -> rusqlite::Result<Option<FolderImageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,camera_id,path,name,ts,processed_ts,width,height,persons,in_zone, \
             ppe_no,ppe_wearing,COALESCE(key,0),COALESCE(violation,0),event_id,snapshot,error,zones,violations \
             FROM folder_images WHERE id=?",
        )?;
        let mut rows = stmt.query_map(params![iid], map_folder_image)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// RA2-S3：folder 识别结果回填（扫描登记行 → 真值）。唯一键（camera_id,path）
    /// 幂等登记的行会被 UPDATE 覆盖成实测值——重复扫描同一文件不重复建事件
    /// （调用方按 `processed_ts`/event_id 判重）。
    #[allow(clippy::too_many_arguments)]
    pub fn update_folder_image_analysis(
        &self,
        iid: i64,
        width: i64,
        height: i64,
        persons: i64,
        in_zone: i64,
        ppe_no: i64,
        ppe_wearing: i64,
        zones: &str,
        violations: &str,
        key: bool,
        violation: bool,
        snapshot: Option<&str>,
        error: Option<&str>,
        processed_ts: &str,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let fresh = tx.execute(
            "UPDATE folder_images SET width=?,height=?,persons=?,in_zone=?,ppe_no=?, \
             ppe_wearing=?,zones=?,violations=?,key=?,violation=?,snapshot=?,error=?, \
             processed_ts=? WHERE id=? AND width IS NULL AND COALESCE(error,'')=''",
            params![
                width,
                height,
                persons,
                in_zone,
                ppe_no,
                ppe_wearing,
                zones,
                violations,
                key as i64,
                violation as i64,
                snapshot,
                error,
                processed_ts,
                iid
            ],
        )?;
        if fresh > 0 {
            let (camera, name): (i64, String) = tx.query_row(
                "SELECT camera_id,COALESCE(name,'图片') FROM folder_images WHERE id=?",
                [iid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let detail = if let Some(error) = error {
                format!("图片「{name}」检查失败。{error}")
            } else {
                let unknown = (persons - ppe_no - ppe_wearing).max(0);
                format!("图片「{name}」检查完成。检出 {persons} 人，实验服证据：已穿 {ppe_wearing}、未穿 {ppe_no}、未知 {unknown}。{}",
                    if violation { "有违规分类，详见图片识别结果。" } else { "未检出明确违规。" })
            };
            let event_id =
                self.add_check_event(camera, "photo_check", &detail, processed_ts, None)?;
            tx.execute(
                "INSERT INTO photo_check_images(event_id,image_id) VALUES(?,?)",
                params![event_id, iid],
            )?;
        }
        tx.commit()
    }

    /// RA2-S3：folder 图片关联首个事件（Python set_folder_image_event 同口径）。
    pub fn set_folder_image_event(&self, iid: i64, eid: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE folder_images SET event_id=? WHERE id=? AND event_id IS NULL",
            params![eid, iid],
        )?;
        Ok(())
    }

    /// RA1.3：folder worker 扫描登记（对齐 Python store.add_folder_image：
    /// INSERT OR IGNORE，唯一键（camera_id,path）冲突返回既有 id——幂等）。
    /// zones/violations 为 JSON 文本；识别字段由调用方给（Rust 独立模式暂无
    /// 推理 → persons 等为 0 的如实占位，RA2 接识别管线后回填真实值）。
    #[allow(clippy::too_many_arguments)]
    pub fn add_folder_image(
        &self,
        camera_id: i64,
        path: &str,
        name: Option<&str>,
        mtime_ns: i64,
        size: i64,
        ts: &str,
        processed_ts: &str,
        width: Option<i64>,
        height: Option<i64>,
        persons: i64,
        in_zone: i64,
        ppe_no: i64,
        ppe_wearing: i64,
        zones: &str,
        violations: &str,
        key: bool,
        violation: bool,
        snapshot: Option<&str>,
        error: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO folder_images(\
             camera_id,path,name,mtime_ns,size,ts,processed_ts,width,height,\
             persons,in_zone,ppe_no,ppe_wearing,zones,violations,key,violation,snapshot,error) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                camera_id,
                path,
                name,
                mtime_ns,
                size,
                ts,
                processed_ts,
                width,
                height,
                persons,
                in_zone,
                ppe_no,
                ppe_wearing,
                zones,
                violations,
                key as i64,
                violation as i64,
                snapshot,
                error
            ],
        )?;
        if self.conn.changes() > 0 {
            Ok(self.conn.last_insert_rowid())
        } else {
            // 唯一键 = (camera_id, path, mtime_ns, size)——对齐 Python schema 的
            // idx_folder_images_uniq 四元组：文件被覆盖（mtime/size 变）即视为新图
            self.conn.query_row(
                "SELECT id FROM folder_images WHERE camera_id=? AND path=? AND mtime_ns=? AND size=?",
                params![camera_id, path, mtime_ns, size],
                |r| r.get(0),
            )
        }
    }

    /// zones 单查（rule-template apply 校验区域归属用）
    pub fn zone(&self, zid: i64) -> rusqlite::Result<Option<(i64, i64, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,camera_id,name,polygon FROM zones WHERE id=?")?;
        let mut rows = stmt.query_map(params![zid], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// 规则模板单查（apply 用）
    pub fn get_rule_template(&self, tid: i64) -> rusqlite::Result<Option<Template>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,name,config_json,created_ts,updated_ts FROM rule_templates WHERE id=?",
        )?;
        let mut rows = stmt.query_map(params![tid], |r| {
            Ok(Template {
                id: r.get(0)?,
                name: r.get(1)?,
                config_json: r.get(2)?,
                created_ts: r.get(3)?,
                updated_ts: r.get(4)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
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
        zones: r.get(17)?,
        violations: r.get(18)?,
    })
}

/// 全量规则行：(id, camera_id, zone_id, behavior, level, cooldown_s,
/// time_start, time_end, threshold, enabled, days_of_week, holiday_only)
/// （days_of_week=CSV 文本 "1,2,3,4,5,6,7"，与 Python store.rules 同口径；RA1.1 补；
///   holiday_only=节假日限定标记，pipeline.holiday_allowed 消费；RA4.2 补）
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
    Option<String>,
    bool,
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
    pub zones: Option<String>,
    pub violations: Option<String>,
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

/// 本地 ISO 日期（days 天前）
fn iso_days_ago(days: i64) -> String {
    ls_core::time::local_days_ago_date(days)
}

/// N 周前的周一（ISO 周，本地时区，与 Python `date - timedelta(days=weekday()) - timedelta(weeks=n)` 同口径）
fn iso_weeks_ago_monday(weeks: i64) -> String {
    let dt = ls_core::time::local_offset_secs(-weeks.saturating_mul(7 * 86_400));
    let wd0 = (ls_core::time::weekday_iso(dt) as i64) - 1; // 0=周一..6=周日
    let mon = ls_core::time::add_millis(dt, -wd0.saturating_mul(86_400_000));
    ls_core::time::format_date(mon)
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
        vlm_verdict: r.get(10)?,
        vlm_ts: r.get(11)?,
        vlm_suppressed: r.get(12)?,
        handled_by: r.get(13)?,
        handled_ts: r.get(14)?,
        ai_review: r.get(15)?,
        ai_review_ts: r.get(16)?,
        ai_review_model: r.get(17)?,
    })
}

/// 本地 ISO 秒时间戳（统一走 `ls_core::time` 本地时区，消除 UTC 8 小时偏移）
fn chrono_now() -> String {
    ls_core::time::local_now_iso()
}

/// Howard Hinnant 的 civil_from_days（公历，days since 1970-01-01）
#[cfg(test)]
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
            eprintln!(
                "[selftest-db] schema v{ver} < 12（库过旧：用新版服务启动一次完成自举/升级）"
            );
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
        // UX2：v12 存量库先迁移到 v14（events cascade 列；幂等复跑安全），
        // 之后的读写 SELECT 都含新列
        assert_eq!(s.migrate().unwrap(), SCHEMA_VERSION_LATEST);
        assert_eq!(s.migrate().unwrap(), SCHEMA_VERSION_LATEST); // 二次调用不炸
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION_LATEST);
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
        s.set_ai_review(
            eid,
            r#"{"verdict":"uncertain"}"#,
            "2026-01-01T00:00:02",
            "legacy-fixture",
        )
        .unwrap();
        assert_eq!(
            s.event_by_id(eid)
                .unwrap()
                .unwrap()
                .ai_review_model
                .as_deref(),
            Some("legacy-fixture")
        );
        s.set_event_status(eid, "acked", Some("rust"), Some("2026-01-01T00:00:02"))
            .unwrap();
        assert_eq!(s.event_by_id(eid).unwrap().unwrap().status, "acked");
        assert_eq!(s.count_events().unwrap(), 1);
        let d = s.add_delivery(eid, "wecom").unwrap();
        s.update_delivery(d, "ok", 1, None).unwrap();
        assert_eq!(s.integrity_check().unwrap(), "ok");
        s.set_event_vlm(eid, r#"{"persons":1}"#, "2026-01-01T00:00:03")
            .unwrap();
        s.set_event_vlm_suppressed(eid).unwrap();
        let cols: Vec<String> = s
            .conn()
            .prepare("PRAGMA table_info(events)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(cols.contains(&"vlm_verdict".to_string()));
        assert!(cols.contains(&"vlm_ts".to_string()));
        assert!(cols.contains(&"vlm_suppressed".to_string()));
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION_LATEST);
    }

    #[test]
    fn migration_records_completed_photo_results_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v20.db");
        let store = Store::open(path.to_str().unwrap()).unwrap();
        store.ensure_schema().unwrap();
        store
            .conn()
            .execute("DELETE FROM schema_version WHERE version >= 21", [])
            .unwrap();
        let cid = store
            .conn_add_camera("Photos", "fixtures", "folder")
            .unwrap();
        store
            .conn()
            .execute(
            "INSERT INTO folder_images(camera_id,path,name,ts,processed_ts,width,persons)
             VALUES(?1,'checked.jpg','checked.jpg','2026-11-23T08:00:00','2026-11-23T08:56:17',1280,1),
                   (?1,'pending.jpg','pending.jpg','2026-11-23T08:00:00','2026-11-23T08:56:18',NULL,0)",
                [cid],
            )
            .unwrap();
        store.migrate().unwrap();
        store.migrate().unwrap();
        let rows = store.events(10, 0, Some(cid), Some("recorded")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].type_, "photo_check");
        assert_eq!(rows[0].ts, "2026-11-23T08:56:17");
        assert!(rows[0].detail.contains("checked.jpg"));
        assert_eq!(
            store.check_image_path(rows[0].id).unwrap().as_deref(),
            Some("checked.jpg")
        );
    }

    #[test]
    fn fresh_bootstrap_schema_v14_has_cascade_columns() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fresh.db");
        let s = Store::open(p.to_str().unwrap()).unwrap();
        assert!(s.ensure_schema().unwrap());
        let cols: Vec<String> = s
            .conn()
            .prepare("PRAGMA table_info(events)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(cols.iter().any(|c| c == "vlm_verdict"));
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION_LATEST);
        // 验证 WAL 模式已开启
        let jm: String = s
            .conn()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(jm.to_ascii_lowercase(), "wal");
        // 验证 add_rule 默认写入全周有效 days_of_week
        let _rid = s
            .add_rule(1, None, "enter", 2, 10, None, None, 0, true)
            .unwrap();
        let rules = s.rules_all().unwrap();
        assert_eq!(rules[0].10.as_deref(), Some("1,2,3,4,5,6,7"));
    }

    #[test]
    fn migrate_v15_repairs_broken_days_of_week_zero() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mig15.db");
        let s = Store::open(p.to_str().unwrap()).unwrap();
        s.ensure_schema().unwrap();
        s.conn()
            .execute(
                "INSERT INTO rules(camera_id,zone_id,behavior,level,cooldown_s,threshold,enabled,days_of_week,holiday_only) VALUES(1,NULL,'enter',2,10,0,1,'0',0)",
                [],
            )
            .unwrap();
        assert_eq!(s.migrate().unwrap(), SCHEMA_VERSION_LATEST);
        let rules = s.rules_all().unwrap();
        assert_eq!(rules[0].10.as_deref(), Some("1,2,3,4,5,6,7"));
    }

    #[test]
    fn dataset_sample_crud_and_class_counts() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path().join("t.db").to_str().unwrap()).unwrap();
        s.ensure_schema().unwrap();
        s.migrate().unwrap();
        let boxes = r#"[{"class":"person","x1":1,"y1":2,"x2":30,"y2":40},
                        {"class":"no_gloves","x1":5,"y1":6,"x2":20,"y2":25}]"#;
        let id = s
            .add_dataset_sample(
                "event",
                "7",
                Some(1),
                "dataset_pool/event_7.jpg",
                1280,
                720,
                boxes,
            )
            .unwrap();
        let rows = s.dataset_samples(None, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "pending");
        // 类别统计在标注框仍存在时计算
        let counts = s.dataset_class_counts().unwrap();
        assert_eq!(counts.len(), 2);
        assert_eq!(s.update_dataset_sample(id, "[]", "verified").unwrap(), 1);
        assert_eq!(s.delete_dataset_sample(id).unwrap(), 1);
        assert!(s.dataset_samples(None, 10).unwrap().is_empty());
    }

    #[test]
    fn share_link_ttl_and_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path().join("t.db").to_str().unwrap()).unwrap();
        s.ensure_schema().unwrap();
        s.migrate().unwrap();
        let hash = "abc123";
        let id = s
            .add_share_link(
                "值班",
                hash,
                24,
                "2026-11-22T10:00:00",
                "2026-11-23T10:00:00",
            )
            .unwrap();
        // 未过期 → 有效
        assert!(s.share_link_valid(hash, "2026-11-22T12:00:00").unwrap());
        // 已过期 → 无效
        assert!(!s.share_link_valid(hash, "2026-11-24T00:00:00").unwrap());
        // 吊销后 → 无效
        assert_eq!(s.revoke_share_link(id).unwrap(), 1);
        assert!(!s.share_link_valid(hash, "2026-11-22T12:00:00").unwrap());
        // 非分享 hash 放行（普通设备令牌）
        assert!(s.share_link_allows("other", "2026-11-22T12:00:00").unwrap());
    }
}
