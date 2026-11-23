use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;

use crate::{chrono_now, Delivery, Store};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS photo_check_images (
    event_id INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
    image_id INTEGER NOT NULL
);
CREATE TRIGGER IF NOT EXISTS prune_photo_check_images AFTER DELETE ON events
BEGIN
    DELETE FROM photo_check_images WHERE event_id=OLD.id;
END;
CREATE INDEX IF NOT EXISTS idx_alert_deliveries_event ON alert_deliveries(event_id, id);
CREATE TABLE IF NOT EXISTS alert_delivery_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    delivery_id INTEGER NOT NULL,
    ts TEXT NOT NULL,
    status TEXT NOT NULL,
    error TEXT
);
CREATE INDEX IF NOT EXISTS idx_alert_delivery_history_id ON alert_delivery_history(delivery_id, id);
CREATE TRIGGER IF NOT EXISTS log_alert_delivery_attempt AFTER UPDATE ON alert_deliveries
WHEN NEW.attempts > OLD.attempts
     OR (NEW.status = 'failed' AND COALESCE(OLD.status, 'pending') = 'pending')
     OR (NEW.status = 'dead_letter' AND OLD.status IS NOT 'dead_letter')
BEGIN
    INSERT INTO alert_delivery_history(delivery_id, ts, status, error)
    VALUES(NEW.id, NEW.updated_ts, NEW.status, NEW.last_error);
END;
CREATE TRIGGER IF NOT EXISTS prune_alert_delivery_history AFTER DELETE ON alert_deliveries
BEGIN
    DELETE FROM alert_delivery_history WHERE delivery_id = OLD.id;
END;
CREATE TABLE IF NOT EXISTS event_notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id INTEGER NOT NULL,
    ts TEXT NOT NULL,
    actor TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_event_notes_id ON event_notes(event_id, id);
CREATE TRIGGER IF NOT EXISTS prune_event_notes AFTER DELETE ON events
BEGIN
    DELETE FROM event_notes WHERE event_id = OLD.id;
