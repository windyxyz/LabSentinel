use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

use crate::{chrono_now, Store};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS mcp_tokens(
        id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, token_prefix TEXT NOT NULL,
        token_hash TEXT NOT NULL UNIQUE, scope TEXT NOT NULL CHECK(scope IN ('read_only','policy')),
        camera_ids TEXT, created_at INTEGER NOT NULL, last_used_at INTEGER,
        expires_at INTEGER, revoked INTEGER NOT NULL DEFAULT 0);";

#[derive(Debug, Serialize)]
pub struct McpToken {
    pub id: i64,
    pub name: String,
    pub token_prefix: String,
    pub scope: String,
    pub camera_ids: Option<Vec<i64>>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revoked: bool,
}

pub struct NewMcpToken<'a> {
    pub name: &'a str,
    pub token_prefix: &'a str,
    pub token_hash: &'a str,
    pub scope: &'a str,
    pub camera_ids: Option<&'a [i64]>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
}

const TOKEN_COLS: &str =
    "id,name,token_prefix,scope,camera_ids,created_at,last_used_at,expires_at,revoked";

fn token_row(row: &Row<'_>) -> rusqlite::Result<McpToken> {
    let raw: Option<String> = row.get(4)?;
    let camera_ids = raw.map(|text| serde_json::from_str(&text).unwrap_or_default());
    Ok(McpToken {
        id: row.get(0)?,
        name: row.get(1)?,
        token_prefix: row.get(2)?,
        scope: row.get(3)?,
        camera_ids,
        created_at: row.get(5)?,
        last_used_at: row.get(6)?,
        expires_at: row.get(7)?,
        revoked: row.get(8)?,
    })
}

impl Store {
    pub fn create_mcp_token(&self, token: &NewMcpToken<'_>) -> rusqlite::Result<i64> {
        self.conn().query_row(
            "INSERT INTO mcp_tokens(name,token_prefix,token_hash,scope,camera_ids,created_at,expires_at)
             VALUES(?,?,?,?,?,?,?) RETURNING id",
            params![token.name, token.token_prefix, token.token_hash, token.scope,
                token.camera_ids.map(|ids| serde_json::json!(ids).to_string()), token.created_at, token.expires_at],
            |r| r.get(0),
        )
    }

    pub fn mcp_tokens(&self) -> rusqlite::Result<Vec<McpToken>> {
        let mut select = self.conn().prepare(&format!(
            "SELECT {TOKEN_COLS} FROM mcp_tokens ORDER BY id DESC"
        ))?;
        let rows = select.query_map([], token_row)?;
        rows.collect()
    }

    pub fn valid_mcp_token(&self, hash: &str, now: i64) -> rusqlite::Result<Option<McpToken>> {
        let token = self.conn().query_row(
            &format!("SELECT {TOKEN_COLS} FROM mcp_tokens WHERE token_hash=?1 AND revoked=0 AND (expires_at IS NULL OR expires_at>?2)"),
            params![hash, now], token_row,
        ).optional()?;
        if let Some(token) = &token {
            if token
                .last_used_at
                .is_none_or(|previous| now.saturating_sub(previous) >= 60)
            {
                self.conn().execute(
                    "UPDATE mcp_tokens SET last_used_at=?2 WHERE id=?1 AND revoked=0",
                    params![token.id, now],
                )?;
            }
        }
        Ok(token)
    }

    pub fn revoke_mcp_token(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self.conn().execute(
            "UPDATE mcp_tokens SET revoked=1 WHERE id=? AND revoked=0",
            [id],
        )? > 0)
    }

    pub fn mcp_audit_start(
        &self,
        request_id: &str,
        actor: &str,
        tool: &str,
        detail: &serde_json::Value,
    ) -> rusqlite::Result<i64> {
        self.conn().query_row(
            "INSERT INTO audit_log(ts,request_id,actor,action,status_code,detail) VALUES(?,?,?,?,202,?) RETURNING id",
            params![chrono_now(), request_id, actor, format!("mcp:{tool}"), detail.to_string()],
            |r| r.get(0),
        )
    }

    pub fn mcp_audit_result(
        &self,
        row: i64,
        code: i64,
        event: Option<i64>,
        detail: &serde_json::Value,
    ) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE audit_log SET status_code=?2,target=?3,detail=?4 WHERE id=?1",
            params![
                row,
                code,
                event.map(|id| id.to_string()),
                detail.to_string()
            ],
        )?;
        Ok(())
    }

    pub fn mcp_receipt_secret(&self, candidate: &str) -> rusqlite::Result<String> {
        let tx = self.conn().unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO app_settings(key,value,updated_ts) VALUES('mcp.receipt_secret',?,?)",
            params![candidate, chrono_now()],
        )?;
        let secret = tx.query_row(
            "SELECT value FROM app_settings WHERE key='mcp.receipt_secret'",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(secret)
    }
}
