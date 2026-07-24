//! SQL for the `step_attempts` table (V31, task P1.8) — per-attempt
//! history rows the driver opens on every dispatch and closes with that
//! attempt's own outcome. Exposed through `FeatureRepository`'s
//! `attempt_*` methods (see `repos/feature.rs`), peer of the
//! `feature_steps.rs` step-execution SQL.

use rusqlite::params;

use crate::domain::ids::StepExecutionId;
use crate::domain::models::StepAttempt;

use super::super::SqliteAdapter;

/// Open a `running` attempt row, assigning the next dense 1-based
/// `attempt_no` for the step. The `(step_execution_id, attempt_no)`
/// UNIQUE constraint makes a racing double-open an error instead of a
/// silent duplicate — the driver is single-tasked per feature, so this
/// never fires in practice.
///
/// `workspace_fingerprint` is the P1.14 workspace state at node start
/// (`<HEAD>:<dirty|clean>`, `None` when the probe failed); the row's
/// `idempotency_key` is derived here — `<se_id>#<attempt_no>#<fp>` —
/// because only this function knows the assigned `attempt_no`.
pub fn attempt_open(
    adapter: &SqliteAdapter,
    step_execution_id: &StepExecutionId,
    now: i64,
    workspace_fingerprint: Option<&str>,
) -> Result<u32, String> {
    let conn = adapter.conn.lock()?;
    // A crashed or killed process leaves its in-flight attempt `running`
    // forever; a step re-dispatch means any such row is stale by
    // definition (the driver runs one attempt at a time per step). Close
    // them as `interrupted` so history stays honest — the same
    // self-healing `subtask_runs_interrupt_stale` applies to task rows.
    conn.execute(
        "UPDATE step_attempts
         SET status = 'interrupted', ended_at = ?2
         WHERE step_execution_id = ?1 AND status = 'running'",
        params![step_execution_id.0, now],
    )
    .map_err(|e| e.to_string())?;
    let next_no: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(attempt_no), 0) + 1 FROM step_attempts
             WHERE step_execution_id = ?1",
            params![step_execution_id.0],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let idempotency_key = format!(
        "{}#{}#{}",
        step_execution_id.0,
        next_no,
        workspace_fingerprint.unwrap_or("unknown")
    );
    conn.execute(
        "INSERT INTO step_attempts
             (step_execution_id, attempt_no, status, started_at,
              workspace_fingerprint, idempotency_key)
         VALUES (?1, ?2, 'running', ?3, ?4, ?5)",
        params![
            step_execution_id.0,
            next_no,
            now,
            workspace_fingerprint,
            idempotency_key
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(next_no)
}

#[allow(clippy::too_many_arguments)]
pub fn attempt_close(
    adapter: &SqliteAdapter,
    step_execution_id: &StepExecutionId,
    attempt_no: u32,
    status: &str,
    cost_usd: f64,
    tokens: i64,
    wall_clock_ms: u64,
    error_class: Option<&str>,
    failure_fingerprint: Option<&str>,
    applied_rule: Option<&str>,
    now: i64,
) -> Result<(), String> {
    let conn = adapter.conn.lock()?;
    conn.execute(
        "UPDATE step_attempts
         SET status = ?3, cost_usd = ?4, tokens = ?5, wall_clock_ms = ?6,
             error_class = ?7, failure_fingerprint = ?8, applied_rule = ?9,
             ended_at = ?10
         WHERE step_execution_id = ?1 AND attempt_no = ?2",
        params![
            step_execution_id.0,
            attempt_no,
            status,
            cost_usd,
            tokens,
            wall_clock_ms as i64,
            error_class,
            failure_fingerprint,
            applied_rule,
            now,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn attempts_for_step(
    adapter: &SqliteAdapter,
    step_execution_id: &StepExecutionId,
) -> Result<Vec<StepAttempt>, String> {
    let conn = adapter.conn.lock()?;
    let mut stmt = conn
        .prepare(
            "SELECT step_execution_id, attempt_no, status, cost_usd, tokens,
                    wall_clock_ms, error_class, failure_fingerprint,
                    applied_rule, workspace_fingerprint, idempotency_key,
                    started_at, ended_at
             FROM step_attempts
             WHERE step_execution_id = ?1
             ORDER BY attempt_no",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![step_execution_id.0], |row| {
            Ok(StepAttempt {
                step_execution_id: row.get(0)?,
                attempt_no: row.get(1)?,
                status: row.get(2)?,
                cost_usd: row.get(3)?,
                tokens: row.get(4)?,
                wall_clock_ms: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                error_class: row.get(6)?,
                failure_fingerprint: row.get(7)?,
                applied_rule: row.get(8)?,
                workspace_fingerprint: row.get(9)?,
                idempotency_key: row.get(10)?,
                started_at: row.get(11)?,
                ended_at: row.get(12)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/step_attempts.rs"]
mod tests;
