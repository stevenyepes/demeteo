//! SQL for `hub_install`, `hub_pairing` and `hub_pinned_passkeys` (V64).
//! This file is the only `INSERT` target for those tables, and what it is
//! given is [`HubPins`], which has no field for a token or a private key —
//! so `grep -rn 'INSERT'` here is the whole check that no Hub secret reaches
//! SQLite.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rusqlite::{params, OptionalExtension};

use crate::domain::hub_pairing::{HubPins, PinnedPasskey};
use crate::error::AppError;
use crate::ports::hub::HubPinRepository;
use crate::shared::time::now_secs;

use super::super::{DbError, SqliteAdapter};

const PAIRING_COLUMNS: &str =
    "hub_origin, instance_id, request_signing_key, rp_id, scopes, device_jkt, paired_at";

const PASSKEY_COLUMNS: &str = "credential_id, public_key";

fn row_to_pins(row: &rusqlite::Row) -> rusqlite::Result<HubPins> {
    let scopes: String = row.get(4)?;
    Ok(HubPins {
        hub_origin: row.get(0)?,
        instance_id: row.get(1)?,
        request_signing_key: row.get(2)?,
        rp_id: row.get(3)?,
        scopes: scopes.split_whitespace().map(str::to_owned).collect(),
        device_jkt: row.get(5)?,
        paired_at: row.get(6)?,
        passkeys: Vec::new(),
    })
}

fn row_to_passkey(row: &rusqlite::Row) -> rusqlite::Result<PinnedPasskey> {
    Ok(PinnedPasskey {
        credential_id: row.get(0)?,
        public_key: row.get(1)?,
    })
}

/// Public, but never a timestamp or a counter: it names the keyring
/// accounts, and the dev and installed builds must not collide on them.
fn new_install_id() -> Result<String, AppError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| AppError::internal(format!("no OS randomness for install_id: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn unix_now() -> i64 {
    i64::try_from(now_secs()).unwrap_or(i64::MAX)
}

impl HubPinRepository for SqliteAdapter {
    fn install_id(&self) -> Result<String, AppError> {
        let conn = self.conn.lock()?;
        let existing: Option<String> = conn
            .query_row("SELECT install_id FROM hub_install WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(DbError::Sqlite)?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = new_install_id()?;
        conn.execute(
            "INSERT INTO hub_install (id, install_id, created_at) VALUES (1, ?1, ?2)",
            params![id, unix_now()],
        )
        .map_err(DbError::Sqlite)?;
        Ok(id)
    }

    fn load(&self) -> Result<Option<HubPins>, AppError> {
        let conn = self.conn.lock()?;
        let pins = conn
            .query_row(
                &format!("SELECT {PAIRING_COLUMNS} FROM hub_pairing WHERE id = 1"),
                [],
                row_to_pins,
            )
            .optional()
            .map_err(DbError::Sqlite)?;
        let Some(mut pins) = pins else {
            return Ok(None);
        };
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PASSKEY_COLUMNS} FROM hub_pinned_passkeys ORDER BY rowid"
            ))
            .map_err(DbError::Sqlite)?;
        pins.passkeys = stmt
            .query_map([], row_to_passkey)
            .map_err(DbError::Sqlite)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(DbError::Sqlite)?;
        Ok(Some(pins))
    }

    fn save(&self, pins: &HubPins) -> Result<(), AppError> {
        let mut conn = self.conn.lock()?;
        let tx = conn.transaction().map_err(DbError::Sqlite)?;
        tx.execute("DELETE FROM hub_pinned_passkeys", [])
            .map_err(DbError::Sqlite)?;
        tx.execute(
            &format!(
                "INSERT OR REPLACE INTO hub_pairing (id, {PAIRING_COLUMNS})
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ),
            params![
                pins.hub_origin,
                pins.instance_id,
                pins.request_signing_key,
                pins.rp_id,
                pins.scopes.join(" "),
                pins.device_jkt,
                pins.paired_at,
            ],
        )
        .map_err(DbError::Sqlite)?;
        for passkey in &pins.passkeys {
            tx.execute(
                &format!(
                    "INSERT INTO hub_pinned_passkeys ({PASSKEY_COLUMNS}, pinned_at)
                     VALUES (?1, ?2, ?3)"
                ),
                params![passkey.credential_id, passkey.public_key, pins.paired_at],
            )
            .map_err(DbError::Sqlite)?;
        }
        tx.commit().map_err(DbError::Sqlite)?;
        Ok(())
    }

    fn clear(&self) -> Result<(), AppError> {
        let mut conn = self.conn.lock()?;
        let tx = conn.transaction().map_err(DbError::Sqlite)?;
        tx.execute("DELETE FROM hub_pinned_passkeys", [])
            .map_err(DbError::Sqlite)?;
        tx.execute("DELETE FROM hub_pairing", [])
            .map_err(DbError::Sqlite)?;
        tx.commit().map_err(DbError::Sqlite)?;
        Ok(())
    }
}
