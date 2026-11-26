use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::{chrono_now, Store};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS recording_segments(
        event_id INTEGER PRIMARY KEY, state TEXT NOT NULL, codec TEXT,
        start_ts TEXT, end_ts TEXT, reason TEXT, updated_ts TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS recording_gops(
        event_id INTEGER NOT NULL, ts TEXT NOT NULL, byte_offset INTEGER NOT NULL,
        PRIMARY KEY(event_id, byte_offset));
    CREATE TRIGGER IF NOT EXISTS prune_event_recording AFTER DELETE ON events BEGIN
        DELETE FROM recording_segments WHERE event_id=OLD.id;
        DELETE FROM recording_gops WHERE event_id=OLD.id;
    END;
    CREATE TRIGGER IF NOT EXISTS prune_unavailable_gops AFTER UPDATE OF video ON events
    WHEN NEW.video IS NULL BEGIN
        DELETE FROM recording_gops WHERE event_id=OLD.id;
    END;";

#[derive(Debug, Serialize)]
pub struct EventRecording {
    pub state: String,
    pub codec: Option<String>,
    pub start_ts: Option<String>,
    pub end_ts: Option<String>,
    pub reason: Option<String>,
}

impl Store {
    pub fn start_event_recording(&self, event: i64) -> rusqlite::Result<bool> {
        Ok(self.conn().execute(
            "INSERT OR IGNORE INTO recording_segments(event_id,state,updated_ts)
             SELECT id,'capturing',?2 FROM events WHERE id=?1 AND video IS NULL",
            params![event, chrono_now()],
        )? > 0)
    }

    pub fn event_recording_failed(&self, event: i64, reason: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE recording_segments SET state='unavailable',reason=?2,updated_ts=?3
             WHERE event_id=?1 AND state='capturing'",
            params![event, reason, chrono_now()],
        )?;
        Ok(())
    }

    pub fn recover_interrupted_recordings(&self) -> rusqlite::Result<usize> {
        self.conn().execute(
            "UPDATE recording_segments SET state='unavailable',reason='interrupted',updated_ts=?
             WHERE state='capturing'",
            [chrono_now()],
        )
    }

    pub fn save_event_recording(
        &self,
        event: i64,
        video_path: &str,
        metadata: &EventRecording,
        gops: &[(String, i64)],
    ) -> rusqlite::Result<bool> {
        let tx = self.conn().unchecked_transaction()?;
        let updated = tx.execute(
            "UPDATE recording_segments SET state=?2,codec=?3,start_ts=?4,end_ts=?5,reason=?6,updated_ts=?7
             WHERE event_id=?1 AND state='capturing'",
            params![event, metadata.state, metadata.codec, metadata.start_ts, metadata.end_ts, metadata.reason, chrono_now()],
        )?;
        if updated == 0
            || tx.execute(
                "UPDATE events SET video=?2 WHERE id=?1",
                params![event, video_path],
            )? == 0
        {
            return Ok(false);
        }
        for (timestamp, offset) in gops {
            tx.execute(
                "INSERT INTO recording_gops(event_id,ts,byte_offset) VALUES(?,?,?)",
                params![event, timestamp, offset],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn event_recording(&self, event: i64) -> rusqlite::Result<Option<EventRecording>> {
        self.conn().query_row(
            "SELECT CASE WHEN v.video IS NULL AND s.state IN ('original','reencoded') THEN 'expired' ELSE s.state END,
                    s.codec,s.start_ts,s.end_ts,s.reason
             FROM recording_segments s JOIN events v ON v.id=s.event_id WHERE s.event_id=?",
            [event],
            |r| Ok(EventRecording {state:r.get(0)?, codec:r.get(1)?, start_ts:r.get(2)?, end_ts:r.get(3)?, reason:r.get(4)?}),
        ).optional()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_clip_has_keyframe_index_and_expires_with_its_video() {
        let store = Store::open(":memory:").unwrap();
        store.ensure_schema().unwrap();
        let event = store
            .add_check_event(
                4,
                "motion_recording",
                "fixture",
                "2026-11-25T00:00:01",
                None,
            )
            .unwrap();
        assert!(store.start_event_recording(event).unwrap());
        assert!(!store.start_event_recording(event).unwrap());
        let info = EventRecording {
            state: "original".into(),
            codec: Some("hevc".into()),
            start_ts: Some("2026-11-25T00:00:00".into()),
            end_ts: Some("2026-11-25T00:00:03".into()),
            reason: None,
        };
        let start = info.start_ts.as_ref().unwrap();
        assert!(store
            .save_event_recording(event, "videos/event.mp4", &info, &[(start.clone(), 131)])
            .unwrap());
        let offset: i64 = store
            .conn()
            .query_row(
                "SELECT byte_offset FROM recording_gops WHERE event_id=?",
                [event],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(offset, 131);
        assert_eq!(
            store.event_recording(event).unwrap().unwrap().state,
            "original"
        );
        store
            .conn()
            .execute("UPDATE events SET video=NULL WHERE id=?", [event])
            .unwrap();
        assert_eq!(
            store.event_recording(event).unwrap().unwrap().state,
            "expired"
        );
        let count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM recording_gops", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
