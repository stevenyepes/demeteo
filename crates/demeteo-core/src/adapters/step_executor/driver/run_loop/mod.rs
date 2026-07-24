//! The `ExecutionDriver::run` main loop, decomposed by concern.
//!
//! Each submodule owns one slice:
//! - `schedule` — DB-row → `NodeState` derivation, skip persistence,
//!   redirect rewinds (the P1.12 scheduler glue)
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
//!
//! # The ready-set walk (P1.12)
//!
//! The v1 loop held a `step_index` cursor and did `step_index += 1`
//! (redirects jumped it backwards). This loop instead re-derives the
//! scheduler's node-state view from the persisted `step_executions` rows
//! each iteration (`schedule::derive_states`) and asks the pure ready-set
//! scheduler (P1.11) what may run:
//!
//! 1. **Skips are persisted first** — durable before anything acts on
//!    them, then the loop re-evaluates (a skip can cascade).
//! 2. **One node dispatches per iteration** (max_parallel_nodes = 1;
//!    the P4.1 write-scope work raises it): the first ready node in
//!    topological order, which for a migrated v1 chain is exactly the
//!    step the old cursor would have picked.
//! 3. **An empty ready set with no pending nodes** is normal completion.
//!    A scheduler [`ScheduleError`] (deadlock, unknown node) is a bug the
//!    graph lint should have prevented — the run fails loudly instead of
//!    idling forever (PRD §5.3 step 4).
//!
//! Every transition is written through the repositories *before* the loop
//! acts on it and the matching event fires after the write — the same
//! durable-first ordering `updates::update_step_status` has always
//! enforced per row. (A single multi-row SQLite transaction per tick
//! needs a transactional port seam the repos don't expose yet; each
//! per-row write is atomic, and the loop tolerates re-observing any
//! prefix of them after a crash.)

use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::scheduler::{evaluate_ready_set, ScheduleError};
use crate::adapters::step_executor::updates;
use crate::domain::expr::ExprValue;

mod attempt;
mod cleanup;
mod dispatch;
mod outcome;
mod resume;
pub(crate) mod schedule;

use outcome::RunAction;

