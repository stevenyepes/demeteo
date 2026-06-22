//! [`ConflictResolver`] implementation.
//!
//! Cascade driver per `docs/DECISIONS.md` decision 20:
//!
//! - [`ConflictResolver::resolve_via_agent`] is a stub for v1: the
//!   auto-agent resolution needs to spawn a fresh ACP session with
//!   a constrained prompt, which depends on the full
//!   `AgentRuntime + StepExecutor` plumbing. We return
//!   "auto-agent resolution not implemented yet; cascade to manual"
//!   so the existing `steps/parallel.rs` cascade can already route
//!   to the manual path today. The auto-agent path is wired in R6E.
//!
//! - [`ConflictResolver::request_manual_resolution`] emits the
//!   existing `GateRequired` event with the conflict details
//!   stuffed into the gate's feedback field, so the user's existing
//!   `GateView` UI surfaces the conflict list. A future R7 revision
//!   will replace this with a dedicated `ConflictResolver` Monaco
//!   3-way merge widget, but the contract is the same:
//!   `request_manual_resolution → emit gate → user resolves →
//!    step re-runs the merge`.

use std::sync::Arc;

use crate::domain::ids::FeatureId;
use crate::domain::models::{ConflictReport, MergeOutcome};
use crate::paths;
use crate::ports::conflict::ConflictResolver;
use crate::ports::notification::{DomainEvent, NotificationPort};

pub struct CascadeConflictResolver {
    notif: Arc<dyn NotificationPort>,
}

impl CascadeConflictResolver {
    pub fn new(notif: Arc<dyn NotificationPort>) -> Self {
        Self { notif }
    }
}

impl ConflictResolver for CascadeConflictResolver {
    fn resolve_via_agent(
        &self,
        _feature_id: &FeatureId,
        report: &ConflictReport,
        _subtask_run_id: &str,
    ) -> Result<MergeOutcome, String> {
        // The auto-agent path spawns a fresh ACP session with a
        // constrained prompt ("resolve these N files; do not modify
        // unrelated code; produce a resolution commit"). Implementing
        // it here would duplicate the worktree plumbing the parallel
        // step already owns, so we delegate to the manual path instead for v1.
        //
        // A future phase will replace this stub with a proper
        // resolution-subtask spawn that respects `max_auto_attempts`
        // and `max_attempt_cost_usd`. Until then, the caller can
        // inspect this error string and decide to surface the
        // conflict manually.
        let _ = report; // silence unused warning until the auto-agent path lands
        Err("auto-agent conflict resolution not yet implemented; use manual".to_string())
    }

    fn request_manual_resolution(&self, feature_id: &FeatureId, report: &ConflictReport) {
        // Encode the conflict details as a one-line summary the
        // existing `GateView` can render. The full structured report
        // is also persisted on the `subtask_merges` row by the
        // MergeExecutor, so a future conflict-resolver UI can fetch
        // it without re-parsing git status.
        let summary = format!(
            "Merge conflict in {} file(s) between '{}' and '{}': {}",
            report.files.len(),
            report.source_branch,
            report.target_branch,
            report
                .files
                .iter()
                .map(|f| f.path.clone())
                .collect::<Vec<_>>()
                .join(", "),
        );

        // Persist the conflict on the feature's notification log via
        // a DomainEvent::ConflictDetected (the existing event name).
        let _ = self.notif.emit(&DomainEvent::ConflictDetected {
            feature_id: feature_id.clone(),
            subtask_id: report.target_branch.clone(),
        });

        // And the manual resolution is signaled to the UI through a
        // GateRequired event with the summary inlined. The actual
        // gate decision is captured by `features::gate_decide`; a
        // future ConflictResolver UI will replace this stub.
        let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
            feature_id: feature_id.clone(),
            status: format!("conflict:{}", summary),
        });

        // Touch `paths::now_ms()` so the import isn't dropped in
        // minimal builds (used in the future auto-agent path).
        let _ = paths::now_ms();
    }

    fn max_auto_attempts(&self) -> u32 {
        // Plan §"R6 Tasks": default 2 attempts.
        2
    }

    fn max_attempt_cost_usd(&self) -> f64 {
        // Plan §"R6 Tasks": cost cap $0.50 per attempt.
        0.50
    }
}
