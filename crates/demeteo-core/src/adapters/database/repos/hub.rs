//! SQL for `hub_install`, `hub_pairing` and `hub_pinned_passkeys` (V64,
//! with the passkeys' `sign_count` from V66). This file is the only `INSERT`
//! target for those tables, and what it is given is [`HubPins`] or an endorsed
//! passkey's id and public key, none of which has a field for a token or a
//! private key — so `grep -rn 'INSERT'` here is the whole check that no Hub
//! secret reaches SQLite.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rusqlite::{params, OptionalExtension};

use crate::domain::hub_pairing::{HubPins, PinnedPasskey};
use crate::domain::hub_request::passkeys::{
    passkey_origin, HubTrustedPasskey, TrustWrite, TrustedPasskey,
};
use crate::error::AppError;
use crate::ports::hub::{HubPasskeyReader, HubPasskeyTrust, HubPinRepository};
use crate::shared::time::now_secs;

use super::super::{DbError, SqliteAdapter};

const PAIRING_COLUMNS: &str =
    "hub_origin, instance_id, request_signing_key, rp_id, scopes, device_jkt, paired_at";

const PASSKEY_COLUMNS: &str = "credential_id, public_key";

const TRUSTED_COLUMNS: &str = "credential_id, public_key, sign_count";

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

/// A count outside `u32` is an error rather than a clamp: a clamped count
/// is one the row does not hold, so no compare-and-set could ever match it
/// and every assertion would be refused as though the row were gone.
fn row_to_trusted(row: &rusqlite::Row) -> rusqlite::Result<TrustedPasskey> {
    let sign_count: i64 = row.get(2)?;
    Ok(TrustedPasskey {
        credential_id: row.get(0)?,
        public_key: row.get(1)?,
        sign_count: u32::try_from(sign_count)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, sign_count))?,
    })
}

fn row_to_listed(row: &rusqlite::Row) -> rusqlite::Result<HubTrustedPasskey> {
    let sign_count: i64 = row.get(1)?;
    let pinned_at: i64 = row.get(2)?;
    let paired_at: i64 = row.get(3)?;
    Ok(HubTrustedPasskey {
        credential_id: row.get(0)?,
        sign_count: u32::try_from(sign_count)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, sign_count))?,
        pinned_at,
        origin: passkey_origin(pinned_at, paired_at),
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

fn cas_count(
    conn: &rusqlite::Connection,
    credential_id: &str,
    expected: u32,
    new: u32,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "UPDATE hub_pinned_passkeys SET sign_count = ?1
         WHERE credential_id = ?2 AND sign_count = ?3",
        params![new, credential_id, expected],
    )?;
    Ok(changed == 1)
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

impl HubPasskeyReader for SqliteAdapter {
    fn listed(&self) -> Result<Vec<HubTrustedPasskey>, AppError> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT p.credential_id, p.sign_count, p.pinned_at, h.paired_at
                 FROM hub_pinned_passkeys p JOIN hub_pairing h ON h.id = 1
                 ORDER BY p.rowid",
            )
            .map_err(DbError::Sqlite)?;
        let listed = stmt
            .query_map([], row_to_listed)
            .map_err(DbError::Sqlite)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(DbError::Sqlite)?;
        Ok(listed)
    }
}

impl HubPasskeyTrust for SqliteAdapter {
    fn trusted(&self) -> Result<Vec<TrustedPasskey>, AppError> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TRUSTED_COLUMNS} FROM hub_pinned_passkeys ORDER BY rowid"
            ))
            .map_err(DbError::Sqlite)?;
        let trusted = stmt
            .query_map([], row_to_trusted)
            .map_err(DbError::Sqlite)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(DbError::Sqlite)?;
        Ok(trusted)
    }

    fn advance_count(
        &self,
        credential_id: &str,
        expected: u32,
        new: u32,
    ) -> Result<bool, AppError> {
        let conn = self.conn.lock()?;
        Ok(cas_count(&conn, credential_id, expected, new)?)
    }

    fn endorse(
        &self,
        signer: &str,
        expected: u32,
        new: u32,
        credential_id: &str,
        public_key: &str,
        pinned_at: i64,
    ) -> Result<TrustWrite, AppError> {
        let mut conn = self.conn.lock()?;
        let tx = conn.transaction().map_err(DbError::Sqlite)?;
        if !cas_count(&tx, signer, expected, new)? {
            return Ok(TrustWrite::SignerChanged);
        }
        // `DO NOTHING` absorbs only the primary-key conflict; `OR REPLACE`
        // would swap a trusted key for one only its endorser vouched for.
        let inserted = tx
            .execute(
                "INSERT INTO hub_pinned_passkeys (credential_id, public_key, pinned_at, sign_count)
                 VALUES (?1, ?2, ?3, 0)
                 ON CONFLICT(credential_id) DO NOTHING",
                params![credential_id, public_key, pinned_at],
            )
            .map_err(DbError::Sqlite)?;
        if inserted == 0 {
            return Ok(TrustWrite::AlreadyPinned);
        }
        tx.commit().map_err(DbError::Sqlite)?;
        Ok(TrustWrite::Applied)
    }

    fn revoke(
        &self,
        signer: &str,
        expected: u32,
        new: u32,
        target: &str,
    ) -> Result<TrustWrite, AppError> {
        let mut conn = self.conn.lock()?;
        let tx = conn.transaction().map_err(DbError::Sqlite)?;
        if !cas_count(&tx, signer, expected, new)? {
            return Ok(TrustWrite::SignerChanged);
        }
        let deleted = tx
            .execute(
                "DELETE FROM hub_pinned_passkeys
                 WHERE credential_id = ?1
                   AND EXISTS (SELECT 1 FROM hub_pinned_passkeys WHERE credential_id = ?2)
                   AND (SELECT COUNT(*) FROM hub_pinned_passkeys) > 1",
                params![target, signer],
            )
            .map_err(DbError::Sqlite)?;
        if deleted == 1 {
            tx.commit().map_err(DbError::Sqlite)?;
            return Ok(TrustWrite::Applied);
        }
        let target_pinned: bool = tx
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM hub_pinned_passkeys WHERE credential_id = ?1)",
                params![target],
                |r| r.get(0),
            )
            .map_err(DbError::Sqlite)?;
        Ok(if target_pinned {
            TrustWrite::LastCredential
        } else {
            TrustWrite::TargetGone
        })
    }
}