/// Drive the feature by walking the scheduler's ready set until every
/// node is terminal or a step outcome terminates the run.
pub(crate) async fn run(mut driver: ExecutionDriver) {
    // Edge guards (`when`) don't exist on migrated v1 definitions and
    // node outputs aren't published yet (P1.13 grows the vocabulary), so
    // the resolver knows no values — a guard authored anyway fails
    // closed (unsatisfiable edge), never silently passes.
    let resolve_no_outputs = |_node: &str, _output: &str| -> Option<ExprValue> { None };

    loop {
        if *driver.cancel_watch.borrow() {
            driver.cancel_feature().await;
            return;
        }

        let step_execs = match driver.features.steps_for_feature(&driver.f_id) {
            Ok(list) => list,
            Err(_) => break,
        };

        let states = schedule::derive_states(&driver.graph, &step_execs);
        let ready_set =
            match evaluate_ready_set(&driver.def_v2, &driver.graph, &states, &resolve_no_outputs) {
                Ok(rs) => rs,
                Err(e) => {
                    fail_unschedulable(&driver, &step_execs, &e).await;
                    return;
                }
            };

        // Persist scheduler-decided skips before acting, then re-derive:
        // a skip changes the state view the next evaluation reads.
        if !ready_set.skip.is_empty() {
            for (node_id, reason) in &ready_set.skip {
                tracing::info!(
                    feature_id = %driver.f_id,
                    step_id = %node_id.0,
                    reason = %reason,
                    "step skipped"
                );
                driver.persist_skip(&step_execs, node_id, reason);
            }
            continue;
        }

        // Nothing ready and nothing skipped: every node is terminal (a
        // stuck graph surfaced as ScheduleError above) — the run is done.
        // Taking only the *first* ready node is the max_parallel_nodes = 1
        // ceiling (PRD §5.6); P4.1 owns dispatching more per tick.
        let Some(node_id) = ready_set.ready.first().cloned() else {
            break;
        };

        // Resolve the ready node back to its config + execution row. Both
        // come from the same seeded step list the graph was built from, so
        // a miss is driver bookkeeping corruption — fail loudly.
        let Some(step_index) = driver.steps.iter().position(|s| s.id == node_id) else {
            fail_unschedulable(
                &driver,
                &step_execs,
                &ScheduleError::UnknownNode(node_id.to_string()),
            )
            .await;
            return;
        };
        let step_conf = driver.steps[step_index].clone();
        let Some(step_exec) = step_execs.iter().find(|s| s.step_id == node_id) else {
            fail_unschedulable(
                &driver,
                &step_execs,
                &ScheduleError::UnknownNode(node_id.to_string()),
            )
            .await;
            return;
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

        // Resume fingerprint guard (P1.14): a node the watchdog marked
        // `interrupted` is only re-dispatched blindly when the workspace
        // still matches what its interrupted attempt started from; a
        // mismatch parks on the synthetic gate the watchdog surfaced
        // (Decision 14). Once per driver life — an approval is a human
        // blessing for the rest of this life.
        if step_exec.status == "interrupted" && !driver.resume_guard_done {
            driver.resume_guard_done = true;
            match driver.resume_fingerprint_guard(step_exec).await {
                resume::GuardVerdict::Proceed => {}
                resume::GuardVerdict::Cancelled => {
                    driver.cancel_feature().await;
                    return;
                }
                resume::GuardVerdict::Rejected(msg) => {
                    driver
                        .fail_step_and_feature(
                            step_exec,
                            &msg,
                            step_exec.cost_usd.unwrap_or(0.0),
                            step_exec.tokens.unwrap_or(0),
                            Instant::now(),
                        )
                        .await;
                    return;
                }
            }
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
                step_index,
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
                // Rewind the redirect target and its downstream cone to
                // `pending` (durable before the next evaluation) so the
                // ready set re-schedules from the target — the DAG form
                // of the v1 cursor jump.
                match driver.steps.get(idx) {
                    Some(target) => {
                        let target_id = target.id.clone();
                        driver.reset_for_redirect(&step_execs, &target_id);
                    }
                    None => tracing::error!(
                        feature_id = %driver.f_id,
                        redirect_index = idx,
                        "redirect target index out of bounds; re-evaluating without rewind"
                    ),
                }
            }
            RunAction::Terminate => return,
        }
    }

    driver.finalize_run().await;
}

/// The scheduler refused to schedule (deadlocked join, unknown node) —
/// a structural bug the lint should have caught, never a wait state.
/// Mark the stuck rows and the feature failed so the run surfaces the
/// invariant violation instead of idling forever.
async fn fail_unschedulable(
    driver: &ExecutionDriver,
    step_execs: &[crate::domain::models::StepExecution],
    err: &ScheduleError,
) {
    let msg = format!("workflow cannot advance: {err}");
    tracing::error!(feature_id = %driver.f_id, error = %msg, "run unschedulable");
    if let ScheduleError::Deadlock(stuck) = err {
        for node_id in stuck {
            if let Some(row) = step_execs.iter().find(|s| s.step_id == *node_id) {
                updates::update_step_status(
                    &*driver.features,
                    &*driver.notif,
                    row,
                    &driver.f_id,
                    "failed",
                    row.cost_usd.unwrap_or(0.0),
                    row.tokens,
                    row.wall_clock_secs.unwrap_or(0),
                    None,
                    Some(msg.clone()),
                    None,
                    None,
                );
            }
        }
    }
    updates::finish_feature(
        &*driver.features,
        &*driver.notif,
        &driver.f_id,
        "failed",
        driver.start_time,
    );
    // Session sweep only — the spawn-tail's `deregister_guard` owns the
    // registry entry on every exit path, including this one.
    driver
        .registry
        .kill_all_for_feature(driver.f_id.as_str())
        .await;
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/driver_run_loop.rs"]
mod run_loop_tests;
