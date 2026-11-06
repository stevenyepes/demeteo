use super::{StoreError, UserId};
use sqlx::{FromRow, PgExecutor};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct GitHostId(pub Uuid);

impl GitHostId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

/// The one kind a row can hold today; the column's `CHECK` agrees.
pub const GITHUB_APP: &str = "github_app";

/// A connected GitHub App. Nothing in it is secret: the key it signs with is
/// named by `transit_key` and held by Transit.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct GitHostRow {
    pub id: GitHostId,
    pub host: String,
    pub api_base: String,
    pub web_base: String,
    pub app_id: i64,
    pub app_slug: String,
    pub transit_key: String,
    pub account: Option<String>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGitHost {
    pub id: GitHostId,
    pub host: String,
    pub api_base: String,
    pub web_base: String,
    pub app_id: i64,
    pub app_slug: String,
    pub transit_key: String,
    pub account: Option<String>,
}

pub async fn insert_git_host(
    user: UserId,
    db: impl PgExecutor<'_>,
    host: &NewGitHost,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO git_hosts \
         (id, user_id, kind, host, api_base, web_base, app_id, app_slug, transit_key, account) \
         VALUES ($2, $1, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(user)
    .bind(host.id)
    .bind(GITHUB_APP)
    .bind(&host.host)
    .bind(&host.api_base)
    .bind(&host.web_base)
    .bind(host.app_id)
    .bind(&host.app_slug)
    .bind(&host.transit_key)
    .bind(&host.account)
    .execute(db)
    .await?;
    Ok(())
}

/// Oldest first, so a list reads in the order hosts were connected.
pub async fn git_hosts(
    user: UserId,
    db: impl PgExecutor<'_>,
) -> Result<Vec<GitHostRow>, StoreError> {
    Ok(sqlx::query_as::<_, GitHostRow>(
        "SELECT id, host, api_base, web_base, app_id, app_slug, transit_key, account, created_at \
         FROM git_hosts WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await?)
}

/// The tenant's Apps on `host`, compared as stored: callers pass the
/// lowercase spelling a start's repository carries.
pub async fn git_hosts_on(
    user: UserId,
    db: impl PgExecutor<'_>,
    host: &str,
) -> Result<Vec<GitHostRow>, StoreError> {
    Ok(sqlx::query_as::<_, GitHostRow>(
        "SELECT id, host, api_base, web_base, app_id, app_slug, transit_key, account, created_at \
         FROM git_hosts WHERE user_id = $1 AND host = $2 ORDER BY created_at, id",
    )
    .bind(user)
    .bind(host)
    .fetch_all(db)
    .await?)
}

/// Whether a row was removed. The App's Transit key stays: deleting a Transit
/// key needs `deletion_allowed` set on it first, which the Hub's policy does
/// not grant, and an orphaned key signs for nobody once no row names it.
pub async fn remove_git_host(
    user: UserId,
    db: impl PgExecutor<'_>,
    id: GitHostId,
) -> Result<bool, StoreError> {
    let removed = sqlx::query("DELETE FROM git_hosts WHERE user_id = $1 AND id = $2")
        .bind(user)
        .bind(id)
        .execute(db)
        .await?;
    Ok(removed.rows_affected() > 0)
}
