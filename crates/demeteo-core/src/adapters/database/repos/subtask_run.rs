use rusqlite::params;

use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::{SubtaskRunMirrorRow, SubtaskRunRow};
use crate::ports::db::SubtaskRunRepository;

use super::super::SqliteAdapter;

/// Every `subtask_runs` row for a step execution, in start order — the
/// per-task status/cost the sequence-node drill-down joins onto the plan
/// (task P2.5). A read, so it lives beside the write impl but is surfaced
/// through `FeatureRepository` (peer of the sequence-state reads), which is
/// what `RunView` holds.
pub fn subtask_runs_for_step(
    adapter: &SqliteAdapter,
    step_execution_id: &StepExecutionId,
) -> Result<Vec<SubtaskRunRow>, String> {
    let conn = adapter.conn.lock()?;
    let mut stmt = conn
        .prepare(
            "SELECT subtask_id, status, cost_usd, tokens, error_message
             FROM subtask_runs
             WHERE step_execution_id = ?1
             ORDER BY started_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![step_execution_id.0], |row| {
            Ok(SubtaskRunRow {
                subtask_id: row.get(0)?,
                status: row.get(1)?,
                cost_usd: row.get(2)?,
                tokens: row.get(3)?,
                error_message: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Every `subtask_runs` row for a step execution, whole — the read behind
/// the runner's `get_sequence_state` RPC (C4.1). `subtask_runs_for_step`
/// above drops `id`/`agent_id`/`worktree_path`/`branch`, which the mirror
/// write on the laptop side needs to reconstruct the row.
pub fn subtask_runs_mirror_for_step(
    adapter: &SqliteAdapter,
    step_execution_id: &StepExecutionId,
) -> Result<Vec<SubtaskRunMirrorRow>, String> {
    let conn = adapter.conn.lock()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, subtask_id, agent_id, worktree_path, branch, status, cost_usd,
                    tokens, error_message, started_at, ended_at
             FROM subtask_runs
             WHERE step_execution_id = ?1
             ORDER BY started_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![step_execution_id.0], |row| {
            Ok(SubtaskRunMirrorRow {
                id: row.get(0)?,
                subtask_id: row.get(1)?,
                agent_id: row.get(2)?,
                worktree_path: row.get(3)?,
                branch: row.get(4)?,
                status: row.get(5)?,
                cost_usd: row.get(6)?,
                tokens: row.get(7)?,
                error_message: row.get(8)?,
                started_at: row.get(9)?,
                ended_at: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Replace every `subtask_runs` row for a step execution with `rows`, in one
/// write — the laptop-side counterpart `hydrate_shadow_feature` calls with a
/// detached run's `get_sequence_state` RPC response. One transaction so a
/// crash mid-write cannot leave the table holding half the old set and half
/// the new one.
pub fn subtask_runs_replace_for_step(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_execution_id: &StepExecutionId,
    rows: &[SubtaskRunMirrorRow],
) -> Result<(), String> {
    let mut conn = adapter.conn.lock()?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM subtask_runs WHERE step_execution_id = ?1",
        params![step_execution_id.0],
    )
    .map_err(|e| e.to_string())?;
    for row in rows {
        tx.execute(
            "INSERT INTO subtask_runs
             (id, feature_id, step_execution_id, subtask_id, agent_id, worktree_path,
              branch, status, cost_usd, tokens, error_message, started_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                row.id,
                feature_id.0,
                step_execution_id.0,
                row.subtask_id,
                row.agent_id,
                row.worktree_path,
                row.branch,
                row.status,
                row.cost_usd,
                row.tokens,
                row.error_message,
                row.started_at,
                row.ended_at,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

impl SubtaskRunRepository for SqliteAdapter {
    #[allow(clippy::too_many_arguments)]
    fn subtask_run_start(
        &self,
        id: &str,
        feature_id: &FeatureId,
        step_execution_id: &StepExecutionId,
        subtask_id: &str,
        agent_id: &str,
        worktree_path: &str,
        branch: &str,
        now: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO subtask_runs
             (id, feature_id, step_execution_id, subtask_id, agent_id, worktree_path,
              branch, status, cost_usd, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', 0.0, ?8)",
            params![
                id,
                feature_id.0,
                step_execution_id.0,
                subtask_id,
                agent_id,
                worktree_path,
                branch,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn subtask_run_finish(
        &self,
        id: &str,
        status: &str,
        cost_usd: f64,
        tokens: i64,
        error_message: Option<&str>,
        now: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE subtask_runs
             SET status = ?2, cost_usd = ?3, tokens = ?4, error_message = ?5, ended_at = ?6
             WHERE id = ?1",
            params![id, status, cost_usd, tokens, error_message, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn subtask_runs_interrupt_stale(
        &self,
        step_execution_id: &StepExecutionId,
        now: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE subtask_runs
             SET status = 'interrupted', ended_at = ?2,
                 error_message = COALESCE(error_message, 'interrupted by restart')
             WHERE step_execution_id = ?1 AND status = 'running'",
            params![step_execution_id.0, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/subtask_run.rs"]
mod subtask_run_tests;
