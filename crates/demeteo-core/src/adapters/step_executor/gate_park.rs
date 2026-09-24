//! Parking a *non-gate* node on the synthetic gate and waiting for a human.
//!
//! A synthetic gate is not a new mechanism or a new type: it is an ordinary
//! `gate_decisions` row hung off a node whose kind is not `gate`, answered
//! by the same `gate_decide` the real gates use. Its id names the question:
//! `gd-resume-<step execution>` is the startup watchdog's "may this resume?",
//! `gd-syn-<step execution>` a park's own. [`void_resume_question`] reads
//! that difference; nothing else does.
//!
//! This module is the part that has nothing to do with *why* the run
//! stopped — create the row, surface it, wait, clean up. The reason and the
//! meaning of the answer belong to the caller and to
//! [`crate::domain::step_park`], which is what lets the resume-fingerprint
//! guard and a zero-ticket rework park through the same code instead of two
//! copies that drift.
//!
//! Deliberately a free function over the four ports it needs rather than a
//! method on `ExecutionDriver` (AGENTS.md §3): the driver carries twenty-odd
//! ports this never touches, and a test of parking should not have to stub
//! nineteen of them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;

use crate::domain::ids::{FeatureId, GateDecisionId, StepExecutionId};
use crate::domain::models::GateDecision;
use crate::paths;
use crate::ports::db::GateRepository;
use crate::ports::notification::{DomainEvent, NotificationPort};

use super::gate_waiter::GateWaiter;

/// How often a parked gate re-reads its own decision row.
///
/// [`GateWaiter`]'s docs call the DB the source of truth and the waiter a
/// fast-path wakeup, but without this poll that is only true across a
/// process boundary: in-process, a lost waiter is a lost run. The map is
/// shared by every driver and lives as long as the app, so every way an
/// entry can go missing — a peer run's teardown sweeping too widely, a
/// future registry edit, a race nobody has thought of — ends the same way:
/// the decision durable in SQLite, the driver parked on a rendezvous no
/// `gate_decide` can find, and `ensure_driver_running` declining to spawn a
/// replacement because the wedged driver is still alive. Reading the row the
/// human's click already wrote costs one indexed lookup per parked gate and
/// bounds every one of those to this interval.
///
/// Not a substitute for delivering the wakeup: a human waits on the fast
/// path, and this is the floor under it.
pub(crate) const GATE_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// The ports a park needs, bundled because they travel together.
pub(crate) struct SyntheticGate<'a> {
    pub gates: &'a dyn GateRepository,
    pub notif: &'a dyn NotificationPort,
    pub waiters: &'a Arc<Mutex<HashMap<String, Arc<GateWaiter>>>>,
    pub f_id: &'a FeatureId,
}

fn park_question_id(step_exec_id: &StepExecutionId) -> GateDecisionId {
    GateDecisionId::from(format!("gd-syn-{}", step_exec_id.0))
}

fn resume_question_id(step_exec_id: &StepExecutionId) -> GateDecisionId {
    GateDecisionId::from(format!("gd-resume-{}", step_exec_id.0))
}

fn undecided_row(id: GateDecisionId, step_exec_id: &StepExecutionId) -> GateDecision {
    GateDecision {
        id,
        step_execution_id: step_exec_id.clone(),
        decision: None,
        feedback: None,
        created_at: paths::now_ms(),
    }
}

/// Ask afresh whether `step_exec_id` may resume, discarding any answer its
/// row already holds.
///
/// The row id is derived from the step execution, which outlives any one
/// question: a restart that interrupts the same node twice poses two
/// questions on one row. An answer left from the first — given to a
/// watchdog prompt the fingerprint guard then proceeded past without
/// consuming — would otherwise be read by [`park_for_human`]'s fast path as
/// the answer to the second, and the run resumes over a moved workspace
/// with no human in it.
pub(crate) fn pose_synthetic_gate(
    gates: &dyn GateRepository,
    step_exec_id: &StepExecutionId,
) -> Result<(), String> {
    gates.reopen(undecided_row(
        resume_question_id(step_exec_id),
        step_exec_id,
    ))
}

/// Withdraw the watchdog's "may this resume?" from a node about to run.
///
/// Once the node runs the question is moot, but its row stays answerable
/// (`gate_decision_refusal` admits any non-terminal step), and an answer
/// given to it then would be consumed, unasked, by the next park on this
/// step execution — [`park_for_human`] keeps its fast path only because
/// that cannot happen.
///
/// A park's own question is left alone. A node parked when the process
/// died keeps that row across the restart, and the answer the human gives
/// it is what the re-run's identical park reads on its fast path; voiding
/// it would ask the same question again and lose a `redirect`.
pub(crate) fn void_resume_question(
    gates: &dyn GateRepository,
    step_exec_id: &StepExecutionId,
) -> Result<(), String> {
    match gates.latest_for_step(step_exec_id)? {
        Some(row) if row.id == resume_question_id(step_exec_id) => {
            gates.reset_for_step_execution(step_exec_id)
        }
        _ => Ok(()),
    }
}

/// Park `step_exec_id` on a synthetic gate and wait.
///
/// Returns the decision, or `None` when the run was cancelled while parked.
/// The row is deleted on the way out either way, so a later visit re-asks
/// rather than replaying an answer to a question that is no longer the one
/// being posed.
///
/// A decision already durable in the row is consumed without waiting — the
/// human may have answered the watchdog's prompt before this driver armed.
pub(crate) async fn park_for_human(
    g: SyntheticGate<'_>,
    step_exec_id: &StepExecutionId,
    mut cancel: watch::Receiver<bool>,
) -> Option<GateDecision> {
    // `create`, not `reopen`: the watchdog, or this park before a restart,
    // posed this question and the answer may already be in the row. Keeping
    // it is only sound because nothing older can be — the watchdog reopens
    // rather than creates, and dispatching a non-gate step voids its
    // question (`void_resume_question`).
    let _ = g
        .gates
        .create(undecided_row(park_question_id(step_exec_id), step_exec_id));

    if let Ok(Some(rec)) = g.gates.latest_for_step(step_exec_id) {
        if rec.decision.is_some() {
            let _ = g.gates.reset_for_step_execution(step_exec_id);
            return Some(rec);
        }
    }

    let _ = g.notif.emit(&DomainEvent::GateRequired {
        feature_id: g.f_id.clone(),
        step_execution_id: step_exec_id.clone(),
    });
    let waiter = GateWaiter::new();
    g.waiters
        .lock()
        .unwrap()
        .insert(step_exec_id.0.clone(), waiter.clone());

    let decision = loop {
        tokio::select! {
            d = waiter.wait() => break d,
            _ = cancel.changed() => break None,
            _ = tokio::time::sleep(GATE_POLL_INTERVAL) => {
                match g.gates.latest_for_step(step_exec_id) {
                    Ok(Some(rec)) if rec.decision.is_some() => break Some(rec),
                    _ => continue,
                }
            }
        }
    };

    g.waiters.lock().unwrap().remove(&step_exec_id.0);
    let _ = g.gates.reset_for_step_execution(step_exec_id);
    decision
}

#[cfg(test)]
#[path = "../../../tests/adapters/step_executor/gate_park.rs"]
mod tests;
