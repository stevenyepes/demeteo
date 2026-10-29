use super::passkeys::{insert_passkey, NewPasskey};
use super::sessions::{insert_session, NewSession, SessionHash};
use super::StoreError;
use super::UserId;
use crate::auth::policy::Status;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

pub type Db = sqlx::PgPool;

pub async fn connect(database_url: &str) -> Result<Db, StoreError> {
    Ok(PgPoolOptions::new().connect(database_url).await?)
}

/// A pool that parses `database_url` and dials nothing until a query asks for
/// a connection. It must be built inside a Tokio runtime: sqlx spawns the
/// pool's idle reaper on creation.
#[cfg(any(test, feature = "test-support"))]
pub fn connect_lazy(database_url: &str) -> Result<Db, StoreError> {
    Ok(PgPoolOptions::new().connect_lazy(database_url)?)
}

/// Embeds `migrations/` at compile time; `build.rs` re-runs when the directory
/// changes so a new file is picked up. Applied migrations are checksummed, so
/// later changes append a new file rather than edit `0001_init.sql`.
pub async fn migrate(db: &Db) -> Result<(), StoreError> {
    sqlx::migrate!("./migrations").run(db).await?;
    Ok(())
}

/// A tenant id chosen when a registration ceremony starts, before any row
/// exists: the authenticator stores it as the WebAuthn user handle, so it has
/// to be known before the credential that will prove the registration is.
///
/// It is not a [`UserId`] and cannot be used as one. Only
/// [`register_user`] turns it into a tenant, which is what keeps a started and
/// abandoned ceremony from leaving a `users` row behind. There is deliberately
/// no constructor from a UUID and no `Clone`: an id read back from a user
/// handle, a path or a body is a value the browser chose.
#[derive(Debug)]
pub struct ReservedUser(UserId);

impl ReservedUser {
    pub fn generate() -> Self {
        Self(UserId::generate())
    }

    /// What [`UserId::handle`] returns once the user exists.
    pub fn handle(&self) -> Uuid {
        self.0.handle()
    }
}

async fn insert_user(db: impl PgExecutor<'_>, id: UserId) -> Result<(), StoreError> {
    sqlx::query("INSERT INTO users (id) VALUES ($1)")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// `users` is the one table with no `user_id` column, so this and
/// [`register_user`] are the functions that create a tenant instead of taking
/// a [`UserId`].
pub async fn create_user(db: &Db) -> Result<UserId, StoreError> {
    let id = UserId::generate();
    insert_user(db, id).await?;
    Ok(id)
}

/// Every tenant, for a job that has to visit each in turn, such as the
/// attachment sweep. Reading `users` is how such a job learns them without a
/// statement over another table that names no tenant.
pub async fn tenants(db: &Db) -> Result<Vec<UserId>, StoreError> {
    Ok(sqlx::query_scalar::<_, UserId>("SELECT id FROM users")
        .fetch_all(db)
        .await?)
}

/// Creates the user, its first passkey and the session that passkey signed in
/// with, or none of them: a credential id another tenant already holds is
/// [`StoreError::CredentialExists`] and leaves no `users` row.
///
/// `status` is the caller's decision, as it is for [`insert_passkey`]. The
/// session takes its credential id from `passkey` rather than from a second
/// argument, so it cannot name a passkey other than the one stored with it.
pub async fn register_user(
    reserved: ReservedUser,
    db: &Db,
    passkey: &NewPasskey,
    status: Status,
    session: SessionHash,
    session_expires_at: OffsetDateTime,
) -> Result<UserId, StoreError> {
    let user = reserved.0;
    let session = NewSession {
        hash: session,
        credential_id: passkey.credential_id.clone(),
        expires_at: session_expires_at,
    };
    let mut tx = db.begin().await?;
    insert_user(&mut *tx, user).await?;
    insert_passkey(user, &mut *tx, passkey, status).await?;
    insert_session(user, &mut *tx, &session).await?;
    tx.commit().await?;
    Ok(user)
}
