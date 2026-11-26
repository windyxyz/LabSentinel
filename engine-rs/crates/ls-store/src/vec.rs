use std::sync::OnceLock;

use rusqlite::{ffi, params, OptionalExtension, Transaction, TransactionBehavior};

use crate::{Event, Store};

pub const EMBEDDING_DIMS: usize = 768;
const VECTOR_READ_MAP_BYTES: i64 = 128 * 1024 * 1024;

pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS event_embeddings_meta(
    event_id INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
    encoder TEXT NOT NULL, snapshot_sha256 TEXT NOT NULL, indexed_at TEXT NOT NULL,
    snapshot_path TEXT NOT NULL, event_snapshot_sha256 TEXT);
    CREATE VIRTUAL TABLE IF NOT EXISTS vec_event_embeddings
        USING vec0(embedding float[768] distance_metric=cosine, camera_id INTEGER partition key);
    CREATE TRIGGER IF NOT EXISTS prune_event_embeddings AFTER DELETE ON events
    BEGIN DELETE FROM event_embeddings_meta WHERE event_id=OLD.id; END;";

#[allow(unsafe_code)]
pub(crate) fn register() -> rusqlite::Result<()> {
    type Init = unsafe extern "C" fn(
        *mut ffi::sqlite3,
        *mut *mut std::ffi::c_char,
        *const ffi::sqlite3_api_routines,
    ) -> std::ffi::c_int;
    static STATUS: OnceLock<std::ffi::c_int> = OnceLock::new();
    let result = *STATUS.get_or_init(|| unsafe {
        // sqlite-vec exports SQLite's C extension entry point as an untyped fn.
        let init =
            std::mem::transmute::<*const (), Init>(sqlite_vec::sqlite3_vec_init as *const ());
        ffi::sqlite3_auto_extension(Some(init))
    });
    if result == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(result),
            Some("sqlite-vec registration failed".into()),
        ))
    }
}

