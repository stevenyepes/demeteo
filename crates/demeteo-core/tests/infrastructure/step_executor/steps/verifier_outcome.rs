// Tests for `impl From<VerifierError> for StepOutcome`. `super` = the
// `steps` module. No doubles and no runtime — the mapping is total and pure.

use super::*;

use crate::domain::verifier::{VerdictFailure, VerifierError};

#[test]
fn a_verdict_failure_keeps_its_structure_for_the_retry_loop() {
    let failure = VerdictFailure {
        reason: "criterion 2 is not met".into(),
        failing_tests: vec!["suite::case".into()],
        implicated_files: vec!["src/lib.rs".into()],
    };
    match StepOutcome::from(VerifierError::Verdict(failure)) {
        StepOutcome::VerdictFailed(f) => {
            assert_eq!(f.reason, "criterion 2 is not met");
            assert_eq!(f.failing_tests, vec!["suite::case".to_string()]);
            assert_eq!(f.implicated_files, vec!["src/lib.rs".to_string()]);
        }
        _ => panic!("only a Verdict failure may feed the on_failure retry loop"),
    }
}

#[test]
fn an_infrastructure_error_is_non_retryable_and_names_the_verifier_config() {
    match StepOutcome::from(VerifierError::Infrastructure("harness timed out".into())) {
        StepOutcome::NonRetryable(msg) => assert_eq!(
            msg,
            "[verifier infrastructure error — check verifier config] harness timed out"
        ),
        _ => panic!("a broken verifier setup must not be retried as an implementation defect"),
    }
}

#[test]
fn an_environment_error_carries_its_remediation_bare() {
    let remediation = "Install libgtk-3-dev, then re-run `cargo test`.";
    match StepOutcome::from(VerifierError::Environment(remediation.into())) {
        StepOutcome::NonRetryable(msg) => assert_eq!(
            msg, remediation,
            "the triage message is already user-facing; a prefix would bury it"
        ),
        _ => panic!("an unprovisioned box must terminate, not burn the retry budget"),
    }
}

#[test]
fn a_cancel_is_not_a_failure() {
    assert!(
        matches!(
            StepOutcome::from(VerifierError::Cancelled),
            StepOutcome::Cancelled
        ),
        "nothing was judged and nothing should be persisted as an error"
    );
}
