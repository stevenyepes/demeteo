// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::is_transport_failure;
use crate::ports::execution::TRANSPORT_ERROR_PREFIX;

#[test]
fn transport_prefixed_error_is_transport() {
    let err = format!(
        "{}Timed out after the transport wall cap (1800s)",
        TRANSPORT_ERROR_PREFIX
    );
    assert!(is_transport_failure(&err));
}

#[test]
fn command_failure_is_not_transport() {
    // The non-zero-exit path ("Command failed (...)") carries no prefix,
    // so it stays a Verdict (a real red build the retry loop should act on).
    assert!(!is_transport_failure(
        "Command failed (exit code: 1): cd src-tauri && cargo test"
    ));
}
