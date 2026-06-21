use rusqlite::Connection;
use std::sync::{Mutex, MutexGuard};

/// Thin wrapper around `Mutex<Connection>` that provides a clean
/// `lock()` method returning a structured error.
pub struct SqliteConnection {
    conn: Mutex<Connection>,
}

impl SqliteConnection {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
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
