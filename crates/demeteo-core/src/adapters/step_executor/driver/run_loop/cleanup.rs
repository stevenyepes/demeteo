//! Terminal cleanup after the run loop exits.
//!
//! Called by `mod.rs::run()` when the per-step loop ends without
//! `RunAction::Terminate` — every step done, step_index past the end.
//! Decides the terminal feature status (completed vs awaiting_mr) and
//! finishes the feature row, then sweeps live sessions and the gate
//! waiter map, and finally deregisters from the live-driver registry.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::updates;

impl ExecutionDriver {
    /// Decide the terminal feature status and finish the feature row.
    /// Then sweep sessions + deregister.
    pub(crate) async fn finalize_run(&mut self) {
        let target_status = match self.features.get(&self.f_id) {
            Ok(Some(f)) if f.mr_url.as_ref().is_some_and(|u| !u.is_empty()) => "completed",
            _ => {
                // Every step is done. If the `finalize` step left a PR summary
                // on the row, open the PR now — that is what replaces the old
                // "park in awaiting_mr and wait for a human to click Publish
                // and type a title" flow.
                self.auto_publish_pr().await
            }
        };

        updates::finish_feature(
            &*self.features,
            &*self.notif,
            &self.f_id,
            target_status,
            self.start_time,
        );

        // The pipeline is done. Agent-step sessions are killed inline
        // on every non-success outcome (see `handle_agent_step`), but
        // a session from the *last successful* step is deliberately
        // left alive so a same-fingerprint retry could `--resume` it
        // — nothing does that anymore once the run itself ends here,
        // so sweep every session this feature ever touched (it may
        // have visited more than one fingerprint) rather than leaking
        // them for the life of the app.
        self.registry.kill_all_for_feature(self.f_id.as_str()).await;

        // Drop any stale gate waiter left behind — the loop above
        // consumes them on success, but cancellation / failure paths
        // can leak. Idempotent; an already-absent entry is fine.
        self.gate_waiters.lock().unwrap().clear();

        // Deregister so a follow-up `ensure_driver_running` for this
        // feature knows to start a fresh driver instead of trusting a
        // (now-completed) registry entry.
        self.driver_registry.deregister(&self.f_id);
    }
}
