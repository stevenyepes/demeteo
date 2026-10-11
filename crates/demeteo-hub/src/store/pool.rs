use super::StoreError;
use super::UserId;
use sqlx::postgres::PgPoolOptions;

pub type Db = sqlx::PgPool;

pub async fn connect(database_url: &str) -> Result<Db, StoreError> {
    Ok(PgPoolOptions::new().connect(database_url).await?)
}

/// Embeds `migrations/` at compile time; `build.rs` re-runs when the directory
/// changes so a new file is picked up. Applied migrations are checksummed, so
/// later changes append a new file rather than edit `0001_init.sql`.
pub async fn migrate(db: &Db) -> Result<(), StoreError> {
    sqlx::migrate!("./migrations").run(db).await?;
    Ok(())
}

/// `users` is the one table with no `user_id` column, so this is the one
/// function that does not take a [`UserId`].
pub async fn create_user(db: &Db) -> Result<UserId, StoreError> {
    let id = UserId::generate();
    sqlx::query("INSERT INTO users (id) VALUES ($1)")
        .bind(id)
        .execute(db)
        .await?;
    Ok(id)
}
