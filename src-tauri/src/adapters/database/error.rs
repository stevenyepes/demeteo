use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database lock poisoned")]
    LockPoisoned,

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error("migration failed: {0}")]
    Migration(String),

    #[error("{0}")]
    Other(String),
}
