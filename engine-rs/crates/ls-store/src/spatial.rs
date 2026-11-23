use crate::{NewEvent, Store};
use ls_core::Point;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS camera_masks(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER NOT NULL, name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('motion','object_filter')), polygon TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_camera_masks_camera ON camera_masks(camera_id);
CREATE TABLE IF NOT EXISTS fp_spots(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER NOT NULL, event_id INTEGER NOT NULL UNIQUE,
  behavior TEXT NOT NULL, bbox TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_fp_spots_camera ON fp_spots(camera_id);
CREATE TABLE IF NOT EXISTS event_locations(event_id INTEGER PRIMARY KEY, bbox TEXT NOT NULL);
CREATE TRIGGER IF NOT EXISTS delete_event_location AFTER DELETE ON events
BEGIN DELETE FROM event_locations WHERE event_id=OLD.id; END;
CREATE TABLE IF NOT EXISTS suppression_audit(
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT NOT NULL, camera_id INTEGER NOT NULL,
  reason TEXT NOT NULL, source_id INTEGER NOT NULL, behavior TEXT NOT NULL,
  frame_bucket INTEGER NOT NULL, hits INTEGER NOT NULL DEFAULT 1,
  UNIQUE(camera_id,reason,source_id,behavior,frame_bucket));
CREATE INDEX IF NOT EXISTS idx_suppression_audit_ts ON suppression_audit(ts);
CREATE INDEX IF NOT EXISTS idx_suppression_audit_camera ON suppression_audit(camera_id,id DESC);
";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskKind {
    Motion,
    ObjectFilter,
}

impl MaskKind {
    fn name(self) -> &'static str {
        match self {
            Self::Motion => "motion",
            Self::ObjectFilter => "object_filter",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CameraMask {
    pub id: i64,
    pub camera_id: i64,
    pub name: String,
    pub kind: MaskKind,
    pub polygon: Vec<Point>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FalsePositiveSpot {
    pub id: i64,
    pub event_id: i64,
    pub behavior: String,
    pub bbox: [f32; 4],
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct SuppressionRecord {
    pub id: i64,
    pub ts: String,
    pub camera_id: i64,
    pub reason: String,
    pub source_id: i64,
    pub behavior: String,
    pub hits: i64,
}

impl Store {
    pub fn camera_masks(&self, camera_id: i64) -> rusqlite::Result<Vec<CameraMask>> {
        let mut stmt = self.conn().prepare(
            "SELECT id,name,kind,polygon FROM camera_masks WHERE camera_id=? ORDER BY id",
        )?;
        let rows = stmt.query_map([camera_id], |row| {
            let kind: String = row.get(2)?;
            Ok(CameraMask {
                id: row.get(0)?,
                camera_id,
                name: row.get(1)?,
                kind: if kind == "motion" {
                    MaskKind::Motion
                } else {
                    MaskKind::ObjectFilter
                },
                polygon: read_json(row, 3)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_camera_mask(
        &self,
        camera_id: i64,
        name: &str,
        kind: MaskKind,
        polygon: &[Point],
    ) -> rusqlite::Result<i64> {
        self.conn().execute(
            "INSERT INTO camera_masks(camera_id,name,kind,polygon) VALUES(?,?,?,?)",
            params![
                camera_id,
                name,
                kind.name(),
                serde_json::json!(polygon).to_string()
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn delete_camera_mask(&self, camera_id: i64, id: i64) -> rusqlite::Result<usize> {
        self.conn().execute(
            "DELETE FROM camera_masks WHERE camera_id=? AND id=?",
            params![camera_id, id],
        )
    }

    pub fn add_located_event(&self, event: &NewEvent<'_>, bbox: [f32; 4]) -> rusqlite::Result<i64> {
        let tx = self.conn().unchecked_transaction()?;
        let id = self.add_event(event)?;
        tx.execute(
            "INSERT INTO event_locations(event_id,bbox) VALUES(?,?)",
            params![id, serde_json::json!(bbox).to_string()],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn remember_false_positive(&self, event_id: i64) -> rusqlite::Result<Option<(i64, i64)>> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO fp_spots(camera_id,event_id,behavior,bbox,created_at)
             SELECT e.camera_id,e.id,e.type,l.bbox,datetime('now','localtime')
             FROM events e JOIN event_locations l ON l.event_id=e.id JOIN cameras c ON c.id=e.camera_id
             WHERE e.id=? AND e.status='false_positive' AND e.type IN ('enter','enter_no_ppe','no_mask')",
            [event_id],
        )?;
        let spot = tx
            .query_row(
                "SELECT id,camera_id FROM fp_spots WHERE event_id=?",
                [event_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        tx.commit()?;
        Ok(spot)
    }

    pub fn fp_spots(&self, camera_id: i64) -> rusqlite::Result<Vec<FalsePositiveSpot>> {
        let mut stmt = self.conn().prepare(
            "SELECT id,event_id,behavior,bbox,created_at FROM fp_spots WHERE camera_id=? ORDER BY id",
        )?;
        let rows = stmt.query_map([camera_id], |row| {
            Ok(FalsePositiveSpot {
                id: row.get(0)?,
                event_id: row.get(1)?,
                behavior: row.get(2)?,
                bbox: read_json(row, 3)?,
                created_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete_fp_spot(&self, camera_id: i64, id: i64) -> rusqlite::Result<usize> {
        self.conn().execute(
            "DELETE FROM fp_spots WHERE camera_id=? AND id=?",
            params![camera_id, id],
        )
    }

    pub fn add_suppression(
        &self,
        camera_id: i64,
        reason: &str,
        source_id: i64,
        behavior: &str,
        ts: &str,
    ) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT INTO suppression_audit(ts,camera_id,reason,source_id,behavior,frame_bucket)
             VALUES(?,?,?,?,?,unixepoch()/30)
             ON CONFLICT(camera_id,reason,source_id,behavior,frame_bucket)
             DO UPDATE SET ts=excluded.ts,hits=hits+1",
            params![ts, camera_id, reason, source_id, behavior],
        )?;
        Ok(())
    }

    pub fn suppressions(
        &self,
        camera_id: Option<i64>,
        before_id: Option<i64>,
        limit: i64,
    ) -> rusqlite::Result<Vec<SuppressionRecord>> {
        let mut stmt = self.conn().prepare(
            "SELECT id,ts,camera_id,reason,source_id,behavior,hits FROM suppression_audit
             WHERE (?1 IS NULL OR camera_id=?1) AND id < ?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![
                camera_id,
                before_id.unwrap_or(i64::MAX),
                limit.clamp(1, 200)
            ],
            |row| {
                Ok(SuppressionRecord {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    camera_id: row.get(2)?,
                    reason: row.get(3)?,
                    source_id: row.get(4)?,
                    behavior: row.get(5)?,
                    hits: row.get(6)?,
                })
            },
        )?;
        rows.collect()
    }

    pub fn delete_suppressions_older_than(&self, cutoff: &str) -> rusqlite::Result<usize> {
        self.conn()
            .execute("DELETE FROM suppression_audit WHERE ts < ?", [cutoff])
    }
}

fn read_json<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}
