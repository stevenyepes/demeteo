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

pub(crate) mod agent;
pub(crate) mod gate;
pub(crate) mod parallel;
pub(crate) mod sync;