impl Store {
    pub fn next_unindexed_event(
        &self,
        encoder: &str,
        since: i64,
    ) -> rusqlite::Result<Option<Event>> {
        let id: Option<i64> = self
            .conn()
            .query_row(
                "SELECT e.id FROM events e LEFT JOIN event_embeddings_meta m ON e.id=m.event_id
             WHERE e.id>?1 AND e.snapshot IS NOT NULL AND
               (m.event_id IS NULL OR m.encoder!=?2 OR e.snapshot!=m.snapshot_path
                OR e.snapshot_sha256 IS NOT m.event_snapshot_sha256)
             ORDER BY e.id LIMIT 1",
                params![since, encoder],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|id| self.event_by_id(id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn event_embedding(
        &self,
        event: &Event,
        encoder: &str,
    ) -> rusqlite::Result<Option<Vec<f32>>> {
        let bytes: Option<Vec<u8>> = self.conn().query_row(
            "SELECT v.embedding FROM vec_event_embeddings v JOIN event_embeddings_meta m ON m.event_id=v.rowid
             WHERE v.rowid=?1 AND m.encoder=?2 AND m.snapshot_path=?3 AND m.event_snapshot_sha256 IS ?4",
            params![event.id, encoder, event.snapshot, event.snapshot_sha256],
            |row| row.get(0),
        ).optional()?;
        Ok(bytes
            .filter(|data| data.len() == EMBEDDING_DIMS * 4)
            .map(|data| {
                data.as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
                    .map(f32::from_le_bytes)
                    .collect()
            }))
    }

    pub fn index_event_embedding(
        &self,
        event: &Event,
        encoder: &str,
        image_sha256: &str,
        embedding: &[f32],
    ) -> rusqlite::Result<bool> {
        let bytes = embedding_bytes(embedding)?;
        let tx = Transaction::new_unchecked(self.conn(), TransactionBehavior::Immediate)?;
        let camera_id: Option<i64> = tx
            .query_row(
                "SELECT camera_id FROM events WHERE id=?1 AND snapshot IS ?2
             AND snapshot_sha256 IS ?3 AND snapshot IS NOT NULL",
                params![event.id, event.snapshot, event.snapshot_sha256],
                |row| row.get(0),
            )
            .optional()?;
        let Some(camera_id) = camera_id else {
            return Ok(false);
        };
        tx.execute("DELETE FROM vec_event_embeddings WHERE rowid=?", [event.id])?;
        tx.execute(
            "INSERT INTO vec_event_embeddings(rowid, embedding, camera_id) VALUES(?,?,?)",
            params![event.id, bytes, camera_id],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO event_embeddings_meta(
                event_id, encoder, snapshot_sha256, indexed_at, snapshot_path, event_snapshot_sha256
             ) VALUES(?,?,?,datetime('now'),?,?)",
            params![
                event.id,
                encoder,
                image_sha256,
                event.snapshot,
                event.snapshot_sha256
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn rank_event_embeddings(
        &self,
        encoder: &str,
        embedding: &[f32],
        camera_scope: Option<&[i64]>,
        skip_event: Option<i64>,
        limit: i64,
    ) -> rusqlite::Result<Vec<(i64, f32)>> {
        self.rank_event_embeddings_scoped(encoder, embedding, camera_scope, skip_event, limit, None)
    }

    pub fn rank_event_embeddings_scoped(
        &self,
        encoder: &str,
        embedding: &[f32],
        camera_scope: Option<&[i64]>,
        skip_event: Option<i64>,
        limit: i64,
        allowed_types: Option<&[&str]>,
    ) -> rusqlite::Result<Vec<(i64, f32)>> {
        if camera_scope == Some(&[]) {
            return Ok(Vec::new());
        }
        let bytes = embedding_bytes(embedding)?;
        let scope = camera_scope.map(|ids| serde_json::json!(ids).to_string());
        let types = allowed_types.map(|types| serde_json::json!(types).to_string());
        let conn = self.conn();
        // vec0 reads entire vector chunks. Map the WAL database only while
        // ranking so Windows can still truncate it during storage maintenance.
        let previous_map = if cfg!(windows)
            && conn
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                .is_ok_and(|mode| mode == "wal")
        {
            conn.pragma_query_value(None, "mmap_size", |row| row.get::<_, i64>(0))
                .ok()
                .filter(|&size| size < VECTOR_READ_MAP_BYTES)
                .filter(|_| {
                    conn.pragma_update(None, "mmap_size", VECTOR_READ_MAP_BYTES)
                        .is_ok()
                })
        } else {
            None
        };
        let result = (|| {
            let mut stmt = conn.prepare(
            "SELECT rowid,distance FROM vec_event_embeddings
             WHERE embedding MATCH ?1 AND k=?2 AND rowid IN (
               SELECT m.event_id FROM event_embeddings_meta m JOIN events e ON e.id=m.event_id
               WHERE m.encoder=?3 AND e.snapshot=m.snapshot_path AND e.snapshot_sha256 IS m.event_snapshot_sha256
                 AND (?4 IS NULL OR e.camera_id IN (SELECT value FROM json_each(?4)))
                 AND (?5 IS NULL OR e.id!=?5)
                 AND (?6 IS NULL OR e.type IN (SELECT value FROM json_each(?6)))
             ) ORDER BY distance",
            )?;
            let rows = stmt.query_map(
                params![
                    bytes,
                    limit.clamp(1, 100),
                    encoder,
                    scope,
                    skip_event,
                    types
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            rows.collect()
        })();
        if let Some(size) = previous_map {
            conn.pragma_update(None, "mmap_size", size)?;
        }
        result
    }

    pub fn prune_event_vectors(&self) -> rusqlite::Result<usize> {
        self.conn().execute(
            "DELETE FROM vec_event_embeddings WHERE rowid IN (
               SELECT v.rowid FROM vec_event_embeddings v LEFT JOIN event_embeddings_meta m ON m.event_id=v.rowid
               WHERE m.event_id IS NULL LIMIT 128
             )",
            [],
        )
    }
}

fn embedding_bytes(embedding: &[f32]) -> rusqlite::Result<Vec<u8>> {
    if embedding.len() != EMBEDDING_DIMS
        || embedding.iter().any(|value| !value.is_finite())
        || embedding.iter().all(|value| *value == 0.0)
    {
        return Err(rusqlite::Error::InvalidParameterName("embedding".into()));
    }
    Ok(embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewEvent;

    #[test]
    fn ranking_filters_before_top_k_and_ignores_old_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("vectors.db");
        let store = Store::open(path.to_str().unwrap()).unwrap();
        store.ensure_schema().unwrap();
        store.migrate().unwrap();
        let mut query = vec![0.0; EMBEDDING_DIMS];
        query[0] = 1.0;
        let mut ids = Vec::new();
        for (camera_id, path, score) in [(1, "a.jpg", 1.0), (2, "b.jpg", 0.8), (2, "c.jpg", 0.6)] {
            let id = store
                .add_event(&NewEvent {
                    camera_id,
                    type_: "no_mask",
                    level: 2,
                    track_id: 1,
                    detail: "",
                    snapshot: Some(path),
                    ts: "2026-11-25T12:00:00",
                })
                .unwrap();
            let mut vector = query.clone();
            vector[0] = score;
            vector[1] = (1.0_f32 - score * score).sqrt();
            let event = store.event_by_id(id).unwrap().unwrap();
            assert!(store
                .index_event_embedding(&event, "test", "hash", &vector)
                .unwrap());
            ids.push(id);
        }
        let rank = |scope, skip| {
            store
                .rank_event_embeddings("test", &query, scope, skip, 1)
                .unwrap()
        };
        assert_eq!(rank(None, None)[0].0, ids[0]);
        assert_eq!(rank(Some(&[2]), None)[0].0, ids[1]);
        assert_eq!(rank(Some(&[2]), Some(ids[1]))[0].0, ids[2]);
        assert!(rank(Some(&[]), None).is_empty());
        store
            .conn()
            .execute("UPDATE events SET snapshot='new.jpg' WHERE id=?", [ids[1]])
            .unwrap();
        assert_eq!(rank(Some(&[2]), None)[0].0, ids[2]);
        store
            .conn()
            .execute("DELETE FROM events WHERE id=?", [ids[2]])
            .unwrap();
        assert!(rank(Some(&[2]), None).is_empty());
        assert_eq!(store.prune_event_vectors().unwrap(), 1);
        if cfg!(windows) {
            let map: i64 = store
                .conn()
                .pragma_query_value(None, "mmap_size", |row| row.get(0))
                .unwrap();
            assert_eq!(map, 0);
        }
        store.conn().execute_batch("VACUUM").unwrap();
        assert_eq!(store.integrity_check().unwrap(), "ok");
    }
}
