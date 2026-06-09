use rusqlite::{Connection, Result};
use std::fs;
use std::path::PathBuf;

/// Initializes the SQLite database file in the app data directory and creates tables.
pub fn init_db(app_data_dir: PathBuf) -> Result<Connection> {
    if !app_data_dir.exists() {
        fs::create_dir_all(&app_data_dir).expect("Failed to create app data directory");
    }

    let db_path = app_data_dir.join("demeteo.db");
    let conn = Connection::open(db_path)?;

    conn.execute("PRAGMA foreign_keys = ON;", [])?;

    conn.execute_batch(
        "BEGIN;
        
        CREATE TABLE IF NOT EXISTS machines (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            host TEXT NOT NULL,
            port INTEGER NOT NULL,
            username TEXT NOT NULL,
            auth_type TEXT NOT NULL,
            key_path TEXT,
            agents TEXT,
            auto_approved_rules TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS agent_profiles (
            id TEXT PRIMARY KEY,
            machine_id TEXT NOT NULL,
            name TEXT NOT NULL,
            agent_type TEXT NOT NULL,
            command TEXT,
            work_dir TEXT,
            port INTEGER,
            ready_check TEXT,
            FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS chat_sessions (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            title TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(agent_id) REFERENCES agent_profiles(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            sender TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS session_history (
            id TEXT PRIMARY KEY,
            machine_id TEXT NOT NULL,
            session_type TEXT NOT NULL,
            title TEXT NOT NULL,
            content TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS thread_sessions (
            id TEXT PRIMARY KEY,
            machine_id TEXT NOT NULL,
            title TEXT NOT NULL,
            mode TEXT NOT NULL,
            branch TEXT,
            repo_path TEXT,
            sandbox_path TEXT,
            status TEXT NOT NULL,
            agent_kind TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS thread_working_memory (
            thread_id      TEXT NOT NULL,
            file_path      TEXT NOT NULL,
            line_count     INTEGER,
            size_bytes     INTEGER,
            modified_at    INTEGER,
            first_read_at  INTEGER NOT NULL,
            last_read_at   INTEGER NOT NULL,
            PRIMARY KEY (thread_id, file_path),
            FOREIGN KEY (thread_id) REFERENCES thread_sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_twm_thread_last_read
            ON thread_working_memory(thread_id, last_read_at DESC);

        COMMIT;"
    )?;

    Ok(conn)
}