END;
CREATE TABLE IF NOT EXISTS camera_outages (
    camera_id INTEGER PRIMARY KEY,
    since TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS compliance_daily (
    day TEXT NOT NULL,
    camera_id INTEGER NOT NULL,
    frames INTEGER NOT NULL,
    compliant INTEGER NOT NULL,
    violations INTEGER NOT NULL,
    unknown INTEGER NOT NULL,
    PRIMARY KEY(day, camera_id)
);
CREATE TRIGGER IF NOT EXISTS prune_camera_outage AFTER DELETE ON cameras
BEGIN
    DELETE FROM camera_outages WHERE camera_id = OLD.id;
END;
";

#[derive(Debug, Serialize)]
pub struct DeliveryAttempt {
    pub ts: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeliveryWithHistory {
    #[serde(flatten)]
    pub delivery: Delivery,
    pub history: Vec<DeliveryAttempt>,
}

#[derive(Debug, Serialize)]
pub struct EventNote {
    pub id: i64,
    pub ts: String,
    pub actor: String,
    pub text: String,
}

#[derive(Debug, Default, Serialize)]
pub struct ComplianceCounts {
    pub frames: i64,
    pub cameras: i64,
    pub compliant: i64,
    pub violations: i64,
    pub unknown: i64,
}

impl ComplianceCounts {
    pub fn checked(&self) -> i64 {
        self.compliant + self.violations
    }

    pub fn rate_percent(&self) -> Option<f64> {
        let checked = self.checked();
        (checked > 0).then(|| 100.0 * self.compliant as f64 / checked as f64)
    }
}

impl Store {
    pub fn camera_outage_since(&self, camera_id: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT since FROM camera_outages WHERE camera_id=?",
                [camera_id],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn camera_outages(&self) -> rusqlite::Result<std::collections::HashMap<i64, String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT camera_id,since FROM camera_outages")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    pub fn record_camera_connection_event(
        &self,
        online: bool,
        event: &crate::NewEvent<'_>,
    ) -> rusqlite::Result<Option<i64>> {
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let changed = if online {
            transaction.execute(
                "DELETE FROM camera_outages WHERE camera_id=?",
                [event.camera_id],
            )?
        } else {
            transaction.execute(
                "INSERT OR IGNORE INTO camera_outages(camera_id,since) SELECT id,? FROM cameras WHERE id=?",
                params![event.ts, event.camera_id],
            )?
        };
        let event_id = if changed == 0 {
            None
        } else {
            Some(self.add_event(event)?)
        };
        transaction.commit()?;
        Ok(event_id)
    }

    pub fn add_compliance_sample(
        &self,
        camera: i64,
        day: &str,
        compliant: i64,
        violations: i64,
        unknown: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO compliance_daily(day,camera_id,frames,compliant,violations,unknown) \
             SELECT ?,id,1,?,?,? FROM cameras WHERE id=? \
             ON CONFLICT(day,camera_id) DO UPDATE SET frames=frames+1, \
             compliant=compliant+excluded.compliant, violations=violations+excluded.violations, \
             unknown=unknown+excluded.unknown",
            params![day, compliant, violations, unknown, camera],
        )?;
        Ok(())
    }

    pub fn compliance_counts(
        &self,
        start: &str,
        end: &str,
        group: Option<&str>,
        allowed: Option<&[i64]>,
    ) -> rusqlite::Result<ComplianceCounts> {
        let mut sql = String::from(
            "SELECT COALESCE(SUM(frames),0),COUNT(DISTINCT camera_id),\
            COALESCE(SUM(compliant),0),COALESCE(SUM(violations),0),COALESCE(SUM(unknown),0) \
            FROM compliance_daily JOIN cameras ON cameras.id=camera_id WHERE day BETWEEN ? AND ?",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(start.to_owned()), Box::new(end.to_owned())];
        if let Some(group) = group {
            sql.push_str(" AND COALESCE(cameras.group_name,'')=?");
            args.push(Box::new(group.to_owned()));
        }
        crate::append_camera_scope(&mut sql, &mut args, allowed);
        self.conn
            .query_row(&sql, rusqlite::params_from_iter(args), |row| {
                Ok(ComplianceCounts {
                    frames: row.get(0)?,
                    cameras: row.get(1)?,
                    compliant: row.get(2)?,
                    violations: row.get(3)?,
                    unknown: row.get(4)?,
                })
            })
    }

    pub fn event_sample_id(&self, event_id: i64) -> rusqlite::Result<Option<i64>> {
        self.conn.query_row(
            "SELECT id FROM dataset_samples WHERE source_type='event' AND source_id=? ORDER BY id LIMIT 1",
            [event_id.to_string()],
            |row| row.get(0),
        ).optional()
    }

    pub fn add_event_sample_once(
        &self,
        event: &crate::Event,
        path: &str,
        size: (u32, u32),
        boxes: &str,
    ) -> rusqlite::Result<(i64, bool)> {
        let transaction = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        if let Some(id) = self.event_sample_id(event.id)? {
            return Ok((id, false));
        }
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE id=?)",
            [event.id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        let id = self.add_dataset_sample(
            "event",
            &event.id.to_string(),
            Some(event.camera_id),
            path,
            i64::from(size.0),
            i64::from(size.1),
            boxes,
        )?;
        transaction.commit()?;
        Ok((id, true))
    }

    pub fn event_deliveries(
        &self,
        event_id: i64,
        status: Option<&str>,
        limit: i64,
    ) -> rusqlite::Result<Vec<DeliveryWithHistory>> {
        let deliveries = self.list_deliveries(status, Some(event_id), limit)?;
        let mut query = self.conn.prepare(
            "SELECT ts,status,error FROM alert_delivery_history \
             WHERE delivery_id=? ORDER BY id DESC LIMIT 20",
        )?;
        deliveries
            .into_iter()
            .map(|delivery| {
                let history = query
                    .query_map([delivery.id], |row| {
                        Ok(DeliveryAttempt {
                            ts: row.get(0)?,
                            status: row.get(1)?,
                            error: row.get(2)?,
                        })
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                Ok(DeliveryWithHistory { delivery, history })
            })
            .collect()
    }

    pub fn event_notes(&self, event_id: i64) -> rusqlite::Result<Vec<EventNote>> {
        let mut query = self.conn.prepare(
            "SELECT id,ts,actor,text FROM event_notes WHERE event_id=? ORDER BY id DESC LIMIT 100",
        )?;
        let rows = query.query_map([event_id], |row| {
            Ok(EventNote {
                id: row.get(0)?,
                ts: row.get(1)?,
                actor: row.get(2)?,
                text: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_event_note(
        &self,
        event_id: i64,
        actor: &str,
        text: &str,
    ) -> rusqlite::Result<Option<EventNote>> {
        let ts = chrono_now();
        let count = self.conn.execute(
            "INSERT INTO event_notes(event_id,ts,actor,text) SELECT id,?,?,? FROM events WHERE id=?",
            params![ts, actor, text, event_id],
        )?;
        Ok((count == 1).then(|| EventNote {
            id: self.conn.last_insert_rowid(),
            ts,
            actor: actor.to_owned(),
            text: text.to_owned(),
        }))
    }
}
