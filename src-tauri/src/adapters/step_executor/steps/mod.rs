/// Outcome returned by each step-type handler after execution completes.
pub(crate) enum StepOutcome {
    /// Step finished successfully; advance to the next step.
    Completed,
    /// Step failed with the given error message.
    Failed(String),
    /// Execution was cancelled by the user.
    Cancelled,
    /// Gate "redirect" decision — jump to the given step index.
    RedirectTo(usize),
}

pub(crate) mod agent;
pub(crate) mod gate;
pub(crate) mod parallel;
