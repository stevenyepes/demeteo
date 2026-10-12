use super::{Db, StoreError, UserId};
use crate::auth::policy::Status;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

/// SHA-256 of a session secret. This is the only form of a session the store
/// accepts, so neither the cookie value nor the CSRF token derived from it has
/// a parameter to travel in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHash([u8; 32]);

impl SessionHash {
    pub fn digest(secret: &[u8]) -> Self {
        Self(Sha256::digest(secret).into())
    }
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub hash: SessionHash,
    /// The passkey that signed in, so revoking it can end its sessions.
    pub credential_id: Vec<u8>,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Session {
    pub user_id: UserId,
    pub credential_id: Vec<u8>,
}

/// Also drops the user's expired rows: a sign-in is the only write a session
/// ever causes, so it is where they are collected, in the one statement.
///
/// Takes an executor rather than the pool so the first sign-in can run inside
/// the transaction that creates the user.
pub async fn insert_session(
    user: UserId,
    db: impl PgExecutor<'_>,
    new: &NewSession,
) -> Result<(), StoreError> {
    sqlx::query(
        "WITH expired AS ( \
           DELETE FROM sessions WHERE user_id = $2 AND expires_at <= now() \
         ) \
         INSERT INTO sessions (id, user_id, token_hash, credential_id, expires_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(new.hash.0.as_slice())
    .bind(&new.credential_id)
    .bind(new.expires_at)
    .execute(db)
    .await?;
    Ok(())
}

/// The lookup that establishes the tenant from a session cookie, which is why
/// it takes no [`UserId`] (see the [module root](super)). An expired session
/// is not found, and neither is one whose passkey is not `trusted` as this
/// runs.
///
/// The status is read here, on every request, because nothing orders a
/// session's insert against its passkey's revocation: a sign-in checks the
/// status and writes the row as separate statements, and a revocation that
/// commits between the two has no row to delete yet. Such a row is stored and
/// authenticates nothing. Checking only where the session is written would
/// make "revoked means signed out" depend on that order.
///
/// The join carries the row's own `user_id` as well as its `credential_id`, so
/// a session is only ever paired with a passkey of the tenant it names.
pub async fn find_session(db: &Db, hash: &SessionHash) -> Result<Option<Session>, StoreError> {
    Ok(sqlx::query_as::<_, Session>(
        "SELECT s.user_id, s.credential_id FROM sessions s \
         JOIN passkeys p ON p.user_id = s.user_id AND p.credential_id = s.credential_id \
         WHERE s.token_hash = $1 AND s.expires_at > now() AND p.status = $2",
    )
    .bind(hash.0.as_slice())
    .bind(Status::Trusted.as_str())
    .fetch_optional(db)
    .await?)
}

/// Returns whether a session was deleted.
pub async fn delete_session(user: UserId, db: &Db, hash: &SessionHash) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token_hash = $2")
        .bind(user)
        .bind(hash.0.as_slice())
        .execute(db)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Returns how many sessions were deleted.
///
/// Takes an executor rather than the pool so a revocation can run it in the
/// transaction that changes the passkey's status, and one that rolls back ends
/// no session.
///
/// This is not what stops a revoked passkey's session from authenticating:
/// [`find_session`] reads the status itself, so a row this misses, written by
/// a sign-in after the revocation committed, is already inert. A revoked
/// passkey is never trusted again, so what the delete removes are rows that
/// could only wait for their expiry.
pub async fn delete_sessions_of_credential(
    user: UserId,
    db: impl PgExecutor<'_>,
    credential_id: &[u8],
) -> Result<u64, StoreError> {
    let done = sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND credential_id = $2")
        .bind(user)
        .bind(credential_id)
        .execute(db)
        .await?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_sha256_of_the_secret() {
        const SHA256_OF_ABC: [u8; 32] = [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ];
        assert_eq!(SessionHash::digest(b"abc").0, SHA256_OF_ABC);
    }
}
