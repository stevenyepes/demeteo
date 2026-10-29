//! Restart reconciliation + bounded reboot-retry budget
//! (docs/REMOTE_EXECUTION.md M2.3).
//!
//! The engine's own restart reconciliation (`startup_watchdog` +
//! `resume_interrupted_features`, both already invoked unconditionally by
//! `build_core_context`) re-arms any interrupted feature's driver — that
//! machinery is reused verbatim, not reimplemented here. What's missing
//! without this module: the `runner_runs` mirror row for that feature
//! would stay stuck showing whatever status it had when the process
//! died, so `list_runs`/`get_status` would lie. This module re-attaches
//! each such run to its (already-resuming) feature and keeps polling.
//!
//! A run whose driver never got as far as `feature_start` (crashed before
//! any project/feature existed) has nothing for the engine to resume —
//! this module re-runs `execute_run` from scratch for those instead. Both go
//! through [`crate::run::spawn_run_driver`], so a `retry_step` that arrives
//! while the re-attached driver is still live attaches to it.

use crate::services::RunnerServices;
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::domain::reboot_budget::{
    left_unfinished_by_restart, restart_disposition, restart_interrupted_work, RestartDisposition,
    REBOOT_RETRY_BUDGET,
};
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::paths;
use demeteo_core::ports::db::GateRepository;
use demeteo_core::ports::runner_run::{RunnerRun, RunnerRunPort};
use std::sync::Arc;

/// Record what this restart means for `run`, and return its spec when a task
/// must be re-attached to it.
///
/// `process_started_at` must have been read before `build_core_context` ran:
/// the engine's reconciliation in there poses its own gate rows, and
/// [`restart_interrupted_work`] tells those from a question the run was
/// already waiting on by which side of that reading they fall.
pub(crate) fn settle_restart(
    runner_runs: &dyn RunnerRunPort,
    gates: &dyn GateRepository,
    run: &RunnerRun,
    process_started_at: i64,
    now: i64,
) -> Option<RunSpec> {
    let pending_question_asked_at = run.feature_id.as_ref().and_then(|feature_id| {
        gates
            .pending_for_feature(&FeatureId::from(feature_id.clone()))
            .ok()
            .flatten()
            .map(|gate| gate.created_at)
    });
    let disposition = restart_disposition(
        run.resume_count,
        restart_interrupted_work(pending_question_asked_at, process_started_at),
    );

    if disposition == RestartDisposition::Park {
        eprintln!(
            "[demeteo-runner] run {} exceeded reboot-retry budget ({}); parking as failed (unstable host)",
            run.run_id, REBOOT_RETRY_BUDGET
        );
        let _ = runner_runs.update_status(
            &run.run_id,
            "failed",
            None,
            None,
            Some("unstable host: exceeded reboot-retry budget"),
            None,
            now,
        );
        return None;
    }

    let spec: RunSpec = match serde_json::from_str(&run.spec_json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[demeteo-runner] run {} has an unparseable spec, marking failed: {}",
                run.run_id, e
            );
            let _ = runner_runs.update_status(
                &run.run_id,
                "failed",
                None,
                None,
                Some(&format!("unparseable spec on resume: {}", e)),
                None,
                now,
            );
            return None;
        }
    };

    if disposition == RestartDisposition::ResumeCharged {
        match runner_runs.bump_resume_count(&run.run_id) {
            Ok(attempt) => eprintln!(
                "[demeteo-runner] resuming run {} after restart (attempt {}/{})",
                run.run_id, attempt, REBOOT_RETRY_BUDGET
            ),
            Err(e) => {
                eprintln!(
                    "[demeteo-runner] run {}: failed to bump resume count: {}",
                    run.run_id, e
                );
                return None;
            }
        }
    } else {
        eprintln!(
            "[demeteo-runner] re-attaching run {} after restart (it was waiting on a person; not charged)",
            run.run_id
        );
    }
    let _ = runner_runs.update_status(&run.run_id, "running", None, None, None, None, now);
    Some(spec)
}

pub async fn reconcile_on_startup(svc: Arc<RunnerServices>, process_started_at: i64) {
    let ctx = &svc.ctx;
    let runs = match ctx.runner_runs.list() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[demeteo-runner] reconcile: failed to list runs: {}", e);
            return;
        }
    };

    for run in runs {
        if !left_unfinished_by_restart(&run.status) {
            continue;
        }
        let Some(spec) = settle_restart(
            ctx.runner_runs.as_ref(),
            ctx.gates.as_ref(),
            &run,
            process_started_at,
            paths::now_ms(),
        ) else {
            continue;
        };

        // The engine already re-armed any existing feature's driver; a run
        // that died before `feature_start` has nothing to resume and runs from
        // scratch. `settle_restart` already wrote `running`, so this is not a
        // re-open.
        crate::run::spawn_run_driver(&svc, run.run_id.clone(), spec);
    }
}

#[cfg(test)]
#[path = "reconcile_tests.rs"]
mod tests;
