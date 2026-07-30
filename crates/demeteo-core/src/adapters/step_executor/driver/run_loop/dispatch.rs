//! Single-step dispatch: refresh watchdog, mark running, open attempt row,
//! call NodeTypeRegistry, normalize VerdictFailed, evaluate retry policy,
//! close attempt row. Returns enough state for the outcome handler to act on.

use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::registry::{NodeCtx, NodeTypeRegistry};
use crate::adapters::step_executor::retry_policy::{FailureClass, RetryDecision};
use crate::adapters::step_executor::spend::StepSpend;
use crate::adapters::step_executor::step_status::CacheTokens;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::verifier::VerdictFailure;

/// Everything the outcome handler needs to apply one step's result,
/// bundled so `apply_outcome` doesn't need a 9-arg signature.
pub(crate) struct DispatchResult {
    /// The normalized outcome (`VerdictFailed` is already flattened into
    /// `Failed` here — the outcome handler never sees it).
    pub outcome: StepOutcome,
    /// The retry-policy decision that was evaluated *before* the attempt
    /// row closed, so it could be recorded on the row. `None` for
    /// `Completed`, `Cancelled`, and `RedirectTo` outcomes (no policy
    /// applies), and for `Failed` / `Environmental` / `NonRetryable`
    /// outcomes when the run is cancelled (no policy is applied on
    /// cancellation).
    pub failure_decision: Option<RetryDecision>,
    /// The structured verifier half of a `VerdictFailed` outcome, kept
    /// aside after normalization so the redirect path can carry
    /// `failing_tests` / `implicated_files` into the retry context.
    pub verdict_failure: Option<VerdictFailure>,
    pub accumulated_cost: f64,
    pub accumulated_tokens: i64,
    pub step_start: Instant,
}

impl DispatchResult {
    /// This dispatch's totals as the terminal write paths report them.
    /// The cache half is read from the driver at the call, not stored
    /// here: a later turn can move it, and the transition must carry
    /// whatever the driver last saw.
    pub(crate) fn spend(&self, cache: CacheTokens) -> StepSpend {
        StepSpend {
            cost: self.accumulated_cost,
            tokens: self.accumulated_tokens,
            cache,
            start: self.step_start,
        }
    }
}

