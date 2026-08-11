//! Terminal cleanup after the run loop exits.
//!
//! Called by `mod.rs::run()` when the per-step loop ends without
//! `RunAction::Terminate` — every step done, step_index past the end.
//! Decides the terminal feature status (completed vs awaiting_mr) and
//! finishes the feature row, then sweeps this feature's live sessions and
//! gate waiters, and finally deregisters from the live-driver registry.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::gate_waiter::sweep_feature;
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

        // Drop any stale gate waiter *this feature* left behind — the loop
        // above consumes them on success, but cancellation / failure paths
        // can leak. Idempotent; an already-absent entry is fine.
        //
        // Scoped, never `clear()`: the map is one process-global registry
        // shared by every live driver, so a wipe here unregisters whatever
        // *other* run is parked at a gate right now. That is not a lost
        // notification; it is a run no click can ever finish — the driver
        // stays alive, so `gate_decide` records the decision, finds no
        // waiter, and `ensure_driver_running` declines to spawn a
        // replacement for a driver that has not died.
        sweep_feature(&self.gate_waiters, &self.f_id);

        // Deregister so a follow-up `ensure_driver_running` for this
        // feature knows to start a fresh driver instead of trusting a
        // (now-completed) registry entry.
        self.driver_registry.deregister(&self.f_id);
    }
}
