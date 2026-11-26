use rusqlite::{params, Connection};

use crate::{chrono_now, Store};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertDispatchKind {
    Initial,
    AwaitReview,
    Upgrade,
}

impl AlertDispatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initial | Self::AwaitReview => "initial",
            Self::Upgrade => "upgrade",
        }
    }
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS alert_outbox(
        id INTEGER PRIMARY KEY, event_id INTEGER NOT NULL, kind TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'pending',
        created_ts TEXT NOT NULL,
        UNIQUE(event_id,kind));
    CREATE INDEX IF NOT EXISTS idx_alert_outbox_status ON alert_outbox(status,id);
    CREATE TABLE IF NOT EXISTS alert_dedup_state(
        camera_id INTEGER NOT NULL, event_type TEXT NOT NULL, track_id INTEGER NOT NULL,
        sent_at INTEGER NOT NULL, PRIMARY KEY(camera_id,event_type,track_id));
    CREATE INDEX IF NOT EXISTS idx_alert_dedup_age ON alert_dedup_state(sent_at);
    CREATE TRIGGER IF NOT EXISTS prune_event_outbox AFTER DELETE ON events
    BEGIN DELETE FROM alert_outbox WHERE event_id=OLD.id; END;";

pub struct AlertDeliveryOutcome<'a> {
    pub status: &'a str,
    pub error: Option<&'a str>,
    pub latency_ms: i64,
    pub receipt: Option<i64>,
    pub receipt_summary: Option<&'a str>,
    pub retry_after: Option<&'a str>,
}

pub(crate) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let columns = conn
        .prepare("PRAGMA table_info(alert_deliveries)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
    for (column, kind) in [
        ("idempotency_key", "TEXT"),
        ("next_attempt_ts", "TEXT"),
        ("receipt_code", "INTEGER"),
        ("receipt_body", "TEXT"),
        ("latency_ms", "INTEGER"),
        ("delivered_ts", "TEXT"),
        ("message_text", "TEXT"),
    ] {
        if !columns.contains(column) {
            conn.execute(
                &format!("ALTER TABLE alert_deliveries ADD COLUMN {column} {kind}"),
                [],
            )?;
        }
    }
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_alert_delivery_idempotency
         ON alert_deliveries(idempotency_key) WHERE idempotency_key IS NOT NULL;",
    )
}

pub(crate) fn enqueue(
    conn: &Connection,
    event_id: i64,
    kind: AlertDispatchKind,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "INSERT INTO alert_outbox(event_id,kind,created_ts,status)
         SELECT id,?2,?3,?4 FROM events WHERE id=?1
         ON CONFLICT(event_id,kind) DO UPDATE SET status='pending'
         WHERE alert_outbox.status='awaiting_review' AND excluded.status='pending'",
        params![
            event_id,
            kind.as_str(),
            chrono_now(),
            if kind == AlertDispatchKind::AwaitReview {
                "awaiting_review"
            } else {
                "pending"
            }
        ],
    )?;
    Ok(changed > 0)
}

pub(crate) fn resolve_review(
    conn: &Connection,
    event_id: i64,
    suppressed: bool,
) -> rusqlite::Result<()> {
    if suppressed {
        conn.execute(
            "UPDATE alert_outbox SET status='suppressed'
             WHERE event_id=? AND kind='initial' AND status='awaiting_review'",
            [event_id],
        )?;
    } else {
        enqueue(conn, event_id, AlertDispatchKind::Initial)?;
    }
    Ok(())
}

pub(crate) fn recover_reviews(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE events SET vlm_verdict='{\"status\":\"unreviewed\",\"reason\":\"interrupted\"}',vlm_ts=?
         WHERE vlm_verdict IS NULL AND id IN (SELECT event_id FROM alert_outbox WHERE status='awaiting_review')",
        [chrono_now()],
    )?;
    conn.execute(
        "UPDATE alert_outbox SET status=CASE WHEN
            EXISTS(SELECT 1 FROM events WHERE id=event_id AND vlm_suppressed!=0)
            THEN 'suppressed' ELSE 'pending' END WHERE status='awaiting_review'",
        [],
    )
}

