use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

use crate::Store;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS face_people (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    profile TEXT NOT NULL,
    subject TEXT NOT NULL,
    remote_subject TEXT,
    consent_ts TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    removing INTEGER NOT NULL DEFAULT 0,
    UNIQUE(profile, subject)
);
CREATE TABLE IF NOT EXISTS face_templates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    person_id INTEGER NOT NULL,
    template TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_face_templates_person ON face_templates(person_id);
CREATE TABLE IF NOT EXISTS face_event_matches (
    event_id INTEGER PRIMARY KEY,
    person_id INTEGER NOT NULL,
    similarity REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_face_matches_person ON face_event_matches(person_id);
CREATE TABLE IF NOT EXISTS face_remote_removals (
    profile TEXT NOT NULL,
    remote_subject TEXT NOT NULL,
    PRIMARY KEY(profile, remote_subject)
);
";

#[derive(Debug, Serialize)]
pub struct FacePerson {
    pub id: i64,
    pub profile: String,
    pub subject: String,
    #[serde(skip)]
    pub remote_subject: Option<String>,
    pub consent_ts: String,
    pub expires_at: i64,
    pub removing: bool,
    pub samples: i64,
}

impl FacePerson {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            profile: row.get(1)?,
            subject: row.get(2)?,
            remote_subject: row.get(3)?,
            consent_ts: row.get(4)?,
            expires_at: row.get(5)?,
            removing: row.get(6)?,
            samples: row.get(7)?,
        })
    }
}

pub struct FaceTemplate {
    pub person_id: i64,
    pub subject: String,
    pub encrypted: String,
}

const PEOPLE_QUERY: &str =
    "SELECT id,profile,subject,remote_subject,consent_ts,expires_at,removing,
    (SELECT COUNT(*) FROM face_templates t WHERE t.person_id=p.id) FROM face_people p";

impl Store {
    pub fn face_people(&self) -> rusqlite::Result<Vec<FacePerson>> {
        let sql = format!("{PEOPLE_QUERY} ORDER BY subject,profile");
        let mut statement = self.conn().prepare(&sql)?;
        let rows = statement.query_map([], FacePerson::from_row)?;
        rows.collect()
    }

    pub fn face_person(
        &self,
        profile: &str,
        subject: &str,
    ) -> rusqlite::Result<Option<FacePerson>> {
        let sql = format!("{PEOPLE_QUERY} WHERE profile=? AND subject=?");
        self.conn()
            .query_row(&sql, params![profile, subject], FacePerson::from_row)
            .optional()
    }

    pub fn face_person_by_id(&self, id: i64) -> rusqlite::Result<Option<FacePerson>> {
        let sql = format!("{PEOPLE_QUERY} WHERE id=?");
        self.conn()
            .query_row(&sql, params![id], FacePerson::from_row)
            .optional()
    }

