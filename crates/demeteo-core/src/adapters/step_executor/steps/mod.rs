/// Outcome returned by each step-type handler after execution completes.
pub(crate) enum StepOutcome {
    /// Step finished successfully; advance to the next step.
    Completed,
    /// Step failed with the given error message; may be retried via on_failure.
    Failed(String),
    /// A verifier / harness explicitly failed the work, with structured
    /// data (failing tests, implicated files). Follows the same
    /// on_failure retry path as `Failed`, but the structure survives
    /// into the retry context so the next attempt can be targeted at
    /// the subtasks that own the implicated files.
    VerdictFailed(crate::domain::verifier::VerdictFailure),
    /// The environment broke, not the implementation: agent process
    /// blocked/timed out, spawn failure, worktree provisioning or scope
    /// fence setup failed. Redirecting to an implementation step cannot
    /// fix these, so they must not consume the on_failure retry budget —
    /// the driver retries the same step once, then fails the feature.
    Environmental(String),
    /// Step failed for a reason that retrying the implementation step cannot fix
    /// (e.g. verifier infrastructure error: timeout, spawn failure, parse error).
    /// Fails the step immediately without consulting evaluate_on_failure.
    NonRetryable(String),
    /// Execution was cancelled by the user.
    Cancelled,
    /// Gate "redirect" decision — jump to the given step index.
    RedirectTo(usize),
}

impl From<crate::domain::sequence::outcome::SequenceError> for StepOutcome {
    /// The literal mapping, variant for variant.
    ///
    /// Callers inside the sequence step do **not** use this directly: a
    /// cancel can arrive while a git command is mid-flight and surface as
    /// an ordinary-looking `Failed`, so the step consults its cancel watch
    /// before converting (see `ExecutionDriver::fail_sequence_step`). This
    /// impl is the mapping with no such context — correct wherever the
    /// error is already known to be self-describing.
    fn from(err: crate::domain::sequence::outcome::SequenceError) -> Self {
        use crate::domain::sequence::outcome::SequenceError;
        match err {
            SequenceError::Cancelled => Self::Cancelled,
            SequenceError::Failed(msg) => Self::Failed(msg),
            SequenceError::Environmental(msg) => Self::Environmental(msg),
        }
    }
}

impl From<crate::domain::verifier::VerifierError> for StepOutcome {
    /// The mapping [`VerifierError`]'s own rustdoc states in prose, expressed
    /// once in code instead of re-spelled at each caller.
    ///
    /// [`VerifierError`]: crate::domain::verifier::VerifierError
    fn from(err: crate::domain::verifier::VerifierError) -> Self {
        use crate::domain::verifier::VerifierError;
        match err {
            VerifierError::Verdict(failure) => Self::VerdictFailed(failure),
            VerifierError::Infrastructure(msg) => Self::NonRetryable(format!(
                "[verifier infrastructure error — check verifier config] {}",
                msg
            )),
            // Triaged (C6) as an environment problem: the box is not
            // provisioned, editing source can't fix it. The message
            // is already user-facing remediation and the
            // notification was fired at triage time — terminate now.
            VerifierError::Environment(msg) => Self::NonRetryable(msg),
            // Stop was pressed while the harness was running. Not
            // a failure — the caller has already cleaned up its worktree
            // and nothing should be persisted as an error.
            VerifierError::Cancelled => Self::Cancelled,
        }
    }
}

pub(crate) mod agent;
pub(crate) mod command;
pub(crate) mod conflict_pass;
pub(crate) mod finalize;
pub(crate) mod gate;
pub(crate) mod list_unmerged;
pub(crate) mod sequence;
pub(crate) mod sync;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/steps/verifier_outcome.rs"]
mod verifier_outcome_tests;
