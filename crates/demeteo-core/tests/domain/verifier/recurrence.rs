// Tests for `crates/demeteo-core/src/domain/verifier/recurrence.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

const TREE: &str = "100644 blob aaa\tCargo.toml\n\
                    040000 tree bbb\tartifacts\n\
                    040000 tree ccc\tsrc\n";

fn fail(reason: &str) -> VerdictFailure {
    VerdictFailure::from_reason(reason)
}

#[test]
fn the_artifact_directory_does_not_count_as_code() {
    let rewritten = TREE.replace("bbb", "zzz");
    assert_eq!(
        code_state(TREE, "artifacts"),
        code_state(&rewritten, "artifacts")
    );
    assert_eq!(
        code_state(TREE, "artifacts/"),
        code_state(&rewritten, "artifacts/")
    );
}

#[test]
fn a_source_change_is_a_different_code_state() {
    let changed = TREE.replace("ccc", "ddd");
    assert_ne!(
        code_state(TREE, "artifacts"),
        code_state(&changed, "artifacts")
    );
}

#[test]
fn criteria_are_normalized_sorted_and_deduplicated() {
    assert_eq!(
        implicated_criteria("AC6 lacks evidence; see ac-6 and AC 2, not ACCESS or ACE"),
        vec!["AC2".to_string(), "AC6".to_string()]
    );
    assert!(implicated_criteria("criterion three is not met").is_empty());
}

/// The incident's shape: the model rephrases, the criterion does not.
#[test]
fn a_rephrased_reason_naming_the_same_criterion_is_the_same_finding() {
    let state = code_state(TREE, "artifacts");
    let a = verdict_fingerprint(&state, &fail("AC6: no evidence the new tests failed first"));
    let b = verdict_fingerprint(
        &state,
        &fail("Criterion AC6 is unproven — nothing shows each test was watched failing."),
    );
    assert_eq!(a, b);
    assert!(verdict_recurs(Some(&a), &b));
}

#[test]
fn a_different_criterion_is_a_different_finding() {
    let state = code_state(TREE, "artifacts");
    let a = verdict_fingerprint(&state, &fail("AC6 unproven"));
    let b = verdict_fingerprint(&state, &fail("AC3 unproven"));
    assert!(!verdict_recurs(Some(&a), &b));
}

#[test]
fn the_same_finding_over_changed_code_is_progress_not_a_loop() {
    let a = verdict_fingerprint(&code_state(TREE, "artifacts"), &fail("AC6 unproven"));
    let b = verdict_fingerprint(
        &code_state(&TREE.replace("ccc", "ddd"), "artifacts"),
        &fail("AC6 unproven"),
    );
    assert!(!verdict_recurs(Some(&a), &b));
}

#[test]
fn without_criteria_the_reasons_words_decide() {
    let state = code_state(TREE, "artifacts");
    let a = verdict_fingerprint(&state, &fail("The toggle copy is wrong."));
    let b = verdict_fingerprint(&state, &fail("the  toggle copy is WRONG"));
    let c = verdict_fingerprint(&state, &fail("The empty state is missing."));
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn a_first_failure_or_a_harness_fingerprint_is_never_a_recurrence() {
    let fp = verdict_fingerprint(&code_state(TREE, "artifacts"), &fail("AC6"));
    assert!(!verdict_recurs(None, &fp));
    assert!(!verdict_recurs(
        Some("error[E0425]: cannot find value"),
        &fp
    ));
    assert!(!verdict_recurs(Some("same"), "same"));
}
