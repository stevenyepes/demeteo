//! What a `command` node's run meant.
//!
//! The failure-class table in
//! [`steps::command`](crate::adapters::step_executor::steps::command)'s module
//! header is the contract this module answers to; the caller maps each variant
//! onto the executor's own `StepOutcome`, which stays adapter-side.

use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};

/// Head/tail budget for the stdout carried into a failure message. The
/// full output is always in the artifact; this is only the inline
/// feedback a redirected agent step reads.
pub(crate) const FEEDBACK_TAIL_BYTES: usize = 4_000;

/// What the execution port answered, read as one of four fates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandRun {
    /// Exit 0. The payload is the command's merged stdout+stderr.
    Succeeded(String),
    /// A non-zero exit. The harness spoke; the payload is what it said.
    Failed(String),
    /// The command never ran.
    Transport(String),
    /// The command was abandoned rather than judged.
    TimedOut(String),
}

/// Read the run's `Result` as one of the four fates.
///
/// A transport failure is the machine, not the command: it never
/// ran, so classifying it as a verdict would redirect an agent step
/// to "fix" code that was never tested (C0.2 / D3). A timeout is the
/// same call for the same reason — the command was abandoned, not
/// judged.
pub(crate) fn classify_run(result: Result<String, String>) -> CommandRun {
    match result {
        Ok(out) => CommandRun::Succeeded(out),
        Err(err) if err.starts_with(TRANSPORT_ERROR_PREFIX) => CommandRun::Transport(err),
        Err(err) if err.starts_with(TIMEOUT_ERROR_PREFIX) => CommandRun::TimedOut(err),
        Err(err) => CommandRun::Failed(err),
    }
}

/// Last `limit` bytes of `output`, on a char boundary, prefixed with an
/// elision marker when truncated.
pub(crate) fn feedback_tail(output: &str, limit: usize) -> String {
    if output.len() <= limit {
        return output.to_string();
    }
    let mut start = output.len() - limit;
    while start < output.len() && !output.is_char_boundary(start) {
        start += 1;
    }
    format!("…(truncated)…\n{}", &output[start..])
}

#[cfg(test)]
#[path = "../../../tests/domain/command_step/outcome.rs"]
mod tests;
