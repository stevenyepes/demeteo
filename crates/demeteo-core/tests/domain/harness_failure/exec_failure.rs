// Tests extracted from `src/adapters/step_executor/driver/verifier.rs`, moved
// with the code to `src/domain/harness_failure.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::{classify_exec_failure, HarnessExecFailure};
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};

#[test]
fn transport_prefixed_error_is_transport() {
    let err = format!(
        "{}Timed out after the 1800s drain budget waiting for command stdout",
        TRANSPORT_ERROR_PREFIX
    );
    assert_eq!(classify_exec_failure(&err), HarnessExecFailure::Transport);
}

#[test]
fn timeout_prefixed_error_is_timeout() {
    // The gap this classifier exists to close. Before it, `verifier.rs` never
    // referenced the timeout prefix at all, so a `timeout:` error fell through
    // to the non-zero-exit branch and became a Verdict — redirecting an agent
    // to "fix" code that never finished being tested.
    let err = format!("{}command exceeded its 1800s ceiling", TIMEOUT_ERROR_PREFIX);
    assert_eq!(classify_exec_failure(&err), HarnessExecFailure::Timeout);
}

#[test]
fn command_failure_is_non_zero_exit() {
    // The non-zero-exit path ("Command failed (...)") carries no prefix,
    // so it stays a Verdict (a real red build the retry loop should act on).
    assert_eq!(
        classify_exec_failure("Command failed (exit code: 1): cd src-tauri && cargo test"),
        HarnessExecFailure::NonZeroExit
    );
}

#[test]
fn prefix_must_lead_the_string_not_merely_appear_in_it() {
    // A suite whose own output quotes either prefix is still a red build. The
    // contract is a *prefix* on the port's error string (D3), so matching
    // anywhere would let test output rewrite its own classification — and a
    // failing assertion that prints the word `timeout: ` is not hard to come by.
    let quoting_timeout = format!(
        "Command failed (exit code: 1): assertion failed: expected \"{}...\"",
        TIMEOUT_ERROR_PREFIX
    );
    assert_eq!(
        classify_exec_failure(&quoting_timeout),
        HarnessExecFailure::NonZeroExit
    );

    let quoting_transport = format!(
        "Command failed (exit code: 1): 1 test failed, output contained \"{}\"",
        TRANSPORT_ERROR_PREFIX
    );
    assert_eq!(
        classify_exec_failure(&quoting_transport),
        HarnessExecFailure::NonZeroExit
    );
}
