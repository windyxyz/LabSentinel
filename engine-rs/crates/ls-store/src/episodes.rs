use crate::{EventEvidence, NewEvent, Store};
use rusqlite::{params, OptionalExtension};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS violation_episodes(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  camera_id INTEGER NOT NULL, rule_id INTEGER NOT NULL, zone_id INTEGER,
  track_id INTEGER NOT NULL, behavior TEXT NOT NULL,
  first_ts TEXT NOT NULL, last_ts TEXT NOT NULL,
  duration_s REAL NOT NULL DEFAULT 0, hit_count INTEGER NOT NULL DEFAULT 1,
  peak_conf REAL, alert_tier TEXT NOT NULL DEFAULT 'grace',
  closed_ts TEXT, close_reason TEXT
);
CREATE INDEX IF NOT EXISTS idx_violation_episodes_active
  ON violation_episodes(camera_id, rule_id, track_id) WHERE closed_ts IS NULL;
CREATE INDEX IF NOT EXISTS idx_violation_episodes_history
  ON violation_episodes(camera_id, first_ts);
CREATE INDEX IF NOT EXISTS idx_events_episode
  ON events(episode_id) WHERE episode_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_episode_alarm
  ON events(episode_id) WHERE episode_id IS NOT NULL AND type<>'alert_resolved';
";

pub struct EpisodeEvidence<'a> {
    pub first_ts: &'a str,
    pub last_ts: &'a str,
    pub duration_s: f64,
    pub hits: i64,
    pub peak_conf: f32,
    pub tier: &'a str,
}

pub struct NewEpisode<'a> {
    pub camera_id: i64,
    pub rule_id: i64,
    pub zone_id: Option<i64>,
    pub track_id: i64,
    pub behavior: &'a str,
    pub evidence: EpisodeEvidence<'a>,
}

#[derive(serde::Serialize)]
pub struct EpisodeContext {
    pub id: i64,
    pub rule_id: i64,
    pub zone_id: Option<i64>,
    pub behavior: String,
    pub closed_ts: Option<String>,
    pub close_reason: Option<String>,
}

impl Store {
    pub fn episode_context(&self, id: i64) -> rusqlite::Result<Option<EpisodeContext>> {
        self.conn().query_row(
            "SELECT id,rule_id,zone_id,behavior,closed_ts,close_reason FROM violation_episodes WHERE id=?",
            [id], |row| Ok(EpisodeContext {
                id: row.get(0)?, rule_id: row.get(1)?, zone_id: row.get(2)?, behavior: row.get(3)?,
                closed_ts: row.get(4)?, close_reason: row.get(5)?,
            }),
        ).optional()
    }
    pub fn set_rule_grace(
        &self,
        id: i64,
        seconds: u32,
        warning_ratio: f64,
    ) -> rusqlite::Result<usize> {
        self.conn().execute(
            "UPDATE rules SET grace_s=?,warn_ratio=? WHERE id=? AND behavior IN ('enter_no_ppe','no_mask')",
            params![seconds,warning_ratio,id],
        )
    }

