use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::Serialize;

use super::{EventsFilter, Store, ALERT_EVENTS_SQL, EVENT_COLUMNS};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS ai_chat_sessions (
        id INTEGER PRIMARY KEY, title TEXT NOT NULL, scope_json TEXT NOT NULL,
        revision INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS ai_chat_messages (
        session_id INTEGER NOT NULL REFERENCES ai_chat_sessions(id) ON DELETE CASCADE,
        request_id TEXT NOT NULL, revision INTEGER NOT NULL, question TEXT NOT NULL,
        state TEXT NOT NULL, lease_until INTEGER NOT NULL, answer TEXT NOT NULL DEFAULT '',
        evidence_mode TEXT NOT NULL DEFAULT 'record_only', facts_json TEXT NOT NULL DEFAULT '{}',
        model TEXT, created_at TEXT NOT NULL, PRIMARY KEY(session_id, request_id)
    );
    CREATE UNIQUE INDEX IF NOT EXISTS ai_chat_one_pending ON ai_chat_messages(session_id) WHERE state='pending';
    CREATE INDEX IF NOT EXISTS ai_chat_history ON ai_chat_messages(session_id, revision) WHERE state='done';
    CREATE TABLE IF NOT EXISTS ai_reports (
        id INTEGER PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, period TEXT NOT NULL,
        scope_json TEXT NOT NULL, include_images INTEGER NOT NULL, state TEXT NOT NULL,
        lease_until INTEGER NOT NULL, created_at TEXT NOT NULL, archived_at TEXT,
        answer TEXT NOT NULL DEFAULT '', evidence_mode TEXT NOT NULL DEFAULT 'record_only',
        facts_json TEXT NOT NULL DEFAULT '{}', model TEXT
    );";

const SESSION_FIELDS: &str = "id,title,scope_json,revision,created_at,updated_at";
const MESSAGE_FIELDS: &str =
    "request_id,revision,question,state,answer,evidence_mode,facts_json,model,created_at";
const REPORT_FIELDS: &str = "id,request_id,period,scope_json,include_images,state,created_at,archived_at,answer,evidence_mode,facts_json,model";

#[derive(Serialize)]
pub struct ChatSession {
    pub id: i64,
    pub title: String,
    pub scope: serde_json::Value,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct ChatMessage {
    pub request_id: String,
    pub revision: i64,
    pub question: String,
    pub state: String,
    pub answer: String,
    pub evidence_mode: String,
    pub facts: serde_json::Value,
    pub model: Option<String>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct AiReport {
    pub id: i64,
    pub request_id: String,
    pub period: String,
    pub scope: serde_json::Value,
    pub include_images: bool,
    pub state: String,
    pub created_at: String,
    pub archived_at: Option<String>,
    pub answer: String,
    pub evidence_mode: String,
    pub facts: serde_json::Value,
    pub model: Option<String>,
}

pub enum Reservation<T> {
    Created(T),
    Existing(T),
    Busy,
    Conflict,
    Missing,
}

pub struct AiCompletion<'a> {
    pub answer: &'a str,
    pub evidence_mode: &'a str,
    pub facts_json: &'a str,
    pub model: Option<&'a str>,
}

fn stored_json(row: &Row<'_>, index: usize) -> rusqlite::Result<serde_json::Value> {
    let source = row.get::<_, String>(index)?;
    serde_json::from_str(&source).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn session(row: &Row<'_>) -> rusqlite::Result<ChatSession> {
    Ok(ChatSession {
        id: row.get(0)?,
        title: row.get(1)?,
        scope: stored_json(row, 2)?,
        revision: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn message(row: &Row<'_>) -> rusqlite::Result<ChatMessage> {
    Ok(ChatMessage {
        request_id: row.get(0)?,
        revision: row.get(1)?,
        question: row.get(2)?,
        state: row.get(3)?,
        answer: row.get(4)?,
        evidence_mode: row.get(5)?,
        facts: stored_json(row, 6)?,
        model: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn report(row: &Row<'_>) -> rusqlite::Result<AiReport> {
    Ok(AiReport {
        id: row.get(0)?,
        request_id: row.get(1)?,
        period: row.get(2)?,
        scope: stored_json(row, 3)?,
        include_images: row.get(4)?,
        state: row.get(5)?,
        created_at: row.get(6)?,
        archived_at: row.get(7)?,
        answer: row.get(8)?,
        evidence_mode: row.get(9)?,
        facts: stored_json(row, 10)?,
        model: row.get(11)?,
    })
}

impl Store {
    pub fn create_ai_session(
        &self,
        title: &str,
        scope: &str,
        now: &str,
    ) -> rusqlite::Result<ChatSession> {
        self.conn.execute(
            "INSERT INTO ai_chat_sessions(title,scope_json,created_at,updated_at) VALUES(?,?,?,?)",
            params![title, scope, now, now],
        )?;
        self.ai_session(self.conn.last_insert_rowid())?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    pub fn ai_sessions(&self, offset: i64) -> rusqlite::Result<Vec<ChatSession>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {SESSION_FIELDS} FROM ai_chat_sessions ORDER BY updated_at DESC,id DESC LIMIT 50 OFFSET ?"))?;
        let rows = stmt.query_map([offset], session)?;
        rows.collect()
    }

    pub fn ai_session(&self, id: i64) -> rusqlite::Result<Option<ChatSession>> {
        self.conn
            .query_row(
                &format!("SELECT {SESSION_FIELDS} FROM ai_chat_sessions WHERE id=?"),
                [id],
                session,
            )
            .optional()
    }

    pub fn ai_messages(
        &self,
        id: i64,
        limit: i64,
        offset: i64,
    ) -> rusqlite::Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {MESSAGE_FIELDS} FROM ai_chat_messages WHERE session_id=? AND state='done' ORDER BY revision DESC LIMIT ? OFFSET ?"))?;
        let rows = stmt.query_map(params![id, limit, offset], message)?;
        rows.collect()
    }

    pub fn reserve_ai_message(
        &self,
        id: i64,
        request_id: &str,
        revision: i64,
        question: &str,
        now: i64,
    ) -> rusqlite::Result<Reservation<ChatMessage>> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let Some(current) = tx
            .query_row(
                "SELECT revision FROM ai_chat_sessions WHERE id=?",
                [id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(Reservation::Missing);
        };
        tx.execute("UPDATE ai_chat_messages SET state='interrupted' WHERE session_id=? AND state='pending' AND lease_until<?", params![id, now])?;
        let existing = tx.query_row(&format!("SELECT {MESSAGE_FIELDS} FROM ai_chat_messages WHERE session_id=? AND request_id=?"), params![id, request_id], message).optional()?;
        let reservation = if let Some(existing) = existing {
            if existing.question == question && existing.revision == revision + 1 { Reservation::Existing(existing) } else { Reservation::Conflict }
        } else if current != revision {
            Reservation::Conflict
        } else if tx.execute(
            "INSERT OR IGNORE INTO ai_chat_messages(session_id,request_id,revision,question,state,lease_until,created_at) VALUES(?,?,?,?,'pending',?,?)",
            params![id, request_id, revision + 1, question, now + 240, super::chrono_now()],
        )? == 0 {
            Reservation::Busy
        } else {
            Reservation::Created(tx.query_row(&format!("SELECT {MESSAGE_FIELDS} FROM ai_chat_messages WHERE session_id=? AND request_id=?"), params![id, request_id], message)?)
        };
        tx.commit()?;
        Ok(reservation)
    }

    pub fn complete_ai_message(
        &self,
        id: i64,
        pending: &ChatMessage,
        completion: AiCompletion<'_>,
    ) -> rusqlite::Result<Option<ChatMessage>> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE ai_chat_sessions SET revision=?,updated_at=? WHERE id=? AND revision=?",
            params![
                pending.revision,
                super::chrono_now(),
                id,
                pending.revision - 1
            ],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        let changed = tx.execute("UPDATE ai_chat_messages SET state='done',answer=?,evidence_mode=?,facts_json=?,model=? WHERE session_id=? AND request_id=? AND state='pending'",
            params![completion.answer, completion.evidence_mode, completion.facts_json, completion.model, id, pending.request_id])?;
        if changed != 1 {
            return Ok(None);
        }
        let done = tx.query_row(
            &format!(
                "SELECT {MESSAGE_FIELDS} FROM ai_chat_messages WHERE session_id=? AND request_id=?"
            ),
            params![id, pending.request_id],
            message,
        )?;
        tx.commit()?;
        Ok(Some(done))
    }

    pub fn reserve_ai_report(
        &self,
        request_id: &str,
        period: &str,
        scope: &str,
        include_images: bool,
        now: i64,
    ) -> rusqlite::Result<Reservation<AiReport>> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE ai_reports SET state='interrupted' WHERE state='pending' AND lease_until<?",
            [now],
        )?;
        let changed = tx.execute(
            "INSERT OR IGNORE INTO ai_reports(request_id,period,scope_json,include_images,state,lease_until,created_at) VALUES(?,?,?,?,'pending',?,?)",
            params![request_id, period, scope, include_images, now + 240, super::chrono_now()],
        )?;
        let row = tx.query_row(
            &format!("SELECT {REPORT_FIELDS} FROM ai_reports WHERE request_id=?"),
            [request_id],
            report,
        )?;
        let outcome = match changed {
            1 => Reservation::Created(row),
            _ if row.period == period
                && serde_json::from_str::<serde_json::Value>(scope)
                    .is_ok_and(|value| value == row.scope)
                && row.include_images == include_images =>
            {
                Reservation::Existing(row)
            }
            _ => Reservation::Conflict,
        };
        tx.commit()?;
        Ok(outcome)
    }

    pub fn complete_ai_report(
        &self,
        id: i64,
        completion: AiCompletion<'_>,
    ) -> rusqlite::Result<Option<AiReport>> {
        if self.conn.execute("UPDATE ai_reports SET state='done',answer=?,evidence_mode=?,facts_json=?,model=? WHERE id=? AND state='pending'", params![completion.answer, completion.evidence_mode, completion.facts_json, completion.model, id])? == 0 {
            return Ok(None);
        }
        self.ai_report(id)
    }

    pub fn ai_reports(&self, archived: bool, offset: i64) -> rusqlite::Result<Vec<AiReport>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {REPORT_FIELDS} FROM ai_reports WHERE state='done' AND (archived_at IS NOT NULL)=? ORDER BY id DESC LIMIT 50 OFFSET ?"))?;
        let rows = stmt.query_map(params![archived, offset], report)?;
        rows.collect()
    }

    pub fn ai_report(&self, id: i64) -> rusqlite::Result<Option<AiReport>> {
        self.conn
            .query_row(
                &format!("SELECT {REPORT_FIELDS} FROM ai_reports WHERE id=? AND state='done'"),
                [id],
                report,
            )
            .optional()
    }

    pub fn archive_ai_report(&self, id: i64, archived: bool) -> rusqlite::Result<bool> {
        let stamp = archived.then(super::chrono_now);
        Ok(self.conn.execute(
            "UPDATE ai_reports SET archived_at=? WHERE id=? AND state='done'",
            params![stamp, id],
        )? > 0)
    }

    pub fn ai_event_distribution(
        &self,
        filter: &EventsFilter<'_>,
    ) -> rusqlite::Result<serde_json::Value> {
        let (conditions, args) = filter.conditions();
        let mut distribution = serde_json::Map::new();
        for (field, sql) in [
            ("by_day", "SUBSTR(ts,1,10)"),
            ("by_hour", "SUBSTR(ts,12,2)"),
            ("by_type", "type"),
            ("by_camera", "CAST(camera_id AS TEXT)"),
        ] {
            let mut stmt = self.conn.prepare(&format!("SELECT {sql},COUNT(*) FROM events{conditions} AND {ALERT_EVENTS_SQL} GROUP BY {sql} ORDER BY COUNT(*) DESC,{sql} LIMIT 24"))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(&args), |row| {
                Ok(serde_json::json!({"key":row.get::<_,String>(0)?,"events":row.get::<_,i64>(1)?}))
            })?;
            distribution.insert(
                field.into(),
                serde_json::Value::Array(rows.collect::<rusqlite::Result<Vec<_>>>()?),
            );
        }
        Ok(distribution.into())
    }

    pub fn ai_evidence(&self, filter: &EventsFilter<'_>) -> rusqlite::Result<Vec<super::Event>> {
        let (conditions, args) = filter.conditions();
        // Favor distinct types and cameras before repeating a series from one view.
        let mut stmt = self.conn.prepare(&format!("SELECT {EVENT_COLUMNS} FROM events JOIN (
            SELECT id AS picked_id,ROW_NUMBER() OVER(PARTITION BY camera_id,type ORDER BY level DESC,id DESC) AS sample_rank
              FROM events{conditions} AND {ALERT_EVENTS_SQL} AND snapshot_sha256 IS NOT NULL AND snapshot IS NOT NULL
            ) ON id=picked_id ORDER BY sample_rank,level DESC,id DESC LIMIT 16"))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(&args), super::map_event)?;
        rows.collect()
    }
}
