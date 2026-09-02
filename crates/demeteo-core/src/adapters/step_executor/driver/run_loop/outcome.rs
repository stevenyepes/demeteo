//! Apply a `StepOutcome` to driver state and the persisted feature row.
//!
//! The loop in `mod.rs` is a thin shell; this module owns the per-variant
//! decision-making: which status row to write, which `on_failure` policy
//! path to walk, which sessions to keep alive, and how the run loop
//! should continue. Returns [`RunAction`] to tell the orchestrator whether
//! to iterate, jump, or exit.

use crate::adapters::step_executor::driver::failure::{
    begin_redirect, record_retry_exhausted, RetryBudget,
};
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
            StepOutcome::ProducerFault { .. } => {
                unreachable!("ProducerFault is normalized into Failed above")
            }
            StepOutcome::Environmental(msg) => self.apply_environmental(step_exec, msg, dr).await,
            StepOutcome::NonRetryable(msg) => self.apply_non_retryable(step_exec, msg, dr).await,
            StepOutcome::AwaitHumanDecision(park) => {
                self.apply_await_human(step_exec, park, dr).await
            }
            StepOutcome::Cancelled => self.apply_cancelled().await,
            StepOutcome::RedirectTo(idx) => self.apply_redirect(*idx),
        }
    }

    /// Park the step on the synthetic gate and act on the answer.
    ///
    /// Lives here rather than in the handler on purpose. By the time an
    /// outcome reaches this layer `close_attempt` has already run
    /// (`dispatch_step` closes the row before returning), so the human's
    /// thinking time lands on no attempt row — the obstacle that makes
    /// parking from inside a handler wrong. `wall_at_park` is captured for
    /// the same reason on the step row: `dr.step_start` keeps running
    /// while a person reads, and a step that ran for ninety seconds must
    /// not report four hours.
    ///
    /// **`Complete` does not re-dispatch the step.** Re-running it would
    /// re-read the same empty task list and park again — a loop with a
    /// human in it. Writing `completed` lets the scheduler advance, which
    /// is where `{{gate_decision_log}}` shows the validator the approval
    /// that was just given. That pairing is the actual fix: the park makes
    /// the question askable, and the log makes the answer count.
    async fn apply_await_human(
        &mut self,
        step_exec: &StepExecution,
        park: &crate::domain::step_park::HumanPark,
        dr: &super::dispatch::DispatchResult,
    ) -> RunAction {
        use crate::domain::step_park::{resolve_park, ParkResolution};

        // Cloned rather than borrowed: the park below takes `&mut self`
        // for hours, and `self.steps` cannot stay borrowed across it.
        let step_conf = self
            .steps
            .iter()
            .find(|s| s.id == step_exec.step_id)
            .cloned()
            .unwrap_or_default();
        let wall_at_park = dr.step_start.elapsed().as_secs();
        tracing::info!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            "step parked for a human decision"
        );
        update_step_status(
            self.status_writers(),
            step_exec,
            StepTransition::awaiting_gate(
                dr.accumulated_cost,
                dr.accumulated_tokens,
                wall_at_park,
                park.reason.clone(),
                self.cache_tokens(),
            ),
        );
        self.park_feature();
        // A park can last hours, and Anthropic's prompt cache expires in
        // minutes — the same reason `handle_gate_step` sweeps before it
        // waits. Resuming a session across the wait would replay the whole
        // transcript at full input price.
        self.registry.kill_all_for_feature(self.f_id.as_str()).await;

        let decision = crate::adapters::step_executor::gate_park::park_for_human(
            crate::adapters::step_executor::gate_park::SyntheticGate {
                gates: self.gates.as_ref(),
                notif: self.notif.as_ref(),
                waiters: &self.gate_waiters,
                f_id: &self.f_id,
            },
            &step_exec.id,
            self.cancel_watch.clone(),
        )
        .await;

        match resolve_park(park, decision.as_ref()) {
            ParkResolution::Complete => {
                self.ensure_feature_running();
                update_step_status(
                    self.status_writers(),
                    step_exec,
                    StepTransition::completed(
                        dr.accumulated_cost,
                        dr.accumulated_tokens,
                        wall_at_park,
                        None,
                        self.cache_tokens(),
                    ),
                );
                self.close_retry_loop_if_done(step_exec);
                RunAction::Continue
            }
            ParkResolution::Redirect { target, feedback } => {
                self.ensure_feature_running();
                // Budgeted like any other redirect. A human's answer is
                // better informed than a policy's, but an unbudgeted one
                // is still an unbounded loop — just with a slow oracle in
                // it — so exhaustion fails here exactly as it does there.
                let decision = self.retry_decision_for(
                    &step_conf,
                    crate::adapters::step_executor::retry_policy::FailureClass::Verdict,
                    step_exec.iteration_count,
                    Some(&target),
                );
                let budget = RetryBudget {
                    attempt: decision.attempt,
                    max: decision.max_attempts,
                };
                match self
                    .redirect_to_step(
                        step_exec,
                        &target,
                        &feedback,
                        budget,
                        Some((Vec::new(), Vec::new())),
                        dr,
                    )
                    .await
                {
                    Some(action) => action,
                    None => {
                        self.fail_step_and_feature(
                            step_exec,
                            &format!(
                                "redirect target '{}' is not a node of this workflow",
                                target.0
                            ),
                            dr.spend(self.cache_tokens()),
                        )
                        .await;
                        RunAction::Terminate
                    }
                }
            }
            ParkResolution::Fail(msg) => {
                self.fail_step_and_feature(step_exec, &msg, dr.spend(self.cache_tokens()))
                    .await;
                RunAction::Terminate
            }
            ParkResolution::Cancelled => {
                self.cancel_feature().await;
                RunAction::Terminate
            }
        }
    }

    /// Rewind to `target`, carrying `feedback` to whatever runs there.
    ///
    /// Shared by the retry policy's redirect and by a human's redirect from
    /// a park, because the two do the same thing and a second copy of the
    /// `RetryContext` construction is how they drift apart. `structured`
    /// carries a verdict's `(failing_tests, implicated_files)` when there is
    /// one and is `None` when the caller wants no feedback bound at all.
    ///
    /// `None` means the target does not resolve to a node — the caller owns
    /// the terminal failure, since what to say about it differs.
    async fn redirect_to_step(
        &mut self,
        step_exec: &StepExecution,
        target: &crate::domain::ids::StepId,
        feedback: &str,
        budget: RetryBudget,
        structured: Option<(Vec<String>, Vec<String>)>,
        dr: &super::dispatch::DispatchResult,
    ) -> Option<RunAction> {
        let redirect_idx = begin_redirect(
            self.status_writers(),
            &self.steps,
            step_exec,
            target,
            feedback,
            budget,
            dr.spend(self.cache_tokens()),
        )?;
        // Capture the failure so the retried step's prompt isn't blind.
        // `iteration_count` was just bumped to `budget.attempt` in
        // begin_redirect — the attempt now starting.
        self.capture_signal(
            Some(step_exec.id.0.clone()),
            crate::domain::memory::SignalKind::Retry,
            format!(
                "Step '{}' redirected to '{}' (attempt {} of {}): {}",
                step_exec.step_id.0, target.0, budget.attempt, budget.max, feedback
            ),
        );
        if let Some((failing_tests, implicated_files)) = structured {
            self.retry_ctx = Some(crate::adapters::step_executor::driver::RetryContext {
                feedback: feedback.to_string(),
                iteration: budget.attempt,
                max: budget.max,
                failing_step_id: step_exec.step_id.0.clone(),
                failing_tests,
                implicated_files,
            });
        }
        Some(RunAction::RedirectTo(redirect_idx))
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
        self.close_retry_loop_if_done(step_exec);
        RunAction::Continue
    }

    /// Drop the retry feedback once the step that opened the loop has
    /// succeeded.
    ///
    /// Retry feedback lives until the step that originally failed completes
    /// successfully. Intermediate steps — the redirect target and
    /// everything between it and the failing step — all see the feedback;
    /// once the failing step passes, the loop is closed and the feedback is
    /// stale.
    ///
    /// Shared by every path that completes a step, which is now two: the
    /// ordinary one and a human approving a park. A park raised *by* the
    /// failing step closes its own loop, and a second copy of this rule is
    /// how one of them would come to leak a previous cycle's feedback into
    /// every prompt after it.
    fn close_retry_loop_if_done(&mut self, step_exec: &StepExecution) {
        if crate::domain::rework::retry_loop_closed(
            self.retry_ctx
                .as_ref()
                .map(|rc| rc.failing_step_id.as_str()),
            &step_exec.step_id.0,
        ) {
            self.retry_ctx = None;
        }
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
                let budget = RetryBudget {
                    attempt: decision.attempt,
                    max: decision.max_attempts,
                };
                if let Some(action) = self
                    .redirect_to_step(
                        step_exec,
                        &target,
                        msg,
                        budget,
                        feedback.then(|| {
                            (
                                verdict_failure
                                    .map(|vf| vf.failing_tests.clone())
                                    .unwrap_or_default(),
                                verdict_failure
                                    .map(|vf| vf.implicated_files.clone())
                                    .unwrap_or_default(),
                            )
                        }),
                        dr,
                    )
                    .await
                {
                    return action;
                }
                // Dangling redirect target — same terminal
                // failure as v1's missing-`on_failure`-step.
                self.fail_step_and_feature(step_exec, msg, dr.spend(self.cache_tokens()))
                    .await;
            }
            RetryAction::Exhausted { target } => {
                if let Some(target) = target.as_ref() {
                    record_retry_exhausted(
                        self.status_writers(),
                        self.notifications.as_ref(),
                        step_exec,
                        target,
                        msg,
                        RetryBudget {
                            attempt: decision.attempt.saturating_sub(1),
                            max: decision.max_attempts,
                        },
                        dr.spend(self.cache_tokens()),
                    );
                }
                self.fail_step_and_feature(step_exec, msg, dr.spend(self.cache_tokens()))
                    .await;
            }
            RetryAction::RetryInPlace { .. } => {
                // Not derivable from v1 definitions for this
                // class; supported for v2 policies (P1.12).
                self.begin_in_place_retry(step_exec, msg, dr.spend(self.cache_tokens()));
                return RunAction::Continue;
            }
            RetryAction::Fail => {
                self.fail_step_and_feature(step_exec, msg, dr.spend(self.cache_tokens()))
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
            self.begin_in_place_retry(step_exec, msg, dr.spend(self.cache_tokens()));
            // Same step_index — the loop re-dispatches this step.
            return RunAction::Continue;
        }
        self.fail_step_and_feature(
            step_exec,
            &format!("[environment — not an implementation failure] {}", msg),
            dr.spend(self.cache_tokens()),
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
        self.fail_step_and_feature(step_exec, msg, dr.spend(self.cache_tokens()))
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
