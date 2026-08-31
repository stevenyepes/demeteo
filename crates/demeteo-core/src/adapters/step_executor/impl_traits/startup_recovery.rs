//! Bringing a fresh process back to a state the previous one can be resumed
//! from: reconcile the rows a dead process abandoned, then re-arm the drivers.
//!
//! Two passes over the same feature set, deliberately apart. The first is
//! synchronous and writes only — it can run before the runtime hands control to
//! user-driven tasks. The second spawns drivers and must not. What each
//! abandoned row *becomes* is not decided here: that is
//! [`crate::domain::restart_reconcile`].

use std::sync::Arc;

use crate::domain::models::GateDecision;
use crate::domain::restart_reconcile::{
    abandoned_out_of_band, interrupted_by_restart, orphaned_by_feature_end,
};
use crate::paths;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::DomainEvent;

use super::super::DagStepExecutor;

impl DagStepExecutor {
    /// Reconcile DB + notifications for any features that were left
    /// mid-run by a previous process. Synchronous (no driver spawns) so
    /// it can be called from the Tauri setup hook before the runtime
    /// hands control to user-driven tasks. Pair with
    /// [`resume_interrupted_features`](Self::resume_interrupted_features)
    /// which spawns the actual drivers.
    ///
    /// Features present in the remote-run mirror (C4.2) are skipped:
    /// those rows are read-only *shadows* of features a `demeteo-runner`
    /// owns and is still driving on another machine. A shadow tracking a
    /// live remote run legitimately sits in `running`/`gated` across an
    /// app restart — no local process was ever driving it — so the
    /// watchdog must not mark its steps interrupted or re-emit gate
    /// prompts for it.
    pub fn startup_watchdog(&self) {
        let runner_owned = self.runner_owned_features();
        let Ok(projects) = self.projects.get_projects() else {
            return;
        };
        for p in &projects {
            if let Ok(active) = self.features.get_active(&p.id) {
                for f in active {
                    if runner_owned.contains(f.id.as_str()) {
                        continue;
                    }
                    if f.status == "running" || f.status == "gated" {
                        let _ = self.projects.update_status(&p.id, "idle");
                        if let Ok(steps) = self.features.steps_for_feature(&f.id) {
                            for s in steps {
                                let Some(reconciled) = interrupted_by_restart(&s.status) else {
                                    continue;
                                };
                                // The step is being marked interrupted, so any
                                // `running` subtask_runs row of its sequence
                                // task loop is stale — close it, or the
                                // dashboard's "nodes" count (which counts
                                // running rows) over-reports forever.
                                if let Err(e) = self
                                    .subtask_runs
                                    .subtask_runs_interrupt_stale(&s.id, paths::now_ms())
                                {
                                    tracing::warn!(
                                        step_execution_id = %s.id.0,
                                        error = %e,
                                        "startup watchdog: could not close stale subtask_runs rows"
                                    );
                                }
                                let _ = self.features.step_update(
                                    &s.id,
                                    &StepExecutionPatch {
                                        last_failure_fingerprint: None,
                                        status: Some("interrupted".to_string()),
                                        cost_usd: s.cost_usd.map(Some),
                                        wall_clock_secs: s.wall_clock_secs.map(Some),
                                        artifact_path: s
                                            .artifact_path
                                            .as_deref()
                                            .map(|v| Some(v.to_string())),
                                        artifact_paths: Some(s.artifact_paths.clone()),
                                        error_message: reconciled.message.map(Some),
                                        ..Default::default()
                                    },
                                );
                                if reconciled.synthesise_gate_decision {
                                    let gate_dec_id = crate::domain::ids::GateDecisionId::from(
                                        format!("gd-syn-{}", s.id.0),
                                    );
                                    let gate_dec = GateDecision {
                                        id: gate_dec_id,
                                        step_execution_id: s.id.clone(),
                                        decision: None,
                                        feedback: None,
                                        created_at: paths::now_ms(),
                                    };
                                    let _ = self.gates.create(gate_dec);
                                }
                                let _ = self.notif.emit(&DomainEvent::GateRequired {
                                    feature_id: f.id.clone(),
                                    step_execution_id: s.id.clone(),
                                });
                            }
                            let _ = self.features.update(
                                &f.id,
                                &FeaturePatch {
                                    status: Some("awaiting_gate".to_string()),
                                    ..Default::default()
                                },
                            );
                            let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
                                feature_id: f.id.clone(),
                                status: "awaiting_gate".into(),
                            });
                        }
                    }
                }
            }
        }

        // Second pass: rows no run will ever move again. Unlike the first it
        // reads every feature, because the out-of-band sync row is only ever
        // found under one that has already finished.
        for p in &projects {
            if let Ok(all_features) = self.features.get_active(&p.id) {
                for f in all_features {
                    if runner_owned.contains(f.id.as_str()) {
                        // A shadow's pending steps mirror the runner's own
                        // rows mid-hydration — not orphans of a local crash.
                        continue;
                    }
                    if let Ok(steps) = self.features.steps_for_feature(&f.id) {
                        for s in &steps {
                            let Some(message) = orphaned_by_feature_end(&f.status, &s.status)
                                .or_else(|| abandoned_out_of_band(&s.step_id.0, &s.status))
                            else {
                                continue;
                            };
                            let _ = self.features.step_update(
                                &s.id,
                                &StepExecutionPatch {
                                    last_failure_fingerprint: None,
                                    status: Some("interrupted".to_string()),
                                    error_message: Some(Some(message)),
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Resume every feature that [`startup_watchdog`](Self::startup_watchdog)
    /// marked as `awaiting_gate`. Idempotent via
    /// [`DriverRegistry`](crate::adapters::step_executor::driver_registry::DriverRegistry):
    /// if the runtime already has a driver alive for a feature, it's a no-op.
    ///
    /// Called once from the Tauri setup hook on a background task so
    /// that the gate prompts the watchdog re-emitted are actually
    /// backed by a live driver.
    ///
    /// Mirror-listed shadows are skipped (same rule as
    /// [`startup_watchdog`](Self::startup_watchdog)): a shadow in
    /// `awaiting_gate`/`gated` is parked on the *runner*, not here — arming a
    /// local driver against it would have two engines driving one feature.
    pub async fn resume_interrupted_features(self: Arc<Self>) {
        let runner_owned = self.runner_owned_features();
        let Ok(projects) = self.projects.get_projects() else {
            return;
        };
        for p in projects {
            let Ok(active) = self.features.get_active(&p.id) else {
                continue;
            };
            for f in active {
                if runner_owned.contains(f.id.as_str()) {
                    continue;
                }
                if f.status == "awaiting_gate" || f.status == "gated" {
                    if let Err(e) = self.ensure_driver_running(&f.id.0).await {
                        eprintln!(
                            "resume_interrupted_features: failed to resume {}: {}",
                            f.id.0, e
                        );
                    }
                }
            }
        }
    }
}