    pub fn camera_rule_grace(&self, camera_id: i64) -> rusqlite::Result<Vec<(i64, u32, f64)>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id,grace_s,warn_ratio FROM rules WHERE camera_id=?")?;
        let rows = stmt.query_map([camera_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect()
    }

    pub fn add_episode(&self, episode: &NewEpisode<'_>) -> rusqlite::Result<i64> {
        let row = &episode.evidence;
        self.conn().execute(
            "INSERT INTO violation_episodes(camera_id,rule_id,zone_id,track_id,behavior,first_ts,
             last_ts,duration_s,hit_count,peak_conf,alert_tier) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            params![
                episode.camera_id,
                episode.rule_id,
                episode.zone_id,
                episode.track_id,
                episode.behavior,
                row.first_ts,
                row.last_ts,
                row.duration_s,
                row.hits,
                row.peak_conf,
                row.tier
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn update_episode(&self, id: i64, row: &EpisodeEvidence<'_>) -> rusqlite::Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute(
            "UPDATE violation_episodes SET last_ts=?2,duration_s=?3,hit_count=?4,peak_conf=?5,
             alert_tier=?6 WHERE id=?1 AND closed_ts IS NULL",
            params![
                id,
                row.last_ts,
                row.duration_s,
                row.hits,
                row.peak_conf,
                row.tier
            ],
        )?;
        tx.execute(
            "UPDATE events SET last_ts=?2,duration_s=?3,hit_count=?4,peak_conf=?5,alert_tier=?6
             WHERE episode_id=?1 AND resolved_ts IS NULL AND type<>'alert_resolved'
               AND EXISTS(SELECT 1 FROM violation_episodes WHERE id=?1 AND closed_ts IS NULL)",
            params![
                id,
                row.last_ts,
                row.duration_s,
                row.hits,
                row.peak_conf,
                row.tier
            ],
        )?;
        tx.commit()
    }

    pub fn close_episode(
        &self,
        id: i64,
        ts: &str,
        reason: &str,
        recovery: Option<&NewEvent<'_>>,
    ) -> rusqlite::Result<Option<i64>> {
        let tx = self.conn().unchecked_transaction()?;
        let updated = tx.execute(
            "UPDATE violation_episodes SET closed_ts=?,close_reason=? WHERE id=? AND closed_ts IS NULL",
            params![ts,reason,id],
        )? != 0;
        let recovered = updated && matches!(reason, "recovered" | "left_zone");
        if recovered {
            tx.execute(
                "UPDATE events SET resolved_ts=? WHERE episode_id=?",
                params![ts, id],
            )?;
        }
        let event_id = if recovered {
            recovery
                .map(|event| {
                    self.add_event_with_evidence(
                        event,
                        EventEvidence {
                            episode_id: Some(id),
                            ..Default::default()
                        },
                    )
                })
                .transpose()?
        } else {
            None
        };
        tx.commit()?;
        Ok(event_id)
    }

    pub fn promote_episode_event(
        &self,
        eid: i64,
        level: i64,
        detail: &str,
    ) -> rusqlite::Result<bool> {
        self.conn().query_row(
            "UPDATE events SET level=MAX(level,?),detail=?,alert_tier='alert' WHERE id=? AND episode_id IS NOT NULL
             RETURNING vlm_verdict IS NOT NULL AND vlm_suppressed=0",
            params![level,detail,eid],
            |row| row.get(0),
        )
    }

    pub fn refresh_episode_snapshot(
        &self,
        eid: i64,
        path: &str,
        digest: &str,
        overlay: Option<&str>,
        bbox: [f32; 4],
    ) -> rusqlite::Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        let updated = tx.execute(
            "UPDATE events SET snapshot=?,snapshot_sha256=?,detections_json=? WHERE id=? AND episode_id IS NOT NULL",
            params![path,digest,overlay,eid],
        )?;
        if updated == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        tx.execute(
            "INSERT INTO event_locations(event_id,bbox) VALUES(?,?)
             ON CONFLICT(event_id) DO UPDATE SET bbox=excluded.bbox",
            params![eid, serde_json::json!(bbox).to_string()],
        )?;
        tx.commit()
    }

    pub fn interrupt_open_episodes(&self, ts: &str) -> rusqlite::Result<usize> {
        self.conn().execute(
            "UPDATE violation_episodes SET closed_ts=?,close_reason='interrupted' WHERE closed_ts IS NULL",
            [ts],
        )
    }

    pub fn delete_unreferenced_episodes_older_than(&self, cutoff: &str) -> rusqlite::Result<usize> {
        self.conn().execute(
            "DELETE FROM violation_episodes WHERE closed_ts < ? AND NOT EXISTS(
             SELECT 1 FROM events WHERE events.episode_id=violation_episodes.id)",
            [cutoff],
        )
    }
}
