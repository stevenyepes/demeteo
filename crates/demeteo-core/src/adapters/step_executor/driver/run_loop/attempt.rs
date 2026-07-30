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

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::spend::StepSpend;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::harness_fingerprint::normalize_failure_fingerprint;
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::ports::db::FeatureRepository;

/// One open attempt row — the `attempt_no` plus the running spend totals
/// captured at open time, so `close_attempt` can record the deltas.
#[derive(Debug, Clone)]
pub(crate) struct AttemptRecord {
    pub attempt_no: u32,
    pub cost_base: f64,
    pub tokens_base: i64,
}

/// How one dispatch ended, in the three terms the attempt row records.
///
/// The triple is one answer, not three: the status and the class are read
/// together by the retry-policy audit, and the fingerprint is what makes
/// two rows the *same* failure.
pub(crate) struct AttemptClassification {
    pub status: &'static str,
    pub error_class: Option<&'static str>,
    pub fingerprint: Option<String>,
}

/// Which `(status, error_class, fingerprint)` triple a dispatch outcome
/// maps to.
///
/// Total and synchronous: it decides, and writes nothing. The match is
/// exhaustive on purpose — a new `StepOutcome` variant has to become a
/// compile error here rather than a silently unclassified attempt row.
///
/// `has_verdict` is whether a structured verifier failure accompanied a
/// `Failed` outcome, which is the only thing separating a `verdict` class
/// from an `agent_failure` one once `VerdictFailed` has been normalized.
pub(crate) fn classify(
    outcome: &StepOutcome,
    has_verdict: bool,
    is_cancelled: bool,
    target_dir: &str,
) -> AttemptClassification {
    use crate::domain::models::step_attempt::error_class;
    let (status, error_class, fingerprint) = match outcome {
        StepOutcome::Completed => ("completed", None, None),
        StepOutcome::Failed(msg) => (
            // The step row records a Failed-during-cancel as
            // `interrupted`; mirror that here.
            if is_cancelled {
                "interrupted"
            } else {
                "failed"
            },
            Some(if has_verdict {
                error_class::VERDICT
            } else {
                error_class::AGENT_FAILURE
            }),
            Some(normalize_failure_fingerprint(msg, target_dir)),
        ),
        // Normalized into `Failed` above; kept exhaustive so a
        // future variant is a compile error, not a silent gap.
        StepOutcome::VerdictFailed(vf) => (
            "failed",
            Some(error_class::VERDICT),
            Some(normalize_failure_fingerprint(&vf.to_feedback(), target_dir)),
        ),
        StepOutcome::Environmental(msg) => (
            "failed",
            Some(error_class::ENVIRONMENT),
            Some(normalize_failure_fingerprint(msg, target_dir)),
        ),
        StepOutcome::NonRetryable(msg) => (
            "failed",
            Some(error_class::NON_RETRYABLE),
            Some(normalize_failure_fingerprint(msg, target_dir)),
        ),
        StepOutcome::Cancelled => ("cancelled", None, None),
        StepOutcome::RedirectTo(_) => ("redirected", None, None),
    };
    AttemptClassification {
        status,
        error_class,
        fingerprint,
    }
}

/// Open a new `step_attempts` row for the dispatch that's about to
/// start, recording the workspace fingerprint at node start (P1.14).
/// Returns `None` on write failure (logged) so the caller can
/// continue running the step — telemetry gaps must not block the run.
pub(crate) fn open_attempt(
    features: &dyn FeatureRepository,
    f_id: &FeatureId,
    step_exec: &StepExecution,
    cost_base: f64,
    tokens_base: i64,
    workspace_fingerprint: Option<&str>,
) -> Option<AttemptRecord> {
    match features.attempt_open(&step_exec.id, crate::paths::now_ms(), workspace_fingerprint) {
        Ok(attempt_no) => Some(AttemptRecord {
            attempt_no,
            cost_base,
            tokens_base,
        }),
        Err(e) => {
            tracing::warn!(
                feature_id = %f_id,
                step_id = %step_exec.step_id.0,
                error = %e,
                "failed to open step_attempts row"
            );
            None
        }
    }
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

    /// Close the attempt row with this attempt's own classification, the
    /// spent delta, the wall-clock milliseconds, and the applied policy
    /// rule (the P1.10 retry-policy vocabulary).
    ///
    /// No-op when `attempt` is `None` (open failed) — the run is
    /// already happening and the gap is logged at [`open_attempt`].
    pub(crate) fn close_attempt(
        &self,
        step_exec: &StepExecution,
        attempt: Option<&AttemptRecord>,
        class: AttemptClassification,
        spend: StepSpend,
        wall_ms: u64,
        rule_id: Option<&str>,
    ) {
        let Some(attempt) = attempt else {
            return;
        };
        if let Err(e) = self.features.attempt_close(
            &step_exec.id,
            attempt.attempt_no,
            class.status,
            spend.cost - attempt.cost_base,
            spend.tokens - attempt.tokens_base,
            wall_ms,
            class.error_class,
            class.fingerprint.as_deref(),
            rule_id,
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

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/driver/attempt_classification.rs"]
mod attempt_classification_tests;
