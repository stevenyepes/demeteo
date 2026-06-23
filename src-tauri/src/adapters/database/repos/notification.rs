use rusqlite::params;
use std::str::FromStr;

use crate::domain::ids::ProjectId;
use crate::domain::models::{Notification, NotificationKind};
use crate::ports::db::NotificationRepository;

use super::super::SqliteAdapter;

const SELECT_COLS: &str =
    "id, project_id, feature_id, kind, message, feature_url, read, created_at";

impl NotificationRepository for SqliteAdapter {
    fn add(&self, n: Notification) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO notifications (id, project_id, feature_id, kind, message, feature_url, read, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                n.id,
                n.project_id,
                n.feature_id,
                n.kind.as_str(),
                n.message,
                n.feature_url,
                n.read as i64,
                n.created_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list(
        &self,
        project_id: Option<&ProjectId>,
        limit: u32,
    ) -> Result<Vec<Notification>, String> {
        let conn = self.conn.lock()?;
        // Two paths because binding `None` to a positional filter is
        // awkward in rusqlite and we want a prepared statement per
        // shape (the row count for the bell panel is tiny).
        let rows: Vec<Notification> = if let Some(pid) = project_id {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {} FROM notifications WHERE project_id = ?1 ORDER BY created_at DESC LIMIT ?2",
                    SELECT_COLS
                ))
                .map_err(|e| e.to_string())?;
            let iter = stmt
                .query_map(params![pid.0, limit], row_to_notification)
                .map_err(|e| e.to_string())?;
            iter.filter_map(|r| r.ok()).collect()
        } else {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {} FROM notifications ORDER BY created_at DESC LIMIT ?1",
                    SELECT_COLS
                ))
                .map_err(|e| e.to_string())?;
            let iter = stmt
                .query_map(params![limit], row_to_notification)
                .map_err(|e| e.to_string())?;
            iter.filter_map(|r| r.ok()).collect()
        };
        Ok(rows)
    }

    fn mark_read(&self, id: &str) -> Result<u32, String> {
        let conn = self.conn.lock()?;
        let updated = conn
            .execute(
                "UPDATE notifications SET read = 1 WHERE id = ?1 AND read = 0",
                params![id],
            )
            .map_err(|e| e.to_string())?;
        Ok(updated as u32)
    }

    fn unread_count(&self) -> Result<u32, String> {
        let conn = self.conn.lock()?;
        // `idx_notifications_unread` is a partial index keyed on
        // `read = 0`, so this stays O(unread) even when the history
        // table grows.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE read = 0",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count as u32)
    }
}

fn row_to_notification(row: &rusqlite::Row) -> rusqlite::Result<Notification> {
    let kind_str: String = row.get(3)?;
    let kind = NotificationKind::from_str(&kind_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(Notification {
        id: row.get(0)?,
        project_id: row.get(1)?,
        feature_id: row.get(2)?,
        kind,
        message: row.get(4)?,
        feature_url: row.get(5)?,
        read: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}
