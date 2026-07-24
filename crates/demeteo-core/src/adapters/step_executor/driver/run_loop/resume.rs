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
//! Decision semantics on the parked gate: `approve` re-dispatches the
//! node (the new attempt records the *current* fingerprint, so the
//! blessed state becomes the new baseline); anything else — reject /
//! cancel / redirect — fails the step and feature with a message naming
//! the workspace change (redirect targeting is a real-gate affordance;
//! the synthetic gate's only question is "safe to re-run here?").

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::gate_waiter::GateWaiter;
use crate::domain::ids::GateDecisionId;
use crate::domain::models::{GateDecision, StepExecution};
use crate::paths;
use crate::ports::notification::DomainEvent;

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
    ) -> GuardVerdict {
        // The comparison baseline: the most recent attempt that recorded
        // a fingerprint. Rows predating P1.14 (or opened while the probe
        // failed) yield no baseline — proceed.
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

        tracing::warn!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            recorded = %recorded,
            current = %current,
            "workspace changed under an interrupted node; parking at the synthetic gate"
        );
        let mismatch = format!(
            "Workspace changed while the run was stopped \
             (recorded {recorded}, now {current})"
        );

        // The watchdog usually created the synthetic row at boot; create
        // is a no-op when it exists (unique per step execution).
        let _ = self.gates.create(GateDecision {
            id: GateDecisionId::from(format!("gd-syn-{}", step_exec.id.0)),
            step_execution_id: step_exec.id.clone(),
            decision: None,
            feedback: None,
            created_at: paths::now_ms(),
        });

        // A decision may already be durable (the user answered the
        // watchdog's prompt before this driver armed). Consume it —
        // clearing the row so a later visit re-asks instead of replaying
        // a stale answer.
        if let Ok(Some(rec)) = self.gates.latest_for_step(&step_exec.id) {
            if let Some(decision) = rec.decision.as_deref() {
                let _ = self.gates.reset_for_step_execution(&step_exec.id);
                return match decision {
                    "approve" => GuardVerdict::Proceed,
                    other => GuardVerdict::Rejected(format!(
                        "{mismatch}; user answered '{other}' at the synthetic gate"
                    )),
                };
            }
        }

        // Park: re-surface the prompt and wait for the human, exactly
        // like a real gate step (same waiter registry `gate_decide`'s
        // fast path delivers to; the DB row above is the durable truth).
        let _ = self.notif.emit(&DomainEvent::GateRequired {
            feature_id: self.f_id.clone(),
            step_execution_id: step_exec.id.clone(),
        });
        let waiter = GateWaiter::new();
        self.gate_waiters
            .lock()
            .unwrap()
            .insert(step_exec.id.0.clone(), waiter.clone());
        let mut cancel_watch = self.cancel_watch.clone();
        let decision = tokio::select! {
            d = waiter.wait() => d,
            _ = cancel_watch.changed() => None,
        };
        self.gate_waiters.lock().unwrap().remove(&step_exec.id.0);

        let Some(decision) = decision else {
            return GuardVerdict::Cancelled;
        };
        let _ = self.gates.reset_for_step_execution(&step_exec.id);
        match decision.decision.as_deref() {
            Some("approve") => GuardVerdict::Proceed,
            other => GuardVerdict::Rejected(format!(
                "{mismatch}; user answered '{}' at the synthetic gate",
                other.unwrap_or("none")
            )),
        }
    }
}
