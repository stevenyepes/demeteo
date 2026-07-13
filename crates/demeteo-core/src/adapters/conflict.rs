//! [`ConflictResolver`] implementation — the cascade driver sketched in
//! `docs/DECISIONS.md` decision 20.
//!
//! **Nothing calls this today.** The two steps that merge a subtask branch
//! back — `steps::agent` and `steps::sequence` — resolve conflicts inline
//! (`steps::conflict_pass`), using the worktree and session they already
//! hold. The cascade this module describes was never wired to them, and the
//! manual-resolution leg has no UI on the other end.
//!
//! It is kept as the shape a future resolution flow would take, not as live
//! code. If you are looking for what actually happens when a merge conflicts,
//! read `steps::conflict_pass`.
//!
//! - [`ConflictResolver::resolve_via_agent`] is a stub: it returns
//!   "not implemented; cascade to manual".
//!
//! - [`ConflictResolver::request_manual_resolution`] emits a
//!   `ConflictDetected` event plus a `conflict:` status, which the UI
//!   currently renders as a notification rather than a resolution surface.

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
        // Resolving here would duplicate the worktree and session plumbing the
        // steps already own — which is why `steps::conflict_pass` does it
        // there instead, and why this stays a stub. A resolver that took over
        // would have to respect `max_auto_attempts` and `max_attempt_cost_usd`
        // the way the inline pass does not.
        let _ = report; // silence unused warning while the auto-agent path is a stub
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
