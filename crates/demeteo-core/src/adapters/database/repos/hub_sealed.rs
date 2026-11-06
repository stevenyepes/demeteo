//! SQL for `hub_sealed_secrets` (V68). What it is given is a
//! [`SealedSecret`], which has no field for a plaintext.

use rusqlite::{params, OptionalExtension};

use crate::error::AppError;
use crate::ports::hub::{HubSealedSecretRepository, SealedSecret};

use super::super::{DbError, SqliteAdapter};

impl HubSealedSecretRepository for SqliteAdapter {
    fn load_sealed(&self, account: &str) -> Result<Option<SealedSecret>, AppError> {
        let conn = self.conn.lock()?;
        let sealed = conn
            .query_row(
                "SELECT nonce, ciphertext FROM hub_sealed_secrets WHERE account = ?1",
                params![account],
                |row| {
                    Ok(SealedSecret {
                        nonce: row.get(0)?,
                        ciphertext: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(DbError::Sqlite)?;
        Ok(sealed)
    }

    fn store_sealed(&self, account: &str, sealed: &SealedSecret) -> Result<(), AppError> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO hub_sealed_secrets (account, nonce, ciphertext) VALUES (?1, ?2, ?3)
             ON CONFLICT (account) DO UPDATE SET nonce = excluded.nonce,
                                                 ciphertext = excluded.ciphertext",
            params![account, sealed.nonce, sealed.ciphertext],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }

    fn delete_sealed(&self, account: &str) -> Result<(), AppError> {
        let conn = self.conn.lock()?;
        conn.execute(
            "DELETE FROM hub_sealed_secrets WHERE account = ?1",
            params![account],
        )
        .map_err(DbError::Sqlite)?;
        Ok(())
    }
}
