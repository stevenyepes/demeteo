use rusqlite::params;

use crate::ports::remote_run_mirror::{RemoteRunMirror, RemoteRunMirrorPort};

use super::super::SqliteAdapter;

const SELECT_COLS: &str = "machine_id, run_id, project_id, title, status, error, feature_id, \
                            pr_url, pushed_branch, last_offset, created_at, updated_at, \
                            last_notified_status";

fn row_to_mirror(row: &rusqlite::Row) -> rusqlite::Result<RemoteRunMirror> {
    Ok(RemoteRunMirror {
        machine_id: row.get(0)?,
        run_id: row.get(1)?,
        project_id: row.get(2)?,
        title: row.get(3)?,
        status: row.get(4)?,
        error: row.get(5)?,
        feature_id: row.get(6)?,
        pr_url: row.get(7)?,
        pushed_branch: row.get(8)?,
        last_offset: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        last_notified_status: row.get(12)?,
    })
}

impl RemoteRunMirrorPort for SqliteAdapter {
    fn upsert_submitted(
        &self,
        machine_id: &str,
        run_id: &str,
        project_id: Option<&str>,
        title: &str,
        now: i64,
    ) -> Result<RemoteRunMirror, String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT OR IGNORE INTO remote_run_mirror
                (machine_id, run_id, project_id, title, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?5)",
            params![machine_id, run_id, project_id, title, now],
        )
        .map_err(|e| e.to_string())?;
        let row: RemoteRunMirror = conn
            .query_row(
                &format!(
                    "SELECT {} FROM remote_run_mirror WHERE machine_id = ?1 AND run_id = ?2",
                    SELECT_COLS
                ),
                params![machine_id, run_id],
                row_to_mirror,
            )
            .map_err(|e| e.to_string())?;
        Ok(row)
    }

    fn update_status(
        &self,
        machine_id: &str,
        run_id: &str,
        status: &str,
        error: Option<&str>,
        feature_id: Option<&str>,
        pr_url: Option<&str>,
        pushed_branch: Option<&str>,
        last_offset: i64,
        now: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE remote_run_mirror
                SET status = ?3,
                    error = COALESCE(?4, error),
                    feature_id = COALESCE(?5, feature_id),
                    pr_url = COALESCE(?6, pr_url),
                    pushed_branch = COALESCE(?7, pushed_branch),
                    last_offset = MAX(last_offset, ?8),
                    updated_at = ?9
              WHERE machine_id = ?1 AND run_id = ?2",
            params![
                machine_id,
                run_id,
                status,
                error,
                feature_id,
                pr_url,
                pushed_branch,
                last_offset,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn mark_notified(&self, machine_id: &str, run_id: &str, status: &str) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE remote_run_mirror SET last_notified_status = ?3
              WHERE machine_id = ?1 AND run_id = ?2",
            params![machine_id, run_id, status],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get(&self, machine_id: &str, run_id: &str) -> Result<Option<RemoteRunMirror>, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!(
                "SELECT {} FROM remote_run_mirror WHERE machine_id = ?1 AND run_id = ?2",
                SELECT_COLS
            ),
            params![machine_id, run_id],
            row_to_mirror,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e.to_string()),
        })
    }

    fn list(&self) -> Result<Vec<RemoteRunMirror>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM remote_run_mirror ORDER BY updated_at DESC",
                SELECT_COLS
            ))
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map([], row_to_mirror)
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/remote_run_mirror.rs"]
mod tests;