impl ExecutionDriver {
    /// Drive one iteration of the run loop:
    /// 1. open a `step_attempts` row,
    /// 2. dispatch through [`NodeTypeRegistry`],
    /// 3. normalize `VerdictFailed` into `Failed`,
    /// 4. evaluate the declarative retry policy,
    /// 5. close the attempt row with this attempt's own spend / class /
    ///    applied-rule telemetry,
    /// 6. stash the cache-token telemetry on `self` for the final
    ///    `update_step_status` and the watchdog's session lifetime tracking.
    ///
    /// Returns `None` when the registry has no handler for this kind —
    /// the equivalent of v1's "Unknown step kind" failure, which has
    /// already written a `failed` step row and a `failed` feature row
    /// by the time it returns; the orchestrator exits the run loop.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn dispatch_step(
        &mut self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        step_execs: &[StepExecution],
        step_index: usize,
        step_start: Instant,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_cache_read: &mut Option<u64>,
        step_cache_creation: &mut Option<u64>,
    ) -> Option<DispatchResult> {
        // Per-attempt history (V31, task P1.8): one row per dispatch,
        // closed below with this attempt's own outcome and spend
        // deltas — retries stop overwriting history. Telemetry only,
        // so a write failure degrades to a warning, never a dead run.
        // The workspace fingerprint at node start (P1.14) rides on the
        // row: the resume guard compares it against the live workspace
        // after a crash, and it seeds the attempt's idempotency key.
        let fingerprint = self.current_workspace_fingerprint().await;
        let attempt = self.open_attempt(
            step_exec,
            *accumulated_cost,
            *accumulated_tokens,
            fingerprint.as_deref(),
        );

        // Every step kind resolves through the NodeTypeRegistry
        // (P1.6/P1.7) — the seam a new node type plugs into with a
        // single registration line. A registry miss is the same
        // "Unknown step kind" failure the old catch-all match arm
        // produced.
        let outcome = match NodeTypeRegistry::global().handler_for(&step_conf.kind) {
            Some(handler) => {
                handler
                    .execute(NodeCtx {
                        driver: self,
                        step_exec,
                        step_conf,
                        accumulated_cost,
                        accumulated_tokens,
                        step_start,
                        step_index,
                        step_execs,
                        out_cache_read: step_cache_read,
                        out_cache_creation: step_cache_creation,
                    })
                    .await
            }
            None => {
                let msg = format!("Unknown step kind: {}", step_conf.kind);
                let spend = StepSpend {
                    cost: *accumulated_cost,
                    tokens: *accumulated_tokens,
                    cache: self.cache_tokens(),
                    start: step_start,
                };
                self.fail_step_and_feature(step_exec, &msg, spend).await;
                return None;
            }
        };

        // Stash the step's cache telemetry on the driver so the
        // final `update_step_status` (and the watchdog's session
        // lifetime tracking) can read it.
        self.last_cache_read = *step_cache_read;
        self.last_cache_creation = *step_cache_creation;

        // A verdict failure follows the exact same on_failure path as a
        // plain failure — normalize it here and keep the structured
        // half aside so the retry context can carry failing tests and
        // implicated files to the redirected step.
        let (outcome, verdict_failure) = match outcome {
            StepOutcome::VerdictFailed(vf) => (StepOutcome::Failed(vf.to_feedback()), Some(vf)),
            other => (other, None),
        };

        // Evaluate the declarative retry policy (P1.10) for failure
        // outcomes *before* the attempt row closes, so the row can
        // record the rule that answered this failure. A cancel
        // preempts policy — no rule is applied to a cancelled run.
        let is_cancelled = *self.cancel_watch.borrow();
        let failure_decision: Option<RetryDecision> = match &outcome {
            StepOutcome::Failed(_) if !is_cancelled => {
                let class = if verdict_failure.is_some() {
                    FailureClass::Verdict
                } else {
                    FailureClass::AgentFailure
                };
                Some(self.retry_decision_for(step_conf, class, step_exec.iteration_count))
            }
            StepOutcome::Environmental(_) if !is_cancelled => {
                // Attempts the class has consumed = closed
                // environment-classed rows (durable V31 history,
                // P1.9 — a restart no longer grants a fresh free
                // retry) plus the failure being evaluated, whose
                // row is still open here. Guards: without attempt
                // accounting (open failed) or on a read error,
                // treat the budget as spent rather than risk an
                // unbounded in-place loop.
                let used = if attempt.is_none() {
                    u32::MAX
                } else {
                    self.features
                        .attempts_for_step(&step_exec.id)
                        .map(|rows| {
                            rows.iter()
                                .filter(|a| {
                                    a.error_class.as_deref()
                                        == Some(
                                            crate::domain::models::step_attempt::error_class::ENVIRONMENT,
                                        )
                                })
                                .count() as u32
                                + 1
                        })
                        .unwrap_or(u32::MAX)
                };
                Some(self.retry_decision_for(step_conf, FailureClass::Environment, used))
            }
            StepOutcome::NonRetryable(_) => {
                Some(self.retry_decision_for(step_conf, FailureClass::NonRetryable, 0))
            }
            _ => None,
        };

        // Close this dispatch's attempt row with its own outcome,
        // failure class (the P1.10 retry-policy vocabulary), the
        // applied policy rule, and spend deltas. Runs before the
        // outcome is acted on so every exit path below — including
        // the early `return`s — leaves a closed row behind.
        let wall_ms = step_start.elapsed().as_millis() as u64;
        self.close_attempt(
            step_exec,
            attempt.as_ref(),
            &outcome,
            *accumulated_cost,
            *accumulated_tokens,
            wall_ms,
            failure_decision.as_ref(),
            verdict_failure.as_ref(),
            &self.target_dir,
            is_cancelled,
        );

        Some(DispatchResult {
            outcome,
            failure_decision,
            verdict_failure,
            accumulated_cost: *accumulated_cost,
            accumulated_tokens: *accumulated_tokens,
            step_start,
        })
    }
}
