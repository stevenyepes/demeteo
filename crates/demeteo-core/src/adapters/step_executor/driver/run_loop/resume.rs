//! Resume fingerprint guard (task P1.14, PRD §5.4; extends Decision 14).
//!
//! When a driver life is about to re-dispatch a node the watchdog marked
//! `interrupted` (only `startup_watchdog` writes that status for active
//! features — manual retries and replays reset rows to `pending`), the
//! workspace may no longer be the one the interrupted attempt was
//! working in: the crash happened at an arbitrary point, and a human may
//! have edited the worktree between crash and restart.
//!
//! The guard compares the fingerprint recorded when that node's last
//! attempt *started* (`step_attempts.workspace_fingerprint`, P1.14
//! columns) against the live workspace:
//!
//! * **match** — the workspace is exactly as the attempt found it; the
//!   interrupted work left no trace and blind re-dispatch is safe (the
//!   pre-P1.14 behavior, preserved for the common kill-while-thinking
//!   crash).
//! * **mismatch** — *something* changed underneath the interrupted node
//!   (partial agent writes, landed sequence prefixes, or a human's
//!   edits — indistinguishable from here). Decision 14 says a mid-step
//!   interrupt surfaces as a **synthetic gate**; the guard makes that
//!   gate real by parking on the same `gd-syn-*` row + [`GateWaiter`]
//!   rendezvous the watchdog already surfaced in the UI, instead of
//!   re-executing while the prompt is still on screen.
//! * **unknown** (no recorded fingerprint / probe failed) — proceed;
//!   missing telemetry must never block a run.
//!
//! The comparison is only *sound* for a node whose entire effect is the
//! worktree, so the node type gets a say first: a handler answering
//! [`ResumePolicy::AlwaysAsk`] (the `command` type's `idempotent: false`
//! case — a deploy leaves no trace a fingerprint could see) parks
//! unconditionally. The guard asks the registry rather than the node's
//! kind, so a future type opts in without touching this file.
//!
//! Decision semantics on the parked gate: `approve` re-dispatches the
//! node (the new attempt records the *current* fingerprint, so the
//! blessed state becomes the new baseline); anything else — reject /
//! cancel / redirect — fails the step and feature with a message naming
//! the workspace change (redirect targeting is a real-gate affordance;
//! the synthetic gate's only question is "safe to re-run here?").

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::registry::{NodeTypeRegistry, ResumePolicy};
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::step_park::{resolve_park, HumanPark, ParkResolution};

/// What the run loop should do after the guard has run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GuardVerdict {
    /// Dispatch the node (fingerprint matched / unknown / approved).
    Proceed,
    /// The user declined to resume — fail the step and feature. Carries
    /// the failure message so the caller owns the (async) writes.
    Rejected(String),
    /// The run was cancelled while parked.
    Cancelled,
}

impl ExecutionDriver {
    /// Run the guard for one watchdog-`interrupted` node. See the module
    /// docs for the decision table.
    pub(crate) async fn resume_fingerprint_guard(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
    ) -> GuardVerdict {
        // Does the fingerprint get a vote at all? A node type that can act
        // outside the worktree answers `AlwaysAsk` and skips straight to
        // the gate — the fingerprint would say "unchanged" about a deploy
        // that already went out.
        let always_ask = NodeTypeRegistry::global()
            .handler_for(&step_conf.kind)
            .map(|h| h.resume_policy(step_conf))
            == Some(ResumePolicy::AlwaysAsk);

        // The comparison baseline: the most recent attempt that recorded
        // a fingerprint. Rows predating P1.14 (or opened while the probe
        // failed) yield no baseline — proceed.
        let mismatch = if always_ask {
            format!(
                "Step '{}' was interrupted mid-run and is not safe to repeat \
                 automatically",
                step_exec.step_id.0
            )
        } else {
            let recorded = match self.features.attempts_for_step(&step_exec.id) {
                Ok(rows) => rows
                    .iter()
                    .rev()
                    .find_map(|a| a.workspace_fingerprint.clone()),
                Err(_) => None,
            };
            let Some(recorded) = recorded else {
                return GuardVerdict::Proceed;
            };
            let Some(current) = self.current_workspace_fingerprint().await else {
                return GuardVerdict::Proceed;
            };
            if recorded == current {
                return GuardVerdict::Proceed;
            }
            format!(
                "Workspace changed while the run was stopped \
                 (recorded {recorded}, now {current})"
            )
        };

        tracing::warn!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            reason = %mismatch,
            "interrupted node cannot be resumed blindly; parking at the synthetic gate"
        );

        // Everything from here is the shared park: create the row,
        // surface it, wait, clean up. Only the reason above is this
        // guard's own — see `domain::step_park` for what the answer means.
        let park = HumanPark {
            reason: mismatch,
            // Redirect targeting is a real-gate affordance. This park's
            // only question is "safe to re-run here?", and no earlier step
            // makes a moved workspace safe.
            redirect_to: None,
        };
        let decision = crate::adapters::step_executor::gate_park::park_for_human(
            crate::adapters::step_executor::gate_park::SyntheticGate {
                gates: self.gates.as_ref(),
                notif: self.notif.as_ref(),
                waiters: &self.gate_waiters,
                f_id: &self.f_id,
            },
            &step_exec.id,
            self.cancel_watch.clone(),
        )
        .await;

        match resolve_park(&park, decision.as_ref()) {
            ParkResolution::Complete => GuardVerdict::Proceed,
            ParkResolution::Cancelled => GuardVerdict::Cancelled,
            ParkResolution::Fail(msg) => GuardVerdict::Rejected(msg),
            // Unreachable with `redirect_to: None` above, and mapped
            // rather than `unreachable!` so a later edit that gives this
            // park a target degrades to the old behaviour instead of
            // panicking a live run.
            ParkResolution::Redirect { .. } => {
                GuardVerdict::Rejected("redirect is not an answer to this gate".to_string())
            }
        }
    }
}
