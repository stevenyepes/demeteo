//! Apply a `StepOutcome` to driver state and the persisted feature row.
//!
//! The loop in `mod.rs` is a thin shell; this module owns the per-variant
//! decision-making: which status row to write, which `on_failure` policy
//! path to walk, which sessions to keep alive, and how the run loop
//! should continue. Returns [`RunAction`] to tell the orchestrator whether
//! to iterate, jump, or exit.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::retry_policy::RetryAction;
use crate::adapters::step_executor::step_status::{update_step_status, StepTransition};
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::models::StepExecution;
use crate::domain::verifier::VerdictFailure;

/// What the orchestrator in `mod.rs` should do after the outcome has
/// been applied to driver state and the database.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunAction {
    /// Continue to the next iteration; the next ready-set evaluation
    /// reads the persisted statuses the outcome handler just wrote (a
    /// `Completed` step's successor becomes ready; an in-place retry
    /// parked its own row back at `pending`).
    Continue,
    /// A `Redirect` retry policy or a `RedirectTo(idx)` step result
    /// wants the run rewound to `driver.steps[idx]`. The orchestrator
    /// resets that node and its descendants to `pending`
    /// (`schedule::reset_for_redirect`) before re-evaluating.
    RedirectTo(usize),
    /// The feature is in a terminal state. The orchestrator returns
    /// from `run()`.
    Terminate,
}

impl ExecutionDriver {
    /// The top-level dispatcher — a thin `match` over `StepOutcome` that
    /// fans out to one `apply_*` per variant. Kept here so the per-variant
    /// helpers below can stay short and side-effect-scoped.
    pub(crate) async fn apply_outcome(
        &mut self,
        step_exec: &StepExecution,
        dr: &super::dispatch::DispatchResult,
    ) -> RunAction {
        match &dr.outcome {
            StepOutcome::Completed => self.apply_completed(step_exec, dr).await,
            StepOutcome::Failed(msg) => {
                let vf = dr.verdict_failure.as_ref();
                self.apply_failed(step_exec, msg, vf, dr).await
            }
            StepOutcome::VerdictFailed(_) => {
                unreachable!("VerdictFailed is normalized into Failed above")
            }
            StepOutcome::Environmental(msg) => self.apply_environmental(step_exec, msg, dr).await,
            StepOutcome::NonRetryable(msg) => self.apply_non_retryable(step_exec, msg, dr).await,
            StepOutcome::Cancelled => self.apply_cancelled().await,
            StepOutcome::RedirectTo(idx) => self.apply_redirect(*idx),
        }
    }

