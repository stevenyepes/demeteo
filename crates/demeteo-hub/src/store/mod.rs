//! Tenant-scoped Postgres access, and the only module that imports `sqlx`.
//!
//! Every function that reads or writes a table other than `users` takes a
//! [`UserId`] as its first parameter and carries `user_id = $n` in its SQL, so
//! a caller cannot ask a question that is not about one tenant. Queries use
//! the runtime `query`, `query_as` and `query_scalar` APIs only: the
//! compile-time `query!` family needs `DATABASE_URL` at build time, which the
//! Docker build and CI do not have.
//!
//! One exception is permitted, and it is a decision, not a gap: a lookup at
//! authentication time by a globally `UNIQUE` credential — `tokens.token_hash`,
//! `passkeys.credential_id` or `sessions.token_hash` — may omit the
//! [`UserId`], because it is the lookup that *establishes* the tenant; nothing
//! upstream knows it yet. Such a function finds its row by that unique column
//! alone and returns the owning [`UserId`] for every later call to take.
//! [`sessions::find_session`] goes on to read the status of the passkey its
//! row names, joined on that row's own `user_id`: a second table, and still
//! the one tenant the first established. Nothing else is exempt: once the
//! tenant is known, or for any column that is not one of those three, the
//! [`UserId`]-first rule holds.
//!
//! A tenant that does not exist yet has no [`UserId`] to pass.
//! [`pool::register_user`] takes a [`pool::ReservedUser`] in that position
//! and writes only rows of the tenant it creates in the same transaction.
//!
//! Secrets arrive already protected: [`tokens::TokenHash`] and
//! [`sessions::SessionHash`] are SHA-256 digests, [`attachments::WrappedKey`]
//! a Transit-wrapped data key. Nothing here accepts a plaintext token, session
//! secret or key (`docs/HUB.md` §9).

pub mod attachments;
pub mod instances;
pub mod passkey_records;
pub mod passkeys;
pub mod pool;
pub mod sessions;
pub mod tokens;

use crate::auth::policy::Status;
use uuid::Uuid;

pub use pool::{connect, migrate, Db};

/// The field is private so nothing outside the store can name a tenant from
/// an arbitrary UUID, such as a path parameter: an existing tenant's id comes
/// only from [`pool::create_user`], [`pool::register_user`] or a lookup that
/// establishes the tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct UserId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct InstanceId(pub Uuid);

impl UserId {
    pub(crate) fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    /// The WebAuthn user handle of this tenant, for a ceremony that adds a
    /// passkey to it. One-way: a handle an authenticator returns is compared
    /// with this and never turned into a [`UserId`].
    pub fn handle(self) -> Uuid {
        self.0
    }
}

impl InstanceId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("{bytes} bytes exceed the BIGINT size column")]
    TooLarge { bytes: usize },
    #[error("a passkey with this credential id already exists")]
    CredentialExists,
    #[error("unknown passkey status {0:?}")]
    UnknownStatus(String),
    #[error("stored sign count {0} is not a 32-bit counter")]
    SignCountOutOfRange(i64),
    #[error("expiry {0} exceeds the BIGINT column")]
    ExpiryTooLarge(u64),
    #[error("stored expiry {0} is not a Unix time")]
    ExpiryOutOfRange(i64),
    #[error("the record names no passkey of this tenant that can become {}", .0.as_str())]
    RecordNamesNoPasskey(Status),
    #[error("the record's assertion gives no signature counter of a passkey of this tenant")]
    AssertionGivesNoCounter,
}
