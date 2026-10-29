use super::passkey_records::{insert_endorsement, insert_revocation};
use super::sessions::delete_sessions_of_credential;
use super::{Db, StoreError, UserId};
use crate::auth::assertion;
use crate::auth::chain::{EndorsementRecord, RevocationRecord, StoredKey};
use crate::auth::challenge::base64url_encode;
use crate::auth::policy::Status;
use demeteo_hub_protocol::WebAuthnAssertion;
use sqlx::{FromRow, PgConnection, PgExecutor, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

/// The name Postgres derives for the `UNIQUE` on `credential_id` in
/// `0001_init.sql`. Matching it rather than any unique violation keeps a
/// collision on the row's own `id` from being reported as a duplicate passkey.
const CREDENTIAL_ID_UNIQUE: &str = "passkeys_credential_id_key";

#[derive(Debug, Clone)]
pub struct NewPasskey {
    pub credential_id: Vec<u8>,
    /// The COSE_Key bytes of [`crate::auth::cose`].
    pub public_key: Vec<u8>,
    pub label: String,
    pub sign_count: u32,
    /// The verifier's own serialised credential, as JSON text. The column is
    /// `JSONB`, so what [`Passkey::credential`] returns is the same JSON value
    /// and not the same bytes: Postgres reorders keys and drops whitespace.
    pub credential: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passkey {
    pub user_id: UserId,
    pub credential_id: Vec<u8>,
    pub public_key: Vec<u8>,
    pub label: String,
    pub sign_count: u32,
    pub status: Status,
    pub credential: String,
    pub created_at: OffsetDateTime,
}

#[derive(FromRow)]
struct PasskeyRow {
    user_id: UserId,
    credential_id: Vec<u8>,
    public_key: Vec<u8>,
    label: String,
    sign_count: i64,
    status: String,
    credential: String,
    created_at: OffsetDateTime,
}

impl TryFrom<PasskeyRow> for Passkey {
    type Error = StoreError;

    fn try_from(row: PasskeyRow) -> Result<Self, StoreError> {
        let Some(status) = Status::parse(&row.status) else {
            return Err(StoreError::UnknownStatus(row.status));
        };
        let sign_count = u32::try_from(row.sign_count)
            .map_err(|_| StoreError::SignCountOutOfRange(row.sign_count))?;
        Ok(Self {
            user_id: row.user_id,
            credential_id: row.credential_id,
            public_key: row.public_key,
            label: row.label,
            sign_count,
            status,
            credential: row.credential,
            created_at: row.created_at,
        })
    }
}

fn insert_error(err: sqlx::Error) -> StoreError {
    match err.as_database_error() {
        Some(db) if db.constraint() == Some(CREDENTIAL_ID_UNIQUE) => StoreError::CredentialExists,
        _ => StoreError::Database(err),
    }
}

/// `status` is a parameter, not a field with a default, and the column has no
/// `DEFAULT` either: the trust a passkey is stored with is always the one a
/// caller decided, so a caller that decided nothing does not compile. A
/// credential id any tenant already holds is [`StoreError::CredentialExists`].
///
/// Takes an executor rather than the pool so the first passkey can be written
/// inside the transaction that creates its user.
pub async fn insert_passkey(
    user: UserId,
    db: impl PgExecutor<'_>,
    new: &NewPasskey,
    status: Status,
) -> Result<Passkey, StoreError> {
    sqlx::query_as::<_, PasskeyRow>(
        "INSERT INTO passkeys \
           (id, user_id, credential_id, public_key, label, sign_count, status, credential) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb) \
         RETURNING user_id, credential_id, public_key, label, sign_count, status, \
           credential::text AS credential, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(&new.credential_id)
    .bind(&new.public_key)
    .bind(&new.label)
    .bind(i64::from(new.sign_count))
    .bind(status.as_str())
    .bind(&new.credential)
    .fetch_one(db)
    .await
    .map_err(insert_error)?
    .try_into()
}

/// Every passkey the tenant has ever had, revoked ones included: rows are
/// never deleted, and the trust decisions need the whole set.
///
/// Takes an executor rather than the pool so a pairing can read them inside
/// the transaction that writes its instance.
pub async fn list_passkeys(
    user: UserId,
    db: impl PgExecutor<'_>,
) -> Result<Vec<Passkey>, StoreError> {
    sqlx::query_as::<_, PasskeyRow>(
        "SELECT user_id, credential_id, public_key, label, sign_count, status, \
           credential::text AS credential, created_at \
         FROM passkeys WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(Passkey::try_from)
    .collect()
}

/// The lookup that establishes the tenant from the credential a sign-in
/// presents, which is why it takes no [`UserId`] (see the
/// [module root](super)). It returns the passkey whatever its status; whether
/// that status may sign in is not decided here.
pub async fn find_passkey(db: &Db, credential_id: &[u8]) -> Result<Option<Passkey>, StoreError> {
    sqlx::query_as::<_, PasskeyRow>(
        "SELECT user_id, credential_id, public_key, label, sign_count, status, \
           credential::text AS credential, created_at \
         FROM passkeys WHERE credential_id = $1",
    )
    .bind(credential_id)
    .fetch_optional(db)
    .await?
    .map(Passkey::try_from)
    .transpose()
}

/// Returns whether a passkey was updated. The count is stored as reported,
/// lower or unchanged included: synced passkeys commonly report zero.
///
/// Takes an executor rather than the pool so a trust change can store its
/// assertion's count inside its own transaction.
pub async fn update_sign_count(
    user: UserId,
    db: impl PgExecutor<'_>,
    credential_id: &[u8],
    sign_count: u32,
) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE passkeys SET sign_count = $3 WHERE user_id = $1 AND credential_id = $2",
    )
    .bind(user)
    .bind(credential_id)
    .bind(i64::from(sign_count))
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Why [`endorse_passkey`], [`revoke_passkey`] or [`add_passkey`] wrote
/// nothing.
#[derive(Debug, thiserror::Error)]
pub enum TrustChangeError<E> {
    /// The decision's own error, returned as it was given.
    #[error("{0}")]
    Refused(E),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// What [`endorse_passkey`] or [`revoke_passkey`] committed. `passkey` is the
/// row whose status changed, as the transaction's own `UPDATE` returned it: a
/// caller answers from it, because a read after the commit can fail for a
/// change that is already durable and cannot be asked for twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustChange<R> {
    pub passkey: Passkey,
    pub record: R,
}

#[derive(FromRow)]
struct LockedRow {
    credential_id: Vec<u8>,
    public_key: Vec<u8>,
    status: String,
}

/// One tenant's passkeys, row-locked until the transaction that read them
/// ends. `keys[i]` is the row whose stored credential id is `raw_ids[i]`.
struct Locked {
    raw_ids: Vec<Vec<u8>>,
    keys: Vec<StoredKey>,
}

impl Locked {
    fn from_rows(rows: Vec<LockedRow>) -> Result<Self, StoreError> {
        let mut locked = Self {
            raw_ids: Vec::with_capacity(rows.len()),
            keys: Vec::with_capacity(rows.len()),
        };
        for row in rows {
            let Some(status) = Status::parse(&row.status) else {
                return Err(StoreError::UnknownStatus(row.status));
            };
            locked.keys.push(StoredKey {
                credential_id: base64url_encode(&row.credential_id),
                public_key: row.public_key,
                status,
            });
            locked.raw_ids.push(row.credential_id);
        }
        Ok(locked)
    }

    /// The stored bytes of the credential a record names, looked up among the
    /// locked rows and never decoded from the record: a record naming a
    /// passkey the decision was not shown has no row here to change.
    fn raw_id(&self, credential_id: &str) -> Option<&[u8]> {
        self.keys
            .iter()
            .position(|key| key.credential_id == credential_id)
            .and_then(|at| self.raw_ids.get(at))
            .map(Vec::as_slice)
    }

    /// [`raw_id`](Self::raw_id) of the passkey a record would make `becoming`.
    fn changing(&self, credential_id: &str, becoming: Status) -> Result<&[u8], StoreError> {
        self.raw_id(credential_id)
            .ok_or(StoreError::RecordNamesNoPasskey(becoming))
    }
}

/// `ORDER BY id` is what keeps two of these on one tenant from deadlocking:
/// both take the row locks in the same order, so the second waits on the
/// first row instead of each holding a row the other wants. Under READ
/// COMMITTED the one that waited then reads the rows as the first committed
/// them, which is the whole point of locking before deciding.
///
/// That holds for the rows this statement found, and for nothing else: a row
/// the first transaction *inserted* is not among them, because the statement's
/// snapshot predates it. A decision that depends on how many rows there are
/// reads them again, as [`add_passkey`] does.
async fn lock_passkeys(user: UserId, tx: &mut PgConnection) -> Result<Locked, StoreError> {
    let rows = sqlx::query_as::<_, LockedRow>(
        "SELECT credential_id, public_key, status FROM passkeys \
         WHERE user_id = $1 ORDER BY id FOR UPDATE",
    )
    .bind(user)
    .fetch_all(tx)
    .await?;
    Locked::from_rows(rows)
}

/// Begins a transaction, locks the tenant's passkeys and asks `decide` about
/// them. On a refusal the transaction is rolled back before this returns.
async fn decide_locked<R, E>(
    user: UserId,
    db: &Db,
    decide: impl FnOnce(&[StoredKey]) -> Result<R, E>,
) -> Result<(Transaction<'static, Postgres>, Locked, R), TrustChangeError<E>> {
    let mut tx = db.begin().await.map_err(StoreError::from)?;
    let locked = lock_passkeys(user, &mut tx).await?;
    match decide(&locked.keys) {
        Ok(record) => Ok((tx, locked, record)),
        Err(refusal) => {
            // A ROLLBACK that fails means the connection is gone, and the
            // server aborts the transaction with it: the refusal is still
            // the answer.
            let _ = tx.rollback().await;
            Err(TrustChangeError::Refused(refusal))
        }
    }
}

/// Returns the row as this statement left it, earlier writes of the same
/// transaction included.
async fn set_status(
    user: UserId,
    tx: &mut PgConnection,
    credential_id: &[u8],
    from: &[Status],
    to: Status,
) -> Result<Passkey, StoreError> {
    let from: Vec<&str> = from.iter().map(|status| status.as_str()).collect();
    sqlx::query_as::<_, PasskeyRow>(
        "UPDATE passkeys SET status = $3 \
         WHERE user_id = $1 AND credential_id = $2 AND status = ANY($4) \
         RETURNING user_id, credential_id, public_key, label, sign_count, status, \
           credential::text AS credential, created_at",
    )
    .bind(user)
    .bind(credential_id)
    .bind(to.as_str())
    .bind(from)
    .fetch_optional(tx)
    .await?
    .ok_or(StoreError::RecordNamesNoPasskey(to))?
    .try_into()
}

/// Stores the counter a record's assertion reported on the passkey that made
/// it, as a sign-in does through [`update_sign_count`]: the next sign-in of
/// that passkey is judged against the column, and a count left stale there
/// lets a clone's lower one pass as an advance.
///
/// A record [`chain`](crate::auth::chain) made always has both the passkey and
/// the counter. One that has neither was not verified against this tenant's
/// keys, and the transaction ends there.
async fn store_asserted_count(
    user: UserId,
    tx: &mut PgConnection,
    locked: &Locked,
    assertion: &WebAuthnAssertion,
) -> Result<(), StoreError> {
    let asserting = locked.raw_id(&assertion.credential_id);
    let (Some(asserting), Some(count)) = (asserting, assertion::sign_count(assertion)) else {
        return Err(StoreError::AssertionGivesNoCounter);
    };
    update_sign_count(user, tx, asserting, count).await?;
    Ok(())
}

/// Makes a pending passkey trusted and stores the endorsement that says why,
/// in one transaction, or does neither. This is the only code that writes
/// `trusted` to an existing row.
///
/// `decide` is handed every passkey of the tenant, locked, and is where the
/// caller runs [`chain::endorse`](crate::auth::chain::endorse): nothing here
/// judges an assertion or a status. It is synchronous because it runs while
/// the locks are held. The passkey that becomes trusted is the one the
/// returned record names, and only if its row is still `pending`; a record
/// naming anything else is [`StoreError::RecordNamesNoPasskey`] and writes
/// nothing.
///
/// The endorsing passkey's `sign_count` becomes the counter its assertion
/// reported, in the same transaction.
pub async fn endorse_passkey<E>(
    user: UserId,
    db: &Db,
    decide: impl FnOnce(&[StoredKey]) -> Result<EndorsementRecord, E>,
) -> Result<TrustChange<EndorsementRecord>, TrustChangeError<E>> {
    let (mut tx, locked, record) = decide_locked(user, db, decide).await?;
    let endorsed = locked.changing(&record.body.new_credential_id, Status::Trusted)?;
    insert_endorsement(user, &mut *tx, &record).await?;
    store_asserted_count(user, &mut tx, &locked, &record.body.assertion).await?;
    let passkey = set_status(user, &mut tx, endorsed, &[Status::Pending], Status::Trusted).await?;
    tx.commit().await.map_err(StoreError::from)?;
    Ok(TrustChange { passkey, record })
}

/// Revokes a passkey, stores the revocation that says why and ends every
/// session that passkey signed in, in one transaction, or does none of it.
///
/// `decide` is as in [`endorse_passkey`], for
/// [`chain::revoke`](crate::auth::chain::revoke). Two revocations of one
/// tenant's keys cannot both be decided on the same statuses: the second waits
/// for the first's locks and is shown what it committed, so two trusted keys
/// revoking each other leave one.
///
/// The revoking passkey's `sign_count` is stored as in [`endorse_passkey`].
pub async fn revoke_passkey<E>(
    user: UserId,
    db: &Db,
    decide: impl FnOnce(&[StoredKey]) -> Result<RevocationRecord, E>,
) -> Result<TrustChange<RevocationRecord>, TrustChangeError<E>> {
    let (mut tx, locked, record) = decide_locked(user, db, decide).await?;
    let revoked = locked.changing(&record.body.credential_id, Status::Revoked)?;
    insert_revocation(user, &mut *tx, &record).await?;
    store_asserted_count(user, &mut tx, &locked, &record.body.assertion).await?;
    let passkey = set_status(
        user,
        &mut tx,
        revoked,
        &[Status::Pending, Status::Trusted],
        Status::Revoked,
    )
    .await?;
    delete_sessions_of_credential(user, &mut *tx, revoked).await?;
    tx.commit().await.map_err(StoreError::from)?;
    Ok(TrustChange { passkey, record })
}

async fn statuses_of(user: UserId, tx: &mut PgConnection) -> Result<Vec<Status>, StoreError> {
    sqlx::query_scalar::<_, String>("SELECT status FROM passkeys WHERE user_id = $1")
        .bind(user)
        .fetch_all(tx)
        .await?
        .into_iter()
        .map(|status| Status::parse(&status).ok_or(StoreError::UnknownStatus(status)))
        .collect()
}

/// Stores a further passkey of a tenant that already has one, with the status
/// `decide` gives it, or stores nothing. `decide` is handed the status of
/// every passkey the tenant has and is where the caller runs the policy:
/// nothing here counts or judges them. It is synchronous for the reason
/// [`endorse_passkey`] gives.
///
/// The statuses are read by a second statement and not taken from the rows
/// the lock statement returned. Two additions of one tenant queue on its row
/// locks, and the one that waited is handed the rows it locked as they were
/// when its own statement began — without the row the first inserted (see
/// [`lock_passkeys`]). Deciding on those, two additions one short of a limit
/// would each count the same rows and both be stored. The statement issued
/// once the locks are held sees everything committed before it.
///
/// What they queue on is a row both can see, and there always is one: a
/// session is opened only by a passkey of its tenant, and no row is deleted.
/// A tenant with no passkey has nothing to lock, so this is not how a first
/// passkey is written; [`register_user`](super::pool::register_user) is.
pub async fn add_passkey<E>(
    user: UserId,
    db: &Db,
    new: &NewPasskey,
    decide: impl FnOnce(&[Status]) -> Result<Status, E>,
) -> Result<Passkey, TrustChangeError<E>> {
    let mut tx = db.begin().await.map_err(StoreError::from)?;
    lock_passkeys(user, &mut tx).await?;
    let existing = statuses_of(user, &mut tx).await?;
    let status = match decide(&existing) {
        Ok(status) => status,
        Err(refusal) => {
            // As in `decide_locked`: a failed ROLLBACK does not change the answer.
            let _ = tx.rollback().await;
            return Err(TrustChangeError::Refused(refusal));
        }
    };
    let added = insert_passkey(user, &mut *tx, new, status).await?;
    tx.commit().await.map_err(StoreError::from)?;
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(status: &str, sign_count: i64) -> PasskeyRow {
        PasskeyRow {
            user_id: UserId::generate(),
            credential_id: vec![1, 2, 3],
            public_key: vec![4, 5, 6],
            label: "laptop".to_owned(),
            sign_count,
            status: status.to_owned(),
            credential: "{}".to_owned(),
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn a_row_carries_the_status_its_column_names() {
        for status in [Status::Pending, Status::Trusted, Status::Revoked] {
            let passkey = Passkey::try_from(row(status.as_str(), 7)).unwrap();
            assert_eq!(passkey.status, status);
            assert_eq!(passkey.sign_count, 7);
        }
    }

    #[test]
    fn a_row_with_an_unknown_status_is_refused() {
        for unknown in ["", "Trusted", "trusted ", "active"] {
            let err = Passkey::try_from(row(unknown, 0)).unwrap_err();
            assert!(
                matches!(&err, StoreError::UnknownStatus(s) if s == unknown),
                "{unknown:?}: {err}"
            );
        }
    }

    #[test]
    fn a_sign_count_no_authenticator_can_report_is_refused() {
        for count in [-1, i64::from(u32::MAX) + 1] {
            let err = Passkey::try_from(row("trusted", count)).unwrap_err();
            assert!(
                matches!(err, StoreError::SignCountOutOfRange(c) if c == count),
                "{count}: {err}"
            );
        }
        let max = Passkey::try_from(row("trusted", i64::from(u32::MAX))).unwrap();
        assert_eq!(max.sign_count, u32::MAX);
    }
}
