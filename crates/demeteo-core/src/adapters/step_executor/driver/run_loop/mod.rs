//! The `ExecutionDriver::run` main loop, decomposed by concern.
//!
//! Each submodule owns one slice:
//! - `dispatch` — single-step dispatch via NodeTypeRegistry
//! - `outcome` — handling each StepOutcome variant (success / failure /
//!   redirect / cancel)
//! - `attempt` — opening/closing the per-dispatch step_attempts row
//! - `cleanup` — post-loop terminal cleanup (cancel sessions, deregister,
//!   publish)
//!
//! `run(driver)` is the free function the executor spawn-tail calls into
//! (via `driver.rs::ExecutionDriver::run`). All non-trivial logic lives
//! in the submodules; this file is the orchestrator.

use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::updates;

mod attempt;
mod cleanup;
mod dispatch;
mod outcome;

use outcome::RunAction;

/// The 600-line original `ExecutionDriver::run` body, decomposed. Each
/// line that called a helper in `driver/failure.rs`, `driver/watchdog.rs`,
/// `driver/resolution.rs`, etc. is preserved — this is a structural
/// refactor, not a behavior change. `driver.rs::run` is a thin wrapper
/// that calls this function.
pub(crate) async fn run(mut driver: ExecutionDriver) {
    if let Ok(step_execs) = driver.features.steps_for_feature(&driver.f_id) {
        for s in &step_execs {
            if s.status == "completed" {
                driver.step_index = s.step_index as usize + 1;
            } else {
                break;
            }
        }
    }

    loop {
        if *driver.cancel_watch.borrow() {
            driver.cancel_feature().await;
            return;
        }

        let step_execs = match driver.features.steps_for_feature(&driver.f_id) {
            Ok(list) => list,
            Err(_) => break,
        };

        if driver.step_index >= step_execs.len() {
            break;
        }

        let step_exec = &step_execs[driver.step_index];
        // Clone `step_conf` so it doesn't borrow `driver.steps` —
        // `handle_gate_step` now takes `&mut self` (it sets
        // `retry_ctx` on a redirect with feedback), and the borrow
        // checker won't let us hold an immutable borrow across
        // that call.
        let step_conf = match driver.steps.iter().find(|s| s.id == step_exec.step_id) {
            Some(sc) => sc.clone(),
            None => break,
        };

        // Refresh the watchdog's model + context-window budget for
        // this step. Resolved before dispatch so a per-step model
        // override takes effect immediately and the next post-step
        // `maybe_watchdog_reset` compares against the correct
        // ceiling.
        {
            let (_agent, model) = driver.resolve_step_agent(&step_conf);
            let effort = driver.resolve_step_effort(&step_conf);
            driver.refresh_watchdog_budget(&step_conf, model.as_deref(), effort);
        }

        tracing::info!(
            feature_id = %driver.f_id,
            step_id = %step_exec.step_id.0,
            step_kind = %step_conf.kind,
            "step start"
        );

        // A resumed run can still read `awaiting_gate`/`gated` here; drop
        // it back to `running` while we drive this step. Gate steps set
        // their own status, so leave those to `handle_gate_step`.
        if step_conf.kind != "gate" {
            driver.ensure_feature_running();
        }

        updates::update_step_status(
            &*driver.features,
            &*driver.notif,
            step_exec,
            &driver.f_id,
            "running",
            step_exec.cost_usd.unwrap_or(0.0),
            step_exec.tokens,
            step_exec.wall_clock_secs.unwrap_or(0),
            None,
            None,
            None,
            None,
        );

        let step_start = Instant::now();
        let mut accumulated_cost = step_exec.cost_usd.unwrap_or(0.0);
        let mut accumulated_tokens = step_exec.tokens.unwrap_or(0);
        let mut step_cache_read: Option<u64> = None;
        let mut step_cache_creation: Option<u64> = None;

        let Some(dr) = driver
            .dispatch_step(
                step_exec,
                &step_conf,
                &step_execs,
                driver.step_index,
                step_start,
                &mut accumulated_cost,
                &mut accumulated_tokens,
                &mut step_cache_read,
                &mut step_cache_creation,
            )
            .await
        else {
            return;
        };

        let action = driver.apply_outcome(step_exec, &dr).await;

        match action {
            RunAction::Continue => continue,
            RunAction::RedirectTo(idx) => {
                driver.step_index = idx;
            }
            RunAction::Terminate => return,
        }
    }

    driver.finalize_run().await;
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/driver_run_loop.rs"]
mod run_loop_tests;