impl Store {
    /// Highest approved alert still active. A closed episode can never drive an output.
    pub fn actuator_level(&self, now: &str, pulse_start: &str) -> rusqlite::Result<u8> {
        self.conn().query_row(
            "WITH candidates AS (
                SELECT e.id, e.camera_id, e.type, e.level, e.status, e.vlm_suppressed, e.resolved_ts
                FROM violation_episodes ep JOIN events e ON e.episode_id=ep.id
                WHERE ep.closed_ts IS NULL
                UNION ALL
                SELECT id, camera_id, type, level, status, vlm_suppressed, resolved_ts FROM events
                WHERE episode_id IS NULL AND ts BETWEEN ?2 AND ?1
             )
             SELECT COALESCE(MAX(e.level), 0) FROM candidates e
             WHERE e.level BETWEEN 1 AND 3 AND e.resolved_ts IS NULL AND e.type<>'alert_resolved'
               AND COALESCE(e.vlm_suppressed,0)=0 AND e.status IN ('new','acked')
               AND EXISTS (SELECT 1 FROM alert_outbox o WHERE o.event_id=e.id AND o.status IN ('pending','planned'))
               AND NOT EXISTS (SELECT 1 FROM alert_silences s WHERE s.until_ts>?1 AND (
                  s.scope IN ('global','all') OR (s.scope='camera' AND s.target=CAST(e.camera_id AS TEXT))
                  OR (s.scope='type' AND s.target=e.type) OR (s.scope='level' AND s.target=CAST(e.level AS TEXT))))",
            params![now, pulse_start],
            |row| row.get(0),
        )
    }

    pub fn alert_on_cooldown(
        &self,
        event: &crate::Event,
        cooldown_s: i64,
    ) -> rusqlite::Result<bool> {
        self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM alert_dedup_state WHERE camera_id=?1 AND event_type=?2
               AND track_id=?3 AND sent_at>unixepoch()-?4)",
            params![event.camera_id, event.type_, event.track_id, cooldown_s],
            |row| row.get(0),
        )
    }

    pub fn enqueue_alert(&self, event_id: i64, kind: AlertDispatchKind) -> rusqlite::Result<bool> {
        enqueue(self.conn(), event_id, kind)
    }

    pub fn pending_alerts(&self) -> rusqlite::Result<Vec<(i64, i64, AlertDispatchKind)>> {
        let mut stmt = self.conn().prepare(
            "SELECT id,event_id,kind FROM alert_outbox WHERE status='pending' ORDER BY id LIMIT 20",
        )?;
        let rows = stmt.query_map([], |row| {
            let kind: String = row.get(2)?;
            let kind = if kind == "upgrade" {
                AlertDispatchKind::Upgrade
            } else {
                AlertDispatchKind::Initial
            };
            Ok((row.get(0)?, row.get(1)?, kind))
        })?;
        rows.collect()
    }

    pub fn plan_alert_deliveries(
        &self,
        outbox_id: i64,
        candidates: &[(String, String)],
        cooldown_s: i64,
    ) -> rusqlite::Result<bool> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.conn(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let changed = tx.execute(
            "UPDATE alert_outbox SET status='planned' WHERE id=? AND status='pending'",
            [outbox_id],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        if !candidates.is_empty() && cooldown_s > 0 {
            tx.execute(
                "DELETE FROM alert_dedup_state WHERE sent_at <= unixepoch()-3600",
                [],
            )?;
            let eligible = tx.execute(
                "INSERT INTO alert_dedup_state(camera_id,event_type,track_id,sent_at)
                 SELECT e.camera_id,e.type,COALESCE(e.track_id,0),unixepoch() FROM alert_outbox o
                   JOIN events e ON e.id=o.event_id WHERE o.id=?1
                 ON CONFLICT(camera_id,event_type,track_id) DO UPDATE SET sent_at=excluded.sent_at
                 WHERE (SELECT kind FROM alert_outbox WHERE id=?1)='upgrade'
                    OR alert_dedup_state.sent_at <= excluded.sent_at-?2",
                params![outbox_id, cooldown_s],
            )?;
            if eligible == 0 {
                tx.commit()?;
                return Ok(true);
            }
        }
        let now = chrono_now();
        for (channel, message) in candidates {
            tx.execute(
                "INSERT INTO alert_deliveries(event_id,channel,status,attempts,updated_ts,idempotency_key,message_text,next_attempt_ts)
                 SELECT event_id,?2,'pending',0,?3,'labsentinel-'||lower(hex(randomblob(16))),?4,?3
                 FROM alert_outbox WHERE id=?1",
                params![outbox_id,channel,now,message],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn begin_outbox_delivery(&self, id: i64) -> rusqlite::Result<Option<(String, String)>> {
        use rusqlite::OptionalExtension;
        self.conn().query_row(
            "UPDATE alert_deliveries SET status='sending', updated_ts=?2
             WHERE id=?1 AND idempotency_key IS NOT NULL AND status IN ('pending','retry') AND next_attempt_ts<=?2
             RETURNING idempotency_key,COALESCE(message_text,'')",
            params![id,chrono_now()], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()
    }

    pub fn finish_outbox_delivery(
        &self,
        id: i64,
        outcome: AlertDeliveryOutcome<'_>,
    ) -> rusqlite::Result<bool> {
        let now = chrono_now();
        Ok(self.conn().execute(
            "UPDATE alert_deliveries SET status=?2, attempts=attempts+1, updated_ts=?3, last_error=?4,
             latency_ms=?5, receipt_code=?6, delivered_ts=CASE WHEN ?2='ok' THEN ?3 END,
             next_attempt_ts=?7,receipt_body=?8 WHERE id=?1 AND status='sending'",
            params![id,outcome.status,now,outcome.error,outcome.latency_ms,outcome.receipt,outcome.retry_after,outcome.receipt_summary],
        )? > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventEvidence, NewEvent};

    #[test]
    fn actuator_prioritizes_ready_alerts_and_resets_after_recovery() {
        use crate::{EpisodeEvidence, NewEpisode};

        let store = Store::open(":memory:").unwrap();
        store.ensure_schema().unwrap();
        let at = "2026-11-25T01:00:15";
        let now = "2026-11-25T01:00:20";
        let pulse_start = "2026-11-25T01:00:05";
        let make_event = |camera_id, level, dispatch, episode_id, ts| {
            store
                .add_event_with_outbox(
                    &NewEvent {
                        camera_id,
                        type_: "no_mask",
                        level,
                        track_id: 1,
                        detail: "fixture",
                        snapshot: None,
                        ts,
                    },
                    EventEvidence {
                        episode_id,
                        ..Default::default()
                    },
                    None,
                    Some(dispatch),
                )
                .unwrap()
        };
        let episode_id = store
            .add_episode(&NewEpisode {
                camera_id: 1,
                rule_id: 1,
                zone_id: None,
                track_id: 1,
                behavior: "no_mask",
                evidence: EpisodeEvidence {
                    first_ts: at,
                    last_ts: at,
                    duration_s: 1.0,
                    hits: 1,
                    peak_conf: 0.9,
                    tier: "alert",
                },
            })
            .unwrap();
        let _warning = make_event(1, 2, AlertDispatchKind::Initial, Some(episode_id), at);
        let awaiting = make_event(2, 3, AlertDispatchKind::AwaitReview, None, at);
        let pulse = make_event(3, 3, AlertDispatchKind::Initial, None, at);
        make_event(
            4,
            3,
            AlertDispatchKind::Initial,
            None,
            "2026-11-25T00:00:00",
        );
        assert_eq!(store.actuator_level(now, pulse_start).unwrap(), 3);
        store
            .set_event_status(pulse, "false_positive", None, None)
            .unwrap();
        assert_eq!(store.actuator_level(now, pulse_start).unwrap(), 2);
        store
            .finish_event_vlm(awaiting, "{}", at, true, true)
            .unwrap();
        let silence = store
            .add_silence("camera", Some("1"), None, "2026-11-25T01:00:30")
            .unwrap();
        assert_eq!(store.actuator_level(now, pulse_start).unwrap(), 0);
        store.delete_silence(silence).unwrap();
        assert_eq!(store.actuator_level(now, pulse_start).unwrap(), 2);
        store
            .close_episode(episode_id, now, "recovered", None)
            .unwrap();
        assert_eq!(store.actuator_level(now, pulse_start).unwrap(), 0);
    }

    #[test]
    fn event_and_alert_intent_commit_together() {
        let store = Store::open(":memory:").unwrap();
        store.ensure_schema().unwrap();
        let event = NewEvent {
            camera_id: 7,
            type_: "no_mask",
            level: 2,
            track_id: 1,
            detail: "fixture",
            snapshot: None,
            ts: "2026-11-25T00:00:00",
        };
        let insert = || {
            store.add_event_with_outbox(
                &event,
                EventEvidence::default(),
                None,
                Some(AlertDispatchKind::Initial),
            )
        };
        store
            .conn()
            .execute_batch(
                "CREATE TRIGGER reject_outbox BEFORE INSERT ON alert_outbox
             BEGIN SELECT RAISE(ABORT,'outbox failed'); END;",
            )
            .unwrap();
        assert!(insert().is_err());
        assert!(store.event_by_id(1).unwrap().is_none());
        store
            .conn()
            .execute_batch("DROP TRIGGER reject_outbox;")
            .unwrap();
        let id = insert().unwrap();
        assert!(!store.enqueue_alert(id, AlertDispatchKind::Initial).unwrap());
        assert!(store.enqueue_alert(id, AlertDispatchKind::Upgrade).unwrap());
        assert_eq!(
            store
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM alert_outbox WHERE event_id=?",
                    [id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
    }

    #[test]
    fn recovery_skips_suppressed_reviews_and_does_not_resend_uncertain_deliveries() {
        let store = Store::open(":memory:").unwrap();
        store.ensure_schema().unwrap();
        let add = |kind| {
            store
                .add_event_with_outbox(
                    &NewEvent {
                        camera_id: 1,
                        type_: "no_mask",
                        level: 2,
                        track_id: 2,
                        detail: "fixture",
                        snapshot: None,
                        ts: "2026-11-25T01:00:00",
                    },
                    EventEvidence::default(),
                    None,
                    Some(kind),
                )
                .unwrap()
        };
        let pending = add(AlertDispatchKind::AwaitReview);
        let suppressed = add(AlertDispatchKind::AwaitReview);
        store
            .finish_event_vlm(suppressed, "{}", "2026-11-25T01:00:01", true, true)
            .unwrap();
        let first = add(AlertDispatchKind::Initial);
        let (outbox, event_id, _) = store.pending_alerts().unwrap()[0];
        assert_eq!(event_id, first);
        store
            .plan_alert_deliveries(outbox, &[("webhook".into(), "fixture".into())], 60)
            .unwrap();
        let delivery_id = store.deliveries(None, 1).unwrap()[0].id;
        let (key, _) = store.begin_outbox_delivery(delivery_id).unwrap().unwrap();
        assert!(store.begin_outbox_delivery(delivery_id).unwrap().is_none());

        assert_eq!(store.recover_interrupted_deliveries().unwrap(), 2);
        assert_eq!(
            store.delivery_by_id(delivery_id).unwrap().unwrap().1,
            "unconfirmed"
        );
        assert!(store.begin_outbox_delivery(delivery_id).unwrap().is_none());
        let recovered = store.pending_alerts().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].1, pending);
        let row = store.event_by_id(pending).unwrap().unwrap();
        assert!(row.vlm_verdict.as_deref().unwrap().contains("interrupted"));
        assert!(store.alert_on_cooldown(&row, 60).unwrap());
        assert!(store.retry_delivery(delivery_id).unwrap());
        assert_eq!(
            store.begin_outbox_delivery(delivery_id).unwrap().unwrap().0,
            key
        );
    }
}
