/// Outcome returned by each step-type handler after execution completes.
pub(crate) enum StepOutcome {
    /// Step finished successfully; advance to the next step.
    Completed,
    /// Step failed with the given error message; may be retried via on_failure.
    Failed(String),
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