    pub fn save_face_person(
        &self,
        person: &FacePerson,
        samples: &[String],
    ) -> rusqlite::Result<i64> {
        let tx = self.conn().unchecked_transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT remote_subject FROM face_people WHERE profile=? AND subject=?",
                params![person.profile, person.subject],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let id: i64 = tx.query_row(
            "INSERT INTO face_people(profile,subject,remote_subject,consent_ts,expires_at)
             VALUES(?,?,?,?,?) ON CONFLICT(profile,subject) DO UPDATE SET
               remote_subject=excluded.remote_subject,consent_ts=excluded.consent_ts,expires_at=excluded.expires_at
             WHERE face_people.removing=0
             RETURNING id",
            params![person.profile, person.subject, person.remote_subject, person.consent_ts, person.expires_at],
            |row| row.get(0),
        )?;
        if let Some(subject) = person.remote_subject.as_deref() {
            tx.execute("DELETE FROM face_templates WHERE person_id=?", [id])?;
            tx.execute(
                "DELETE FROM face_remote_removals WHERE profile=? AND remote_subject=?",
                params![person.profile, subject],
            )?;
            if let Some(old_subject) = previous.as_deref().filter(|old| *old != subject) {
                self.queue_face_remote_removal(&person.profile, old_subject)?;
            }
        }
        for sample in samples {
            tx.execute(
                "INSERT INTO face_templates(person_id,template) VALUES(?,?)",
                params![id, sample],
            )?;
        }
        if person.remote_subject.is_none() {
            tx.execute(
                "DELETE FROM face_templates WHERE person_id=? AND id NOT IN
                (SELECT id FROM face_templates WHERE person_id=? ORDER BY id DESC LIMIT 10)",
                params![id, id],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    pub fn queue_face_remote_removal(&self, profile: &str, subject: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO face_remote_removals(profile,remote_subject) VALUES(?,?)",
            params![profile, subject],
        )?;
        Ok(())
    }

    pub fn face_remote_removals(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut query = self
            .conn()
            .prepare("SELECT profile,remote_subject FROM face_remote_removals ORDER BY rowid")?;
        let rows = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    pub fn clear_face_remote_removal(&self, profile: &str, subject: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "DELETE FROM face_remote_removals WHERE profile=? AND remote_subject=?",
            params![profile, subject],
        )?;
        Ok(())
    }

    pub fn face_templates(&self, profile: &str, now: i64) -> rusqlite::Result<Vec<FaceTemplate>> {
        let mut statement = self.conn().prepare(
            "SELECT p.id,p.subject,t.template FROM face_people p JOIN face_templates t ON t.person_id=p.id
             WHERE p.profile=? AND p.removing=0 AND p.expires_at>?",
        )?;
        let rows = statement.query_map(params![profile, now], |row| {
            Ok(FaceTemplate {
                person_id: row.get(0)?,
                subject: row.get(1)?,
                encrypted: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn face_remote_match(
        &self,
        profile: &str,
        remote: &str,
        now: i64,
    ) -> rusqlite::Result<Option<(i64, String)>> {
        self.conn().query_row(
            "SELECT id,subject FROM face_people WHERE profile=? AND remote_subject=? AND removing=0 AND expires_at>?",
            params![profile, remote, now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()
    }

    pub fn mark_face_removing(&self, id: i64) -> rusqlite::Result<()> {
        self.conn()
            .execute("UPDATE face_people SET removing=1 WHERE id=?", [id])?;
        Ok(())
    }

    pub fn delete_face_person(&self, id: i64) -> rusqlite::Result<()> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute("DELETE FROM face_event_matches WHERE person_id=?", [id])?;
        tx.execute("DELETE FROM face_templates WHERE person_id=?", [id])?;
        tx.execute("DELETE FROM face_people WHERE id=?", [id])?;
        tx.commit()
    }

    pub fn save_face_match(
        &self,
        event: i64,
        person: i64,
        similarity: f32,
        now: i64,
    ) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO face_event_matches(event_id,person_id,similarity)
             SELECT ?,id,? FROM face_people WHERE id=? AND removing=0 AND expires_at>?
                AND EXISTS(SELECT 1 FROM events WHERE id=?)",
            params![event, similarity, person, now, event],
        )?;
        Ok(())
    }

    pub fn add_face_alert(
        &self,
        event: &crate::NewEvent<'_>,
        person: i64,
        similarity: f32,
        now: i64,
        since: &str,
    ) -> rusqlite::Result<Option<i64>> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let inserted = tx.execute(
            "INSERT INTO events(ts,camera_id,type,level,track_id,detail)
             SELECT ?1,?2,'face_match',?3,?4,?5
             WHERE EXISTS (SELECT 1 FROM cameras WHERE id=?2)
               AND EXISTS (SELECT 1 FROM face_people WHERE id=?6 AND removing=0 AND expires_at>?7)
               AND NOT EXISTS (
                 SELECT 1 FROM face_event_matches m JOIN events e ON e.id=m.event_id
                 WHERE m.person_id=?6 AND e.camera_id=?2 AND e.type='face_match' AND e.ts>=?8
               )",
            params![
                event.ts,
                event.camera_id,
                event.level,
                event.track_id,
                event.detail,
                person,
                now,
                since
            ],
        )?;
        if inserted == 0 {
            return Ok(None);
        }
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO face_event_matches(event_id,person_id,similarity) VALUES(?,?,?)",
            params![id, person, similarity],
        )?;
        tx.commit()?;
        Ok(Some(id))
    }

    pub fn event_face_match(
        &self,
        event: i64,
        now: i64,
    ) -> rusqlite::Result<Option<(String, f32)>> {
        self.conn().query_row(
            "SELECT p.subject,m.similarity FROM face_event_matches m JOIN face_people p ON p.id=m.person_id
             WHERE m.event_id=? AND p.removing=0 AND p.expires_at>?
                AND EXISTS(SELECT 1 FROM events WHERE id=?)",
            params![event,now,event],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()
    }

    pub fn prune_face_matches(&self) -> rusqlite::Result<()> {
        self.conn().execute(
            "DELETE FROM face_event_matches WHERE NOT EXISTS (SELECT 1 FROM events WHERE id=event_id)", [],
        )?;
        Ok(())
    }
}
