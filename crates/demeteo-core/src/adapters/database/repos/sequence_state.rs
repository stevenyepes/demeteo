//! SQL for the durable sequence-step run state (V32, task P1.9):
//! `sequence_checkpoints` + `sequence_plan_cache`. Replaces the
//! in-memory `ExecutionDriver::{sequence_checkpoints, cached_plans}`
//! maps so a restart resumes a sequence step from the exact task, not
//! the step head. Exposed through `FeatureRepository` (see
//! `repos/feature.rs`), peer of the `step_attempts.rs` SQL.
//!
//! V35 adds `anchor_sha`: the commit the landed prefix ends at. V32
//! stored ids alone because its only writer merged the prefix to the
//! feature branch first, so "skip these ids" was a complete instruction.
//! The task loop now checkpoints *as each task lands*, before any merge,
//! so the row has to say where the work is as well as what it is.

use rusqlite::params;
use rusqlite::OptionalExtension;

use crate::domain::ids::FeatureId;
use crate::domain::models::{SequenceCheckpoint, SequenceProduced};

use super::super::SqliteAdapter;

/// The (feature, node) resume point: landed task ids in landed order,
/// plus the commit they end at. Empty when the step never checkpointed
/// (or already completed and cleared).
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
            // An unparseable payload degrades to `None` = unknown, which is
            // the same honest answer a pre-V36 row gives. Failing the read
            // would strand a resume that only needed the ids.
            produced: produced_json
                .filter(|s| !s.trim().is_empty())
                .and_then(|s| serde_json::from_str(&s).ok()),
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
pub fn sequence_checkpoint_record(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_id: &str,
    landed_task_ids: &[String],
    anchor_sha: Option<&str>,
    produced: Option<&SequenceProduced>,
    now: i64,
) -> Result<u32, String> {
    let existing = sequence_checkpoint_get(adapter, feature_id, step_id)?;
    let existing_produced = existing
        .produced
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| e.to_string())?;
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
    let produced_json = encode_produced(produced)?.or(existing_produced);
    let json = serde_json::to_string(&merged).map_err(|e| e.to_string())?;
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
pub fn sequence_checkpoint_set(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
    step_id: &str,
    landed_task_ids: &[String],
    anchor_sha: Option<&str>,
    produced: Option<&SequenceProduced>,
    now: i64,
) -> Result<(), String> {
    let json = serde_json::to_string(landed_task_ids).map_err(|e| e.to_string())?;
    let produced_json = encode_produced(produced)?;
    let anchor = anchor_sha
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty());
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

/// Serialize a produced payload for storage. `None` in, `None` out — the
/// callers distinguish "no payload to write" from "an empty payload", and
/// so must the column.
fn encode_produced(produced: Option<&SequenceProduced>) -> Result<Option<String>, String> {
    produced
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| e.to_string())
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

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/sequence_state.rs"]
mod tests;
