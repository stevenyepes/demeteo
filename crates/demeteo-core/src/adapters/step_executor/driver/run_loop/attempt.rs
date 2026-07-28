//! Open / close the per-dispatch `step_attempts` row (V31, task P1.8).
//!
//! Telemetry only — write failures degrade to `tracing::warn!`, never block
//! the run. The attempt number returned by `open_attempt` is what
//! `close_attempt` references when it writes the terminal `status` and
//! the attempt's own spend / classification / applied-rule telemetry.
//!
//! The retry-policy (`RetryDecision`) and the structured verifier failure
//! (`VerdictFailure`) are passed through unchanged so the audit row names
//! the rule that answered each failure — see
//! `step_attempts.applied_rule` and the `step_attempts.error_class` field.

use crate::adapters::step_executor::driver::verifier;
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::retry_policy::RetryDecision;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::models::StepExecution;

/// One open attempt row — the `attempt_no` plus the running spend totals
/// captured at open time, so `close_attempt` can record the deltas.
#[derive(Debug, Clone)]
pub(crate) struct AttemptRecord {
    pub attempt_no: u32,
    pub cost_base: f64,
    pub tokens_base: i64,
}

impl ExecutionDriver {
    /// Probe the feature worktree's current fingerprint
    /// (`<HEAD>:<dirty|clean>`, P1.14) on whatever machine hosts it.
    /// `None` = probe failed; never blocks the run.
    pub(crate) async fn current_workspace_fingerprint(&self) -> Option<String> {
        let machine_str = self.machine_id();
        crate::adapters::step_executor::setup::workspace_fingerprint(
            &*self.exec,
            machine_str,
            &self.target_dir,
        )
        .await
    }

    /// Open a new `step_attempts` row for the dispatch that's about to
    /// start, recording the workspace fingerprint at node start (P1.14).
    /// Returns `None` on write failure (logged) so the caller can
    /// continue running the step — telemetry gaps must not block the run.
    pub(crate) fn open_attempt(
        &self,
        step_exec: &StepExecution,
        cost_base: f64,
        tokens_base: i64,
        workspace_fingerprint: Option<&str>,
    ) -> Option<AttemptRecord> {
        match self.features.attempt_open(
            &step_exec.id,
            crate::paths::now_ms(),
            workspace_fingerprint,
        ) {
            Ok(attempt_no) => Some(AttemptRecord {
                attempt_no,
                cost_base,
                tokens_base,
            }),
            Err(e) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    error = %e,
                    "failed to open step_attempts row"
                );
                None
            }
        }
    }

    /// Close the attempt row with this attempt's own outcome, the
    /// spent delta, the wall-clock milliseconds, the failure class
    /// (the P1.10 retry-policy vocabulary), the normalized failure
    /// fingerprint, and the applied policy rule.
    ///
    /// No-op when `attempt` is `None` (open failed) — the run is
    /// already happening and the gap is logged at `open_attempt`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn close_attempt(
        &self,
        step_exec: &StepExecution,
        attempt: Option<&AttemptRecord>,
        outcome: &StepOutcome,
        accumulated_cost: f64,
        accumulated_tokens: i64,
        wall_ms: u64,
        failure_decision: Option<&RetryDecision>,
        verdict_failure: Option<&crate::domain::verifier::VerdictFailure>,
        target_dir: &str,
        is_cancelled: bool,
    ) {
        let Some(attempt) = attempt else {
            return;
        };
        use crate::domain::models::step_attempt::error_class;
        let (att_status, att_class, att_fingerprint) = match outcome {
            StepOutcome::Completed => ("completed", None, None),
            StepOutcome::Failed(msg) => (
                // The step row records a Failed-during-cancel as
                // `interrupted`; mirror that here.
                if is_cancelled {
                    "interrupted"
                } else {
                    "failed"
                },
                Some(if verdict_failure.is_some() {
                    error_class::VERDICT
                } else {
                    error_class::AGENT_FAILURE
                }),
                Some(verifier::normalize_failure_fingerprint(msg, target_dir)),
            ),
            // Normalized into `Failed` above; kept exhaustive so a
            // future variant is a compile error, not a silent gap.
            StepOutcome::VerdictFailed(vf) => (
                "failed",
                Some(error_class::VERDICT),
                Some(verifier::normalize_failure_fingerprint(
                    &vf.to_feedback(),
                    target_dir,
                )),
            ),
            StepOutcome::Environmental(msg) => (
                "failed",
                Some(error_class::ENVIRONMENT),
                Some(verifier::normalize_failure_fingerprint(msg, target_dir)),
            ),
            StepOutcome::NonRetryable(msg) => (
                "failed",
                Some(error_class::NON_RETRYABLE),
                Some(verifier::normalize_failure_fingerprint(msg, target_dir)),
            ),
            StepOutcome::Cancelled => ("cancelled", None, None),
            StepOutcome::RedirectTo(_) => ("redirected", None, None),
        };
        if let Err(e) = self.features.attempt_close(
            &step_exec.id,
            attempt.attempt_no,
            att_status,
            accumulated_cost - attempt.cost_base,
            accumulated_tokens - attempt.tokens_base,
            wall_ms,
            att_class,
            att_fingerprint.as_deref(),
            failure_decision.map(|d| d.rule_id.as_str()),
            crate::paths::now_ms(),
        ) {
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                attempt_no = attempt.attempt_no,
                error = %e,
                "failed to close step_attempts row"
            );
        }
    }
}
