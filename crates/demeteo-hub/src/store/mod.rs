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
//! authentication time by a globally `UNIQUE` credential — `tokens.token_hash`
//! or `passkeys.credential_id` — may omit the [`UserId`], because it is the
//! lookup that *establishes* the tenant; nothing upstream knows it yet. Such a
//! function selects by that unique column alone and returns the owning
//! [`UserId`] for every later call to take. Nothing else is exempt: once the
//! tenant is known, or for any column that is not one of those two, the
//! [`UserId`]-first rule holds.
//!
//! Secrets arrive already protected: [`tokens::TokenHash`] is a SHA-256
//! digest, [`attachments::WrappedKey`] a Transit-wrapped data key. Nothing here
//! accepts a plaintext token or key (`docs/HUB.md` §9).

pub mod attachments;
pub mod instances;
pub mod pool;
pub mod tokens;

use uuid::Uuid;

pub use pool::{connect, migrate, Db};

/// The field is private so nothing outside the store can name a tenant from
/// an arbitrary UUID, such as a path parameter: an existing tenant's id comes
/// only from [`pool::create_user`] or a lookup that establishes the tenant.
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
}
