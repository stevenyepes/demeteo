//! SQL for `oauth_clients` and `oauth_grants` (V56). `insert_grant` takes the
//! already-hashed `token_hash` — hashing happens in the `mcp` adapter, never
//! here, so this file is the one `grep -rn 'INSERT'` target for the feature's
//! no-credential-material acceptance criterion.

use rusqlite::params;

use crate::domain::ids::{ClientId, GrantId};
use crate::domain::oauth::{GrantRecord, OAuthClient, Scope};
use crate::error::AppError;
use crate::ports::oauth::{OAuthClientRepository, OAuthGrantRepository};

use super::super::{DbError, SqliteAdapter};

const CLIENT_COLUMNS: &str = "id, client_name, redirect_uris, created_at";

// Deliberately excludes `token_hash`: `GrantRecord` mirrors the row minus the
// hash, since validation is only ever given a caller-supplied token to hash
// and compare, never the stored hash itself (see `domain::oauth`).
const GRANT_COLUMNS: &str = "id, client_id, scopes, resource, issued_at, expires_at, revoked_at";

fn encode_scopes(scopes: &[Scope]) -> String {
    scopes
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A scope this build cannot name is dropped rather than kept as an opaque
/// string — mirrors `decode_attachments`: the grant still enforces every
/// scope it does recognize, and a stale/foreign spelling degrades to "not
/// granted" rather than a parse failure.
fn decode_scopes(raw: &str) -> Vec<Scope> {
    raw.split_whitespace().filter_map(Scope::parse).collect()
}

fn encode_redirect_uris(uris: &[String]) -> Result<String, AppError> {
    Ok(serde_json::to_string(uris)?)
}

fn decode_redirect_uris(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn row_to_client(row: &rusqlite::Row) -> rusqlite::Result<OAuthClient> {
    let redirect_uris: String = row.get(2)?;
    Ok(OAuthClient {
        id: row.get(0)?,
        client_name: row.get(1)?,
        redirect_uris: decode_redirect_uris(&redirect_uris),
        created_at: row.get(3)?,
    })
}

/// `offset` is 0 when `GRANT_COLUMNS` is the whole projection
/// (`find_grant_by_token_hash`) and the client column count when it follows a
/// join (`list_active_grants`).
fn row_to_grant(row: &rusqlite::Row, offset: usize) -> rusqlite::Result<GrantRecord> {
    let scopes: String = row.get(offset + 2)?;
    Ok(GrantRecord {
        id: row.get(offset)?,
        client_id: row.get(offset + 1)?,
        scopes: decode_scopes(&scopes),
        resource: row.get(offset + 3)?,
        issued_at: row.get(offset + 4)?,
        expires_at: row.get(offset + 5)?,
        revoked_at: row.get(offset + 6)?,
    })
}

impl OAuthClientRepository for SqliteAdapter {
    fn register_client(&self, client: OAuthClient) -> Result<(), AppError> {
        let redirect_uris = encode_redirect_uris(&client.redirect_uris)?;
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO oauth_clients (id, client_name, redirect_uris, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                client.id,
                client.client_name,
                redirect_uris,
                client.created_at,
            ],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }

    fn register_client_bounded(
        &self,
        client: OAuthClient,
        max_clients: usize,
        prune_before_ms: i64,
    ) -> Result<bool, AppError> {
        let redirect_uris = encode_redirect_uris(&client.redirect_uris)?;
        let mut conn = self.conn.lock()?;
        let tx = conn.transaction().map_err(DbError::Sqlite)?;
        let count = |tx: &rusqlite::Transaction| -> Result<usize, DbError> {
            let n: i64 = tx
                .query_row("SELECT COUNT(*) FROM oauth_clients", [], |r| r.get(0))
                .map_err(DbError::Sqlite)?;
            Ok(usize::try_from(n).unwrap_or(usize::MAX))
        };
        if count(&tx)? >= max_clients {
            tx.execute(
                "DELETE FROM oauth_clients
                 WHERE created_at < ?1
                   AND id NOT IN (SELECT client_id FROM oauth_grants)",
                params![prune_before_ms],
            )
            .map_err(DbError::Sqlite)?;
            if count(&tx)? >= max_clients {
                return Ok(false);
            }
        }
        tx.execute(
            "INSERT INTO oauth_clients (id, client_name, redirect_uris, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                client.id,
                client.client_name,
                redirect_uris,
                client.created_at,
            ],
        )
        .map_err(DbError::Sqlite)?;
        tx.commit().map_err(DbError::Sqlite)?;
        Ok(true)
    }

    fn get_client(&self, id: &ClientId) -> Result<Option<OAuthClient>, AppError> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!("SELECT {CLIENT_COLUMNS} FROM oauth_clients WHERE id = ?1"),
            params![id],
            row_to_client,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(AppError::from(DbError::Sqlite(e))),
        })
    }
}

impl OAuthGrantRepository for SqliteAdapter {
    fn insert_grant(&self, grant: GrantRecord, token_hash: &str) -> Result<(), AppError> {
        let scopes = encode_scopes(&grant.scopes);
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO oauth_grants
                (id, client_id, scopes, resource, token_hash, issued_at, expires_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                grant.id,
                grant.client_id,
                scopes,
                grant.resource,
                token_hash,
                grant.issued_at,
                grant.expires_at,
                grant.revoked_at,
            ],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }

    fn find_grant_by_token_hash(&self, token_hash: &str) -> Result<Option<GrantRecord>, AppError> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!("SELECT {GRANT_COLUMNS} FROM oauth_grants WHERE token_hash = ?1"),
            params![token_hash],
            |row| row_to_grant(row, 0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(AppError::from(DbError::Sqlite(e))),
        })
    }

    fn list_active_grants(&self) -> Result<Vec<(OAuthClient, GrantRecord)>, AppError> {
        let conn = self.conn.lock()?;
        let sql = format!(
            "SELECT {client_cols}, {grant_cols}
               FROM oauth_grants g
               JOIN oauth_clients c ON c.id = g.client_id
              WHERE g.revoked_at IS NULL AND g.expires_at > ?1
              ORDER BY g.issued_at DESC",
            client_cols = CLIENT_COLUMNS
                .split(", ")
                .map(|c| format!("c.{c}"))
                .collect::<Vec<_>>()
                .join(", "),
            grant_cols = GRANT_COLUMNS
                .split(", ")
                .map(|c| format!("g.{c}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        let mut stmt = conn.prepare(&sql).map_err(DbError::Sqlite)?;
        let iter = stmt
            .query_map(params![crate::paths::now_ms()], |row| {
                let client = row_to_client(row)?;
                let grant = row_to_grant(row, CLIENT_COLUMNS.split(", ").count())?;
                Ok((client, grant))
            })
            .map_err(DbError::Sqlite)?;
        iter.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| AppError::from(DbError::Sqlite(e)))
    }

    fn revoke_grant(&self, id: &GrantId) -> Result<(), AppError> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE oauth_grants SET revoked_at = ?2 WHERE id = ?1",
            params![id, crate::paths::now_ms()],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/oauth.rs"]
mod tests;
