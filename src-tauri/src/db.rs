use rusqlite::{Connection, Result};
use std::fs;
use std::path::PathBuf;

/// Initializes the SQLite database file in the app data directory.
/// Table creation and schema migrations are handled by refinery (via
/// `adapters::database::migration::run`).
pub fn init_db(app_data_dir: PathBuf) -> Result<Connection> {
    if !app_data_dir.exists() {
        fs::create_dir_all(&app_data_dir).expect("Failed to create app data directory");
    }

    let db_path = app_data_dir.join("demeteo.db");
    let conn = Connection::open(db_path)?;

    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;"
    )?;

    Ok(conn)
}
