// The gate's own vocabulary. `super` = `domain::gate::decision`.
//
// 135 lines of this used to sit inside a `&mut self` method that also wrote a
// `step_executions` row, emitted `StepProgress`, captured a memory signal and
// mutated the driver's retry context, so none of it was reachable from a test.

use super::*;

fn ids(names: &[&str]) -> Vec<StepId> {
    names.iter().map(|n| StepId::from(*n)).collect()
}

const STEPS: &[&str] = &["s-spec", "s-implement", "s-gate"];

#[test]
fn approve_with_no_feedback_captures_nothing() {
    assert_eq!(
        classify(Some("approve"), None, &ids(STEPS)),
        GateVerdict::Approve { signal: None }
    );
    assert_eq!(
        classify(Some("approve"), Some("   \n "), &ids(STEPS)),
        GateVerdict::Approve { signal: None },
        "whitespace is not a note"
    );
}

#[test]
fn approve_with_prose_captures_a_signal() {
    assert_eq!(
        classify(
            Some("approve"),
            Some("  ship it, but watch the retry budget  "),
            &ids(STEPS)
        ),
        GateVerdict::Approve {
            signal: Some("ship it, but watch the retry budget".to_string())
        },
        "the signal is the trimmed note"
    );
}

#[test]
fn reject_is_a_refusal_and_not_the_catch_all() {
    // `reject` is the remote inbox's word for `cancel`. It must stay
    // distinguishable from a typo, which lands on `Unrecognised` and a
    // different step outcome.
    assert_eq!(
        classify(Some("cancel"), None, &ids(STEPS)),
        GateVerdict::Cancel
    );
    assert_eq!(
        classify(Some("reject"), None, &ids(STEPS)),
        GateVerdict::Cancel
    );
    assert_ne!(
        classify(Some("reject"), None, &ids(STEPS)),
        GateVerdict::Unrecognised
    );
}

#[test]
fn an_unrecognised_string_is_not_a_refusal() {
    for word in ["aprove", "REDIRECT", "", "yes"] {
        assert_eq!(
            classify(Some(word), None, &ids(STEPS)),
            GateVerdict::Unrecognised,
            "'{word}' should fall through"
        );
    }
    assert_eq!(classify(None, None, &ids(STEPS)), GateVerdict::Unrecognised);
}

#[test]
fn a_redirect_naming_a_step_sets_retry_feedback_but_captures_no_signal() {
    // The asymmetry that is easiest to break by "tidying": a step id is an
    // address, not advice, so it is never stored as guidance — but it still
    // reaches the redirected step's prompt, exactly as it does today.
    assert_eq!(
        classify(Some("redirect"), Some(" s-implement "), &ids(STEPS)),
        GateVerdict::Redirect {
            signal: None,
            retry_feedback: Some("s-implement".to_string()),
        }
    );
}

#[test]
fn a_redirect_with_prose_both_captures_and_carries_it() {
    assert_eq!(
        classify(
            Some("redirect"),
            Some("the empty state looks wrong"),
            &ids(STEPS)
        ),
        GateVerdict::Redirect {
            signal: Some("the empty state looks wrong".to_string()),
            retry_feedback: Some("the empty state looks wrong".to_string()),
        }
    );
}

#[test]
fn a_redirect_with_no_feedback_carries_neither() {
    assert_eq!(
        classify(Some("redirect"), Some("  "), &ids(STEPS)),
        GateVerdict::Redirect {
            signal: None,
            retry_feedback: None,
        }
    );
}

#[test]
fn feedback_naming_a_step_alongside_prose_is_still_guidance() {
    // Only an exact match suppresses the signal — "redo s-implement, the
    // split is too coarse" is advice that happens to contain an id.
    assert_eq!(
        classify(
            Some("redirect"),
            Some("redo s-implement, the split is too coarse"),
            &ids(STEPS)
        ),
        GateVerdict::Redirect {
            signal: Some("redo s-implement, the split is too coarse".to_string()),
            retry_feedback: Some("redo s-implement, the split is too coarse".to_string()),
        }
    );
}
