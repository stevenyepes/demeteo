//! Feature-level status reconciliation (`ExecutionDriver`).

use super::ExecutionDriver;

impl ExecutionDriver {
    /// Move the feature to `running` when a resume left it parked at a gate
    /// status while we're actively driving a non-gate step.
    ///
    /// The fresh-start bootstrap tail flips the feature to `running`, but the
    /// resume paths (gate decision, watchdog, app restart) go through
    /// `start_execution_with_ctx`, which never does — so a resumed run keeps
    /// reading `awaiting_gate`/`gated` and the UI mislabels a running feature
    /// as parked at a gate. Idempotent, and only nudges the transient
    /// in-flight statuses: it never clobbers a terminal state (the run loop
    /// wouldn't be executing for one), and gate steps are left to
    /// `handle_gate_step`, which owns the `awaiting_gate` transition.
    pub(crate) fn ensure_feature_running(&self) {
        let cur = self
            .features
            .get(&self.f_id)
            .ok()
            .flatten()
            .map(|f| f.status);
        if matches!(
            cur.as_deref(),
            Some("awaiting_gate") | Some("gated") | Some("bootstrapping")
        ) {
            let _ = self.features.update(
                &self.f_id,
                &crate::ports::db::FeaturePatch {
                    status: Some("running".to_string()),
                    ..Default::default()
                },
            );
            let _ = self.notif.emit(
                &crate::ports::notification::DomainEvent::FeatureStatusChanged {
                    feature_id: self.f_id.clone(),
                    status: "running".to_string(),
                },
            );
        }
    }
}
