//! SQL for `worktree_cleanup_queue` (V39). The contract — retried,
//! bounded, visible — is stated once on
//! [`WorktreeCleanupQueuePort`].

use rusqlite::params;

use crate::ports::worktree_cleanup::{
    normalize_queue_path, CleanupFailure, LeakedWorktree, WorktreeCleanupQueuePort,
};

use super::super::SqliteAdapter;

const COLUMNS: &str = "machine_id, path, feature_id, last_error, attempts,
     auto_attempt_base, first_enqueued_at, last_attempt_at";

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<LeakedWorktree> {
    Ok(LeakedWorktree {
        machine_id: row.get(0)?,
        path: row.get(1)?,
        feature_id: row.get(2)?,
        last_error: row.get(3)?,
        attempts: row.get(4)?,
        auto_attempt_base: row.get(5)?,
        first_enqueued_at: row.get(6)?,
        last_attempt_at: row.get(7)?,
    })
}

impl WorktreeCleanupQueuePort for SqliteAdapter {
    fn record_failure(&self, failure: CleanupFailure<'_>) -> Result<LeakedWorktree, String> {
        let path = normalize_queue_path(failure.path);
        let conn = self.conn.lock()?;
        // COALESCE on `feature_id`: a sweep re-reporting the same path
        // has no feature in hand, and overwriting with NULL would strip
        // the notice of the only thing that names the leftover.
        let sql = format!(
            "INSERT INTO worktree_cleanup_queue
                 ({COLUMNS})
             VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, ?5)
             ON CONFLICT(machine_id, path) DO UPDATE SET
                 attempts = attempts + 1,
                 last_error = excluded.last_error,
                 last_attempt_at = excluded.last_attempt_at,
                 feature_id = COALESCE(excluded.feature_id, feature_id)
             RETURNING {COLUMNS}"
        );
        conn.query_row(
            &sql,
            params![
                failure.machine_id,
                path,
                failure.feature_id,
                failure.error,
                failure.now
            ],
            row_to_entry,
        )
        .map_err(|e| e.to_string())
    }

    fn record_success(&self, machine_id: &str, path: &str) -> Result<bool, String> {
        let path = normalize_queue_path(path);
        let conn = self.conn.lock()?;
        let removed = conn
            .execute(
                "DELETE FROM worktree_cleanup_queue
                 WHERE machine_id = ?1 AND path = ?2",
                params![machine_id, path],
            )
            .map_err(|e| e.to_string())?;
        Ok(removed > 0)
    }

    fn list(&self, machine_id: &str) -> Result<Vec<LeakedWorktree>, String> {
        let conn = self.conn.lock()?;
        let sql = format!(
            "SELECT {COLUMNS} FROM worktree_cleanup_queue
             WHERE machine_id = ?1
             ORDER BY first_enqueued_at ASC, path ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![machine_id], row_to_entry)
            .map_err(|e| e.to_string())?;
        iter.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    // The cap is applied here rather than in a WHERE clause so that
    // `needs_attention` — what the notice keys on — and "still swept" can
    // never disagree about where the boundary is.
    fn due_for_retry(&self, machine_id: &str) -> Result<Vec<LeakedWorktree>, String> {
        Ok(self
            .list(machine_id)?
            .into_iter()
            .filter(|e| !e.needs_attention())
            .collect())
    }

    fn reset_attempts(&self, machine_id: &str, path: &str, now: i64) -> Result<(), String> {
        let path = normalize_queue_path(path);
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE worktree_cleanup_queue
             SET auto_attempt_base = attempts, last_attempt_at = ?3
             WHERE machine_id = ?1 AND path = ?2",
            params![machine_id, path, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/worktree_cleanup.rs"]
mod tests;