    async fn apply_completed(
        &mut self,
        step_exec: &StepExecution,
        dr: &super::dispatch::DispatchResult,
    ) -> RunAction {
        let wall = dr.step_start.elapsed().as_secs();
        tracing::info!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            wall_secs = wall,
            cost_usd = dr.accumulated_cost,
            "step completed"
        );
        let latest_step = self.features.step_get(&step_exec.id).ok().flatten();
        let art_path = latest_step.as_ref().and_then(|s| s.artifact_path.clone());
        update_step_status(
            self.status_writers(),
            step_exec,
            &self.f_id,
            StepTransition::completed(
                dr.accumulated_cost,
                dr.accumulated_tokens,
                wall,
                art_path,
                self.cache_tokens(),
            ),
        );
        // Context-window watchdog: pull the live session's
        // cumulative tokens and decide whether to reset
        // the agent session before the next step starts.
        // On reset, `session_dirty = true` so the next
        // `spawn_agent_session` falls back to fresh spawn
        // + `session_resume_summary` injection.
        self.maybe_watchdog_reset().await;
        // Retry feedback lives until the step that originally
        // failed completes successfully. Intermediate steps (the
        // redirect target and everything between it and the
        // failing step) all see the feedback; once the failing
        // step passes, the loop is closed and the feedback is
        // stale.
        let loop_closed = self.retry_ctx.as_ref().is_none_or(|rc| {
            rc.failing_step_id.is_empty() || rc.failing_step_id == step_exec.step_id.0
        });
        if loop_closed {
            self.retry_ctx = None;
        }
        RunAction::Continue
    }

    async fn apply_failed(
        &mut self,
        step_exec: &StepExecution,
        msg: &str,
        verdict_failure: Option<&VerdictFailure>,
        dr: &super::dispatch::DispatchResult,
    ) -> RunAction {
        tracing::warn!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            reason = %msg,
            "step failed"
        );
        let is_cancelled = *self.cancel_watch.borrow();
        if is_cancelled {
            let wall = dr.step_start.elapsed().as_secs();
            update_step_status(
                self.status_writers(),
                step_exec,
                &self.f_id,
                StepTransition::interrupted(
                    dr.accumulated_cost,
                    dr.accumulated_tokens,
                    wall,
                    format!("Cancelled while step was failing: {}", msg),
                    self.cache_tokens(),
                ),
            );
            self.cancel_feature().await;
            return RunAction::Terminate;
        }
        // Act on the policy decision evaluated above (P1.10).
        let decision = dr
            .failure_decision
            .clone()
            .expect("a non-cancelled Failed outcome evaluates a retry decision");
        self.emit_retry_decision(step_exec, &decision, msg);
        match decision.action {
            RetryAction::Redirect { target, feedback } => {
                if let Some(redirect_idx) = self.begin_redirect(
                    step_exec,
                    &target,
                    msg,
                    decision.attempt,
                    decision.max_attempts,
                    dr.accumulated_cost,
                    dr.accumulated_tokens,
                    dr.step_start,
                ) {
                    // Capture the failure so the retried step's
                    // prompt isn't blind. `iteration_count` was
                    // just bumped to `decision.attempt` in
                    // begin_redirect — the attempt now starting.
                    self.capture_signal(
                        Some(step_exec.id.0.clone()),
                        crate::domain::memory::SignalKind::Retry,
                        format!(
                            "Step '{}' failed (attempt {} of {}), retrying: {}",
                            step_exec.step_id.0, decision.attempt, decision.max_attempts, msg
                        ),
                    );
                    if feedback {
                        self.retry_ctx =
                            Some(crate::adapters::step_executor::driver::RetryContext {
                                feedback: msg.to_string(),
                                iteration: decision.attempt,
                                max: decision.max_attempts,
                                failing_step_id: step_exec.step_id.0.clone(),
                                failing_tests: verdict_failure
                                    .map(|vf| vf.failing_tests.clone())
                                    .unwrap_or_default(),
                                implicated_files: verdict_failure
                                    .map(|vf| vf.implicated_files.clone())
                                    .unwrap_or_default(),
                            });
                    }
                    return RunAction::RedirectTo(redirect_idx);
                }
                // Dangling redirect target — same terminal
                // failure as v1's missing-`on_failure`-step.
                self.fail_step_and_feature(
                    step_exec,
                    msg,
                    dr.accumulated_cost,
                    dr.accumulated_tokens,
                    dr.step_start,
                )
                .await;
            }
            RetryAction::Exhausted { target } => {
                if let Some(target) = target.as_ref() {
                    self.record_retry_exhausted(
                        step_exec,
                        target,
                        msg,
                        decision.attempt.saturating_sub(1),
                        decision.max_attempts,
                        dr.accumulated_cost,
                        dr.accumulated_tokens,
                        dr.step_start,
                    );
                }
                self.fail_step_and_feature(
                    step_exec,
                    msg,
                    dr.accumulated_cost,
                    dr.accumulated_tokens,
                    dr.step_start,
                )
                .await;
            }
            RetryAction::RetryInPlace { .. } => {
                // Not derivable from v1 definitions for this
                // class; supported for v2 policies (P1.12).
                self.begin_in_place_retry(
                    step_exec,
                    msg,
                    dr.accumulated_cost,
                    dr.accumulated_tokens,
                    dr.step_start,
                );
                return RunAction::Continue;
            }
            RetryAction::Fail => {
                self.fail_step_and_feature(
                    step_exec,
                    msg,
                    dr.accumulated_cost,
                    dr.accumulated_tokens,
                    dr.step_start,
                )
                .await;
            }
        }
        RunAction::Terminate
    }

    async fn apply_environmental(
        &mut self,
        step_exec: &StepExecution,
        msg: &str,
        dr: &super::dispatch::DispatchResult,
    ) -> RunAction {
        tracing::warn!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            reason = %msg,
            "step failed (environmental)"
        );
        let is_cancelled = *self.cancel_watch.borrow();
        if is_cancelled {
            self.cancel_feature().await;
            return RunAction::Terminate;
        }
        // The environment broke — a timeout, a dead process, a
        // worktree that wouldn't provision. Redirecting to an
        // implementation step can't fix any of that, and burning
        // the redirect budget on it starves real retries. The
        // policy's environment rule (one free in-place retry,
        // budget derived from the durable V31 attempt history —
        // a restart no longer grants a fresh one) was evaluated
        // above; a spent budget fails the feature with a message
        // that names the environment, not the code.
        let decision = dr
            .failure_decision
            .as_ref()
            .expect("a non-cancelled Environmental outcome evaluates a retry decision");
        self.emit_retry_decision(step_exec, decision, msg);
        if matches!(decision.action, RetryAction::RetryInPlace { .. }) {
            self.begin_in_place_retry(
                step_exec,
                msg,
                dr.accumulated_cost,
                dr.accumulated_tokens,
                dr.step_start,
            );
            // Same step_index — the loop re-dispatches this step.
            return RunAction::Continue;
        }
        self.fail_step_and_feature(
            step_exec,
            &format!("[environment — not an implementation failure] {}", msg),
            dr.accumulated_cost,
            dr.accumulated_tokens,
            dr.step_start,
        )
        .await;
        RunAction::Terminate
    }

    async fn apply_non_retryable(
        &mut self,
        step_exec: &StepExecution,
        msg: &str,
        dr: &super::dispatch::DispatchResult,
    ) -> RunAction {
        tracing::warn!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            reason = %msg,
            "step failed (non-retryable)"
        );
        if let Some(decision) = dr.failure_decision.as_ref() {
            self.emit_retry_decision(step_exec, decision, msg);
        }
        self.fail_step_and_feature(
            step_exec,
            msg,
            dr.accumulated_cost,
            dr.accumulated_tokens,
            dr.step_start,
        )
        .await;
        RunAction::Terminate
    }

    async fn apply_cancelled(&self) -> RunAction {
        self.cancel_feature().await;
        RunAction::Terminate
    }

    fn apply_redirect(&self, idx: usize) -> RunAction {
        RunAction::RedirectTo(idx)
    }
}
