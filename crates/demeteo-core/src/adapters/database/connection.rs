use rusqlite::Connection;
use std::sync::{Arc, Mutex, MutexGuard};

/// Thin wrapper around `Arc<Mutex<Connection>>` that provides a
/// clean `lock()` method returning a structured error. The `Arc`
/// keeps the connection cheaply cloneable so multiple sub-adapters
/// (e.g. `SqliteMergeExecutor` and the existing repos) can share a
/// single underlying handle without opening a second SQLite file.
#[derive(Clone)]
pub struct SqliteConnection {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteConnection {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    /// Acquire the mutex guard. Returns a `String` error on poison so
    /// callers can use `?` directly inside `Result<_, String>` methods.
    pub fn lock(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|_| "database lock poisoned".to_string())
    }
}
