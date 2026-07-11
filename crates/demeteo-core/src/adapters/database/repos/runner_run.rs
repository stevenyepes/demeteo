use rusqlite::params;

use crate::ports::runner_run::{RunnerRun, RunnerRunPort};
use crate::shared::secret_scrub::scrub_secrets;

use super::super::SqliteAdapter;

fn row_to_run(row: &rusqlite::Row) -> rusqlite::Result<RunnerRun> {
    Ok(RunnerRun {
        run_id: row.get(0)?,
        project_id: row.get(1)?,
        feature_id: row.get(2)?,
        spec_json: row.get(3)?,
        status: row.get(4)?,
        error: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        resume_count: row.get(8)?,
        pushed_branch: row.get(9)?,
        owner_client_id: row.get(10)?,
    })
}

const SELECT_COLS: &str = "run_id, project_id, feature_id, spec_json, status, error, created_at, \
                            updated_at, resume_count, pushed_branch, owner_client_id";

impl RunnerRunPort for SqliteAdapter {
    fn get_or_create(
        &self,
        run_id: &str,
        spec_json: &str,
        owner_client_id: &str,
        now: i64,
    ) -> Result<RunnerRun, String> {
        let conn = self.conn.lock()?;
        // `INSERT OR IGNORE` keeps this idempotent (R9/M3.2): a re-submit
        // of an existing `run_id` is a no-op, so the original
        // `owner_client_id` stamped at first insert is never overwritten
        // — a run stays owned by whoever created it.
        conn.execute(
            "INSERT OR IGNORE INTO runner_runs
                (run_id, spec_json, status, owner_client_id, created_at, updated_at)
             VALUES (?1, ?2, 'pending', ?3, ?4, ?4)",
            params![run_id, spec_json, owner_client_id, now],
        )
        .map_err(|e| e.to_string())?;

        conn.query_row(
            &format!("SELECT {} FROM runner_runs WHERE run_id = ?1", SELECT_COLS),
            params![run_id],
            row_to_run,
        )
        .map_err(|e| e.to_string())
    }

    fn update_status(
        &self,
        run_id: &str,
        status: &str,
        project_id: Option<&str>,
        feature_id: Option<&str>,
        error: Option<&str>,
        pushed_branch: Option<&str>,
        now: i64,
    ) -> Result<(), String> {
        // Secret scrubbing (M7.2, §6): `error` is usually a stringified
        // foreign error (a failed clone/push/PR) and is surfaced verbatim
        // in the laptop's return inbox and `get_status`. This column is a
        // laptop-visible sink, so scrub any credential-shaped substring
        // here — the single choke point that also covers the direct
        // failure-path writers in `rpc.rs` that don't go through
        // `run::emit`.
        let error = error.map(|e| scrub_secrets(e).into_owned());
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE runner_runs
             SET status = ?2,
                 project_id = COALESCE(?3, project_id),
                 feature_id = COALESCE(?4, feature_id),
                 error = ?5,
                 pushed_branch = COALESCE(?6, pushed_branch),
                 updated_at = ?7
             WHERE run_id = ?1",
            params![
                run_id,
                status,
                project_id,
                feature_id,
                error,
                pushed_branch,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get(&self, run_id: &str) -> Result<Option<RunnerRun>, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!("SELECT {} FROM runner_runs WHERE run_id = ?1", SELECT_COLS),
            params![run_id],
            row_to_run,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.to_string()),
        })
    }

    fn list(&self) -> Result<Vec<RunnerRun>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM runner_runs ORDER BY created_at DESC",
                SELECT_COLS
            ))
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![], row_to_run)
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn mark_all_running_interrupted(&self, now: i64) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE runner_runs
             SET status = 'interrupted', updated_at = ?1
             WHERE status IN ('running', 'pending')",
            params![now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn bump_resume_count(&self, run_id: &str) -> Result<i64, String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE runner_runs SET resume_count = resume_count + 1 WHERE run_id = ?1",
            params![run_id],
        )
        .map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT resume_count FROM runner_runs WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    }

    fn cancel_if_active(&self, run_id: &str, now: i64) -> Result<Option<RunnerRun>, String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE runner_runs
             SET status = 'cancelled', updated_at = ?2
             WHERE run_id = ?1
               AND status NOT IN ('awaiting_mr', 'completed', 'failed', 'cancelled')",
            params![run_id, now],
        )
        .map_err(|e| e.to_string())?;

        conn.query_row(
            &format!("SELECT {} FROM runner_runs WHERE run_id = ?1", SELECT_COLS),
            params![run_id],
            row_to_run,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other.to_string()),
        })
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/runner_run.rs"]
mod tests;
