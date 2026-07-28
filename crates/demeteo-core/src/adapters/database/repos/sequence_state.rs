//! SQL for the durable sequence-step run state (V32, task P1.9):
//! `sequence_checkpoints` + `sequence_plan_cache`. Replaces the
//! in-memory `ExecutionDriver::{sequence_checkpoints, cached_plans}`
//! maps so a restart resumes a sequence step from the exact task, not
//! the step head. Exposed through
//! [`SequenceResumeRepository`](crate::ports::db::SequenceResumeRepository),
//! whose `impl` for `SqliteAdapter` is at the foot of this file; peer of
//! the `step_attempts.rs` SQL.
//!
//! V35 adds `anchor_sha`: the commit the landed prefix ends at. V32
//! stored ids alone because its only writer merged the prefix to the
//! feature branch first, so "skip these ids" was a complete instruction.
//! The task loop now checkpoints *as each task lands*, before any merge,
//! so the row has to say where the work is as well as what it is.

use rusqlite::params;
use rusqlite::OptionalExtension;

use crate::domain::ids::FeatureId;
use crate::domain::models::{CheckpointProduced, SequenceCheckpoint};
use crate::ports::db::SequenceResumeRepository;

use super::super::SqliteAdapter;

/// Serialize a produced payload for the `produced_json` column; `None`
/// stays `None`, which the column reads back as *unknown*.
fn produced_column(produced: Option<&CheckpointProduced>) -> Result<Option<String>, String> {
    produced
        .map(|p| serde_json::to_string(p).map_err(|e| e.to_string()))
        .transpose()
}

/// The (feature, node) resume point: landed task ids in landed order, the
/// commit they end at, and what they produced. Empty when the step never
/// checkpointed (or already completed and cleared).
pub fn sequence_checkpoint_get(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_id: &str,
) -> Result<SequenceCheckpoint, String> {
    let conn = adapter.conn.lock()?;
    let row: Option<(String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT landed_task_ids, anchor_sha, produced_json FROM sequence_checkpoints
             WHERE feature_id = ?1 AND step_id = ?2",
            params![feature_id.0, step_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match row {
        Some((json, anchor_sha, produced_json)) => Ok(SequenceCheckpoint {
            landed_task_ids: serde_json::from_str(&json).map_err(|e| e.to_string())?,
            anchor_sha: anchor_sha.filter(|s| !s.trim().is_empty()),
            // A payload that will not parse is treated as absent rather
            // than failing the read: "unknown" degrades to the pre-V36
            // behaviour, while an `Err` here would fail the resume and
            // re-run a task list the row could still have shortened.
            produced: produced_json
                .filter(|s| !s.trim().is_empty())
                .and_then(|s| match serde_json::from_str(&s) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        tracing::warn!(
                            feature_id = %feature_id.0,
                            step_id = %step_id,
                            error = %e,
                            "sequence checkpoint: could not parse the produced payload; \
                             treating it as unknown"
                        );
                        None
                    }
                }),
        }),
        None => Ok(SequenceCheckpoint::default()),
    }
}

