use ls_core::lab::LabRules;
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};

use crate::Store;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ref_lines (
    camera_id INTEGER NOT NULL REFERENCES cameras(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    zone_id INTEGER REFERENCES zones(id) ON DELETE CASCADE,
    config_json TEXT NOT NULL CHECK(json_valid(config_json)),
    PRIMARY KEY(camera_id,idx)
);
CREATE TABLE IF NOT EXISTS camera_ppe_policies (
    camera_id INTEGER NOT NULL REFERENCES cameras(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    zone_id INTEGER REFERENCES zones(id) ON DELETE CASCADE,
    config_json TEXT NOT NULL CHECK(json_valid(config_json)),
    PRIMARY KEY(camera_id,idx)
);
CREATE TABLE IF NOT EXISTS camera_vlm_sop (
    camera_id INTEGER NOT NULL REFERENCES cameras(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    zone_id INTEGER REFERENCES zones(id) ON DELETE CASCADE,
    config_json TEXT NOT NULL CHECK(json_valid(config_json)),
    PRIMARY KEY(camera_id,idx)
);
CREATE TRIGGER IF NOT EXISTS delete_camera_lab_configuration AFTER DELETE ON cameras BEGIN
    DELETE FROM ref_lines WHERE camera_id=OLD.id;
    DELETE FROM camera_ppe_policies WHERE camera_id=OLD.id;
    DELETE FROM camera_vlm_sop WHERE camera_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS delete_zone_lab_configuration AFTER DELETE ON zones BEGIN
    DELETE FROM ref_lines WHERE zone_id=OLD.id;
    DELETE FROM camera_ppe_policies WHERE zone_id=OLD.id;
END;";

fn rows<T: DeserializeOwned>(
    conn: &Connection,
    sql: &str,
    camera: i64,
) -> rusqlite::Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let items = stmt.query_map([camera], |row| {
        let payload: String = row.get(0)?;
        serde_json::from_str(&payload).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    })?;
    items.collect()
}

fn write_rows<T: Serialize>(
    connection: &Connection,
    sql: &str,
    camera: i64,
    rules: impl IntoIterator<Item = (Option<i64>, T)>,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(sql)?;
    for (index, (zone, rule)) in rules.into_iter().enumerate() {
        if let Some(zone) = zone {
            let allowed: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM zones WHERE id=? AND camera_id=?)",
                params![zone, camera],
                |row| row.get(0),
            )?;
            if !allowed {
                return Err(rusqlite::Error::InvalidParameterName(
                    "规则引用了其他机位或已删除的区域".into(),
                ));
            }
        }
        let payload = serde_json::to_string(&rule)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        statement.execute(params![camera, index as i64, zone, payload])?;
    }
    Ok(())
}

impl Store {
    pub fn lab_rules(&self, camera: i64) -> rusqlite::Result<LabRules> {
        Ok(LabRules {
            reference_lines: rows(
                self.conn(),
                "SELECT config_json FROM ref_lines WHERE camera_id=? ORDER BY idx",
                camera,
            )?,
            ppe_policies: rows(
                self.conn(),
                "SELECT config_json FROM camera_ppe_policies WHERE camera_id=? ORDER BY idx",
                camera,
            )?,
            fixed_checks: rows(
                self.conn(),
                "SELECT config_json FROM camera_vlm_sop WHERE camera_id=? ORDER BY idx",
                camera,
            )?,
        })
    }

    pub fn replace_lab_rules(&self, camera: i64, rules: &LabRules) -> rusqlite::Result<()> {
        let tx = Transaction::new_unchecked(self.conn(), TransactionBehavior::Immediate)?;
        self.write_lab_rules(camera, rules)?;
        tx.commit()
    }

    /// The caller holds a transaction for the entire policy change.
    pub fn write_lab_rules(&self, camera: i64, rules: &LabRules) -> rusqlite::Result<()> {
        rules
            .validate()
            .map_err(|message| rusqlite::Error::InvalidParameterName(message.into()))?;
        let conn = self.conn();
        let _id: i64 = conn.query_row("SELECT id FROM cameras WHERE id=?", [camera], |row| {
            row.get(0)
        })?;
        for sql in [
            "DELETE FROM ref_lines WHERE camera_id=?",
            "DELETE FROM camera_ppe_policies WHERE camera_id=?",
            "DELETE FROM camera_vlm_sop WHERE camera_id=?",
        ] {
            conn.execute(sql, [camera])?;
        }
        write_rows(
            conn,
            "INSERT INTO ref_lines(camera_id,idx,zone_id,config_json) VALUES(?,?,?,?)",
            camera,
            rules
                .reference_lines
                .iter()
                .map(|rule| (rule.zone_id, rule)),
        )?;
        write_rows(
            conn,
            "INSERT INTO camera_ppe_policies(camera_id,idx,zone_id,config_json) VALUES(?,?,?,?)",
            camera,
            rules.ppe_policies.iter().map(|rule| (rule.zone_id, rule)),
        )?;
        write_rows(
            conn,
            "INSERT INTO camera_vlm_sop(camera_id,idx,zone_id,config_json) VALUES(?,?,?,?)",
            camera,
            rules.fixed_checks.iter().map(|rule| (None, rule)),
        )
    }
}
