//! Ready-set scheduling glue for the run loop (P1.12).
//!
//! The pure half: derive the scheduler's [`NodeState`] view from the
//! persisted `step_executions` rows, and compute which nodes a redirect
//! rewinds. The impure half: persist those decisions (skip statuses,
//! redirect resets) *before* the loop acts on them, emitting the matching
//! `StepProgress` events after each write — the same durable-first
//! ordering `updates::update_step_status` has always used.
//!
//! # Why states are derived, not held
//!
//! The DB is the only state the driver trusts across restarts, replays,
//! and gate-decide recoveries, and the loop already re-reads the step
//! rows every iteration. Deriving the scheduler view from that same read
//! makes restart reconciliation free: whatever `startup_watchdog` or
//! `replay_steps_from` wrote (`interrupted`, `pending`) is exactly what
//! the next evaluation schedules from — there is no in-memory cursor to
//! resynchronize (the v1 `step_index` prefix scan this replaces).
//!
//! The status mapping mirrors the v1 resume semantics precisely:
//! `completed` is terminal, `skipped` is terminal with its reason, and
//! **everything else is `Pending`** — v1 re-dispatched the first
//! non-completed step whatever its status was (`failed` after a crash,
//! `interrupted` from the watchdog, `awaiting_gate` from a dead driver —
//! the gate handler reconciles its own decided row on re-dispatch).
//! Between dispatches nothing is genuinely in flight (`max_parallel_nodes
//! = 1`), so mapping stale active statuses to `Pending` is truthful, not
//! optimistic.

use std::collections::HashMap;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::scheduler::NodeState;
use crate::domain::ids::StepId;
use crate::domain::models::StepExecution;
use crate::domain::workflow_graph::WorkflowGraph;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

/// Step-execution `status` written when the scheduler skips a node
/// (guard evaluated false / dependency unsatisfiable). Unreachable for
/// migrated v1 chains — a failed dependency terminates the run before
/// skip propagation could observe it — but the vocabulary lands with the
/// engine so v2-native graphs (P4) don't need a migration.
pub(crate) const STATUS_SKIPPED: &str = "skipped";

/// Map one persisted step row onto the scheduler's state vocabulary.
pub(crate) fn node_state_for(status: &str, error_message: Option<&str>) -> NodeState {
    match status {
        "completed" => NodeState::Completed,
        STATUS_SKIPPED => NodeState::Skipped {
            reason: error_message.unwrap_or_default().to_string(),
        },
        _ => NodeState::Pending,
    }
}

/// Derive the scheduler view for every node in the graph. A node without
/// a row (bookkeeping lag — rows are seeded 1:1 at feature start) is
/// `Pending`; the dispatch site fails loudly if such a node ever becomes
/// ready without a row to run against.
pub(crate) fn derive_states(
    graph: &WorkflowGraph,
    step_execs: &[StepExecution],
) -> HashMap<StepId, NodeState> {
    graph
        .topological_order()
        .into_iter()
        .map(|id| {
            let state = step_execs
                .iter()
                .find(|s| s.step_id == *id)
                .map(|s| node_state_for(&s.status, s.error_message.as_deref()))
                .unwrap_or(NodeState::Pending);
            (id.clone(), state)
        })
        .collect()
}

/// The nodes a redirect to `target` rewinds: the target plus all its
/// descendants, in topological order. For a chain this is exactly the v1
/// cursor jump — every step from the target to the end of the list gets
/// re-run — and for a DAG it is the whole downstream cone, including the
/// failing node itself (the redirect target is always its ancestor).
pub(crate) fn redirect_reset_set(graph: &WorkflowGraph, target: &StepId) -> Vec<StepId> {
    let Some(descendants) = graph.descendants(target) else {
        return Vec::new();
    };
    graph
        .topological_order()
        .into_iter()
        .filter(|id| *id == target || descendants.contains(*id))
        .cloned()
        .collect()
}

/// The `step_executions` row a skip must be recorded against.
///
/// Split out from [`ExecutionDriver::persist_skip`] so the "no row" decision
/// is testable without a fully wired driver — and because it is a *decision*,
/// not a lookup: returning `Ok(None)` here (what the code used to do
/// implicitly) hands the run loop a skip it will re-decide on every following
/// iteration.
pub(crate) fn skip_target<'a>(
    step_execs: &'a [StepExecution],
    node_id: &StepId,
) -> Result<&'a StepExecution, String> {
    step_execs
        .iter()
        .find(|s| s.step_id == *node_id)
        .ok_or_else(|| {
            format!("no step_executions row for node '{node_id}' to record its skip against")
        })
}

impl ExecutionDriver {
    /// Persist a redirect rewind: park `target` and its descendants back
    /// at `pending` so the next ready-set evaluation re-schedules them,
    /// and mirror each write with a `StepProgress` event (the same
    /// rewind-visibility event `replay_steps_from` emits). Statuses only:
    /// error messages, artifacts, iteration counts, and gate-decision
    /// rows all stay — v1's cursor jump left them for the re-dispatch to
    /// overwrite, and the retry accounting reads `iteration_count`.
    pub(crate) fn reset_for_redirect(&self, step_execs: &[StepExecution], target: &StepId) {
        for id in redirect_reset_set(&self.graph, target) {
            let Some(row) = step_execs.iter().find(|s| s.step_id == id) else {
                continue;
            };
            if row.status == "pending" {
                continue;
            }
            let _ = self.features.step_update(
                &row.id,
                &StepExecutionPatch {
                    status: Some("pending".to_string()),
                    ..Default::default()
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: row.step_id.0.clone(),
                status: "pending".into(),
                cost_usd: row.cost_usd,
                tokens: row.tokens,
                wall_clock_secs: row.wall_clock_secs,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            });
        }
    }

    /// Persist one scheduler-decided skip (durable before the loop acts),
    /// carrying the reason in `error_message` — the dim-with-tooltip
    /// rendering source (PRD §6.1).
    ///
    /// **Fallible on purpose.** The run loop re-derives its whole state view
    /// from these rows every iteration, so a skip that doesn't land is not a
    /// dropped notification — it is a decision the next evaluation will make
    /// again, from the same inputs, forever, with nothing awaited in between.
    /// Both ways that can happen (no row for the node, a repository error)
    /// return `Err` so the caller can fail the run loudly, exactly as the
    /// *ready* path already does for a node with no row.
    pub(crate) fn persist_skip(
        &self,
        step_execs: &[StepExecution],
        node_id: &StepId,
        reason: &str,
    ) -> Result<(), String> {
        let row = skip_target(step_execs, node_id)?;
        super::super::super::updates::try_update_step_status(
            &*self.features,
            &*self.notif,
            row,
            &self.f_id,
            STATUS_SKIPPED,
            row.cost_usd.unwrap_or(0.0),
            row.tokens,
            row.wall_clock_secs.unwrap_or(0),
            None,
            Some(reason.to_string()),
            None,
            None,
        )
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/driver_schedule.rs"]
mod schedule_tests;
