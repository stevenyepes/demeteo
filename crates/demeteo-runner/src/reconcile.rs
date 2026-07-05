//! Restart reconciliation + bounded reboot-retry budget
//! (docs/REMOTE_EXECUTION_PLAN.md M2.3).
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
//! this module re-runs `execute_run` from scratch for those instead.

use crate::services::RunnerServices;
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::paths;
use std::sync::Arc;

/// How many times a run may be auto-resumed after a restart before it's
/// parked as `failed` instead of resumed again — the guard against a
/// crash-looping host resuming (and re-spending agent turns on) the same
/// run forever.
const REBOOT_RETRY_BUDGET: i64 = 5;

pub async fn reconcile_on_startup(svc: Arc<RunnerServices>) {
    let ctx = &svc.ctx;
    let runs = match ctx.runner_runs.list() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[demeteo-runner] reconcile: failed to list runs: {}", e);
            return;
        }
    };

    for run in runs {
        if !matches!(run.status.as_str(), "running" | "pending" | "interrupted") {
            continue;
        }

        let now = paths::now_ms();
        if run.resume_count >= REBOOT_RETRY_BUDGET {
            eprintln!(
                "[demeteo-runner] run {} exceeded reboot-retry budget ({}); parking as failed (unstable host)",
                run.run_id, REBOOT_RETRY_BUDGET
            );
            let _ = ctx.runner_runs.update_status(
                &run.run_id,
                "failed",
                None,
                None,
                Some("unstable host: exceeded reboot-retry budget"),
                None,
                now,
            );
            continue;
        }

        let spec: RunSpec = match serde_json::from_str(&run.spec_json) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[demeteo-runner] run {} has an unparseable spec, marking failed: {}",
                    run.run_id, e
                );
                let _ = ctx.runner_runs.update_status(
                    &run.run_id,
                    "failed",
                    None,
                    None,
                    Some(&format!("unparseable spec on resume: {}", e)),
                    None,
                    now,
                );
                continue;
            }
        };

        let attempt = match ctx.runner_runs.bump_resume_count(&run.run_id) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[demeteo-runner] run {}: failed to bump resume count: {}",
                    run.run_id, e
                );
                continue;
            }
        };
        let _ = ctx
            .runner_runs
            .update_status(&run.run_id, "running", None, None, None, None, now);
        eprintln!(
            "[demeteo-runner] resuming run {} after restart (attempt {}/{})",
            run.run_id, attempt, REBOOT_RETRY_BUDGET
        );

        let svc_bg = svc.clone();
        let run_id_bg = run.run_id.clone();
        let project_id = run.project_id.clone();
        let feature_id = run.feature_id.clone();
        tokio::spawn(async move {
            // The engine already re-armed any existing feature's driver
            // (project_id/feature_id both set); a run that died before
            // `feature_start` has nothing to resume and runs from scratch.
            let result =
                crate::run::resume_or_run(&svc_bg, &run_id_bg, &spec, project_id, feature_id).await;
            let now = paths::now_ms();
            match result {
                Ok(outcome) => {
                    let _ = svc_bg.ctx.runner_runs.update_status(
                        &run_id_bg,
                        &outcome.status,
                        outcome.project_id.as_deref(),
                        outcome.feature_id.as_deref(),
                        None,
                        outcome.pushed_branch.as_deref(),
                        now,
                    );
                }
                Err(e) => {
                    let _ = svc_bg.ctx.runner_runs.update_status(
                        &run_id_bg,
                        "failed",
                        None,
                        None,
                        Some(&e),
                        None,
                        now,
                    );
                }
            }
        });
    }
}
