use rusqlite::params;

use crate::domain::memory::{MemorySignal, SignalKind};
use crate::ports::memory_signals::MemorySignalsPort;

use super::super::SqliteAdapter;

fn row_to_signal(row: &rusqlite::Row) -> rusqlite::Result<MemorySignal> {
    let kind_str: String = row.get(4)?;
    Ok(MemorySignal {
        id: row.get(0)?,
        project_id: row.get(1)?,
        feature_id: row.get(2)?,
        step_execution_id: row.get(3)?,
        kind: SignalKind::from_str(&kind_str),
        content: row.get(5)?,
        created_at: row.get(6)?,
        processed_at: row.get(7)?,
        attempts: row.get(8)?,
    })
}

impl MemorySignalsPort for SqliteAdapter {
    fn enqueue(&self, signal: MemorySignal) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO memory_signals
                (id, project_id, feature_id, step_execution_id, kind, content,
                 created_at, processed_at, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                signal.id,
                signal.project_id,
                signal.feature_id,
                signal.step_execution_id,
                signal.kind.as_str(),
                signal.content,
                signal.created_at,
                signal.processed_at,
                signal.attempts,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn take_unprocessed(
        &self,
        limit: usize,
        max_attempts: i64,
    ) -> Result<Vec<MemorySignal>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, feature_id, step_execution_id, kind, content,
                        created_at, processed_at, attempts
                 FROM memory_signals
                 WHERE processed_at IS NULL AND attempts < ?2
                 ORDER BY created_at ASC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![limit, max_attempts], row_to_signal)
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn mark_processed(&self, ids: &[String], now: i64) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock()?;
        for id in ids {
            conn.execute(
                "UPDATE memory_signals SET processed_at = ?2 WHERE id = ?1",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn bump_attempts(&self, ids: &[String]) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock()?;
        for id in ids {
            conn.execute(
                "UPDATE memory_signals SET attempts = attempts + 1 WHERE id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