/// Union `landed_task_ids` into the (feature, node) checkpoint,
/// preserving landed order and dropping duplicates — the same merge the
/// old in-memory entry did — and move the anchor to `anchor_sha`.
/// Returns the total landed count after the merge (the failure message
/// reports it).
///
/// The anchor is overwritten, not merged: it names the *tip* of the
/// landed prefix, so the newest write is by definition the right one.
/// Passing `None` leaves the stored anchor alone rather than clearing it
/// — a caller that could not read a HEAD knows less than the row does.
///
/// `produced` unions the same way the ids do, under one rule that keeps
/// the payload honest: it is `Some` only when it covers **every** landed
/// id. A row that already names tasks but carries no payload was written
/// before V36, and unioning this task's output into it would produce a
/// set that looks complete while silently omitting the earlier tasks'
/// artifacts — which is exactly the input the declared-deliverable check
/// would then misjudge. Such a row stays "unknown" for the rest of the
/// step's life, and the resume falls back to its pre-V36 behaviour.
pub fn sequence_checkpoint_record(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_id: &str,
    landed_task_ids: &[String],
    anchor_sha: Option<&str>,
    produced: Option<&CheckpointProduced>,
    now: i64,
) -> Result<u32, String> {
    let existing = sequence_checkpoint_get(adapter, feature_id, step_id)?;
    let had_landed = !existing.landed_task_ids.is_empty();
    let mut merged = existing.landed_task_ids;
    for id in landed_task_ids {
        if !merged.contains(id) {
            merged.push(id.clone());
        }
    }
    let anchor = anchor_sha
        .map(|s| s.to_string())
        .or(existing.anchor_sha)
        .filter(|s| !s.trim().is_empty());

    let merged_produced = match (existing.produced, produced) {
        // Nothing new to add: whatever the row knows, it keeps knowing.
        (existing, None) => existing,
        // A pre-V36 row that already claims tasks cannot be completed
        // from here — see the rustdoc above.
        (None, Some(_)) if had_landed => None,
        (existing, Some(new)) => {
            let mut acc = existing.unwrap_or_default();
            for r in &new.artifact_refs {
                if !acc.artifact_refs.contains(r) {
                    acc.artifact_refs.push(r.clone());
                }
            }
            for d in &new.satisfied_decls {
                if !acc.satisfied_decls.contains(d) {
                    acc.satisfied_decls.push(d.clone());
                }
            }
            Some(acc)
        }
    };

    let json = serde_json::to_string(&merged).map_err(|e| e.to_string())?;
    let produced_json = produced_column(merged_produced.as_ref())?;
    let conn = adapter.conn.lock()?;
    conn.execute(
        "INSERT INTO sequence_checkpoints
             (feature_id, step_id, landed_task_ids, anchor_sha, produced_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(feature_id, step_id)
         DO UPDATE SET landed_task_ids = ?3, anchor_sha = ?4, produced_json = ?5,
                       updated_at = ?6",
        params![feature_id.0, step_id, json, anchor, produced_json, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(merged.len() as u32)
}

/// Replace the (feature, node) checkpoint wholesale — the row becomes
/// exactly `landed_task_ids` + `anchor_sha`, and `None` *clears* the
/// anchor rather than leaving it.
///
/// Where `sequence_checkpoint_record` only ever grows the checkpoint,
/// this is the one write that can shrink it, which is what a discarded
/// attempt needs: the rollback moves the branch back, and a checkpoint
/// still naming that attempt's commits would tell the next one to
/// `reset --hard` onto work that no longer exists as anything but a
/// pinned ref. One statement rather than clear-then-record, so a crash
/// mid-rewind cannot drop an *earlier* attempt's merged prefix.
///
/// `produced` is replaced along with the ids, and for the same reason: a
/// rewound row that kept this attempt's artifact references would hand
/// the next attempt output belonging to commits the rollback discarded.
pub fn sequence_checkpoint_set(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_id: &str,
    landed_task_ids: &[String],
    anchor_sha: Option<&str>,
    produced: Option<&CheckpointProduced>,
    now: i64,
) -> Result<(), String> {
    let json = serde_json::to_string(landed_task_ids).map_err(|e| e.to_string())?;
    let anchor = anchor_sha
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty());
    let produced_json = produced_column(produced)?;
    let conn = adapter.conn.lock()?;
    conn.execute(
        "INSERT INTO sequence_checkpoints
             (feature_id, step_id, landed_task_ids, anchor_sha, produced_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(feature_id, step_id)
         DO UPDATE SET landed_task_ids = ?3, anchor_sha = ?4, produced_json = ?5,
                       updated_at = ?6",
        params![feature_id.0, step_id, json, anchor, produced_json, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete the (feature, node) checkpoint — called when the step finally
/// completes, because a stale skip-list would silently exempt tasks
/// from a future full re-run.
pub fn sequence_checkpoint_clear(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_id: &str,
) -> Result<(), String> {
    let conn = adapter.conn.lock()?;
    conn.execute(
        "DELETE FROM sequence_checkpoints WHERE feature_id = ?1 AND step_id = ?2",
        params![feature_id.0, step_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// The last full plan this (feature, node) resolved, as serialized
/// JSON; `None` when the step never planned.
pub fn plan_cache_get(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_id: &str,
) -> Result<Option<String>, String> {
    let conn = adapter.conn.lock()?;
    conn.query_row(
        "SELECT plan_json FROM sequence_plan_cache
         WHERE feature_id = ?1 AND step_id = ?2",
        params![feature_id.0, step_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// Upsert the (feature, node) plan, recording the attempt that
/// produced it.
pub fn plan_cache_put(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_id: &str,
    plan_json: &str,
    attempt_no: Option<u32>,
    now: i64,
) -> Result<(), String> {
    let conn = adapter.conn.lock()?;
    conn.execute(
        "INSERT INTO sequence_plan_cache (feature_id, step_id, plan_json, attempt_no, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(feature_id, step_id)
         DO UPDATE SET plan_json = ?3, attempt_no = ?4, updated_at = ?5",
        params![feature_id.0, step_id, plan_json, attempt_no, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// The port face of this module. Every method delegates to the free
/// function above it — those take `&SqliteAdapter` so the SQL stays
/// callable (and unit-testable) without going through `dyn`.
impl SequenceResumeRepository for SqliteAdapter {
    fn sequence_checkpoint_get(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<SequenceCheckpoint, String> {
        sequence_checkpoint_get(self, feature_id, step_id)
    }

    fn sequence_checkpoint_record(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
        landed_task_ids: &[String],
        anchor_sha: Option<&str>,
        produced: Option<&CheckpointProduced>,
        now: i64,
    ) -> Result<u32, String> {
        sequence_checkpoint_record(
            self,
            feature_id,
            step_id,
            landed_task_ids,
            anchor_sha,
            produced,
            now,
        )
    }

    fn sequence_checkpoint_set(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
        landed_task_ids: &[String],
        anchor_sha: Option<&str>,
        produced: Option<&CheckpointProduced>,
        now: i64,
    ) -> Result<(), String> {
        sequence_checkpoint_set(
            self,
            feature_id,
            step_id,
            landed_task_ids,
            anchor_sha,
            produced,
            now,
        )
    }

    fn sequence_checkpoint_clear(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<(), String> {
        sequence_checkpoint_clear(self, feature_id, step_id)
    }

    fn plan_cache_get(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<Option<String>, String> {
        plan_cache_get(self, feature_id, step_id)
    }

    fn plan_cache_put(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
        plan_json: &str,
        attempt_no: Option<u32>,
        now: i64,
    ) -> Result<(), String> {
        plan_cache_put(self, feature_id, step_id, plan_json, attempt_no, now)
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/sequence_state.rs"]
mod tests;
