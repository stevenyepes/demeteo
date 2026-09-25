// Tests extracted from `crates/demeteo-core/src/domain/step_park.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::StepExecutionId;

fn park(redirect_to: Option<&str>) -> HumanPark {
    HumanPark::new(
        "the producer emitted no tickets".to_string(),
        redirect_to.map(|t| StepId::from(t.to_string())),
    )
}

fn id(s: &str) -> StepId {
    StepId::from(s.to_string())
}

fn with_choices() -> HumanPark {
    HumanPark {
        alternatives: vec![
            RedirectOption {
                named: id("s-spec"),
                lands_on: id("s-spec"),
            },
            RedirectOption {
                named: id("s-implement"),
                lands_on: id("s-tickets"),
            },
        ],
        redirect_brief: Some("evidence only".to_string()),
        ..park(Some("s-tickets"))
    }
}

#[test]
fn a_named_alternative_takes_the_redirect_without_the_default_brief() {
    assert_eq!(
        resolve_park(
            &with_choices(),
            Some(&answered("redirect", Some("s-spec — drop the criterion")))
        ),
        ParkResolution::Redirect {
            target: id("s-spec"),
            feedback: "s-spec — drop the criterion".to_string(),
        }
    );
}

#[test]
fn a_named_alternative_lands_where_naming_it_leads() {
    match resolve_park(
        &with_choices(),
        Some(&answered("redirect", Some("s-implement"))),
    ) {
        ParkResolution::Redirect { target, .. } => assert_eq!(target, id("s-tickets")),
        other => panic!("expected Redirect, got {other:?}"),
    }
}

#[test]
fn the_default_target_is_briefed_ahead_of_the_humans_words() {
    assert_eq!(
        resolve_park(
            &with_choices(),
            Some(&answered("redirect", Some("rerun the unit tests")))
        ),
        ParkResolution::Redirect {
            target: id("s-tickets"),
            feedback: "evidence only\n\nrerun the unit tests".to_string(),
        }
    );
}

/// Naming is whole-word, like a real gate's: a step id inside another word
/// is not an address.
#[test]
fn a_step_id_inside_another_word_is_not_an_address() {
    match resolve_park(
        &with_choices(),
        Some(&answered("redirect", Some("see s-spec2 notes"))),
    ) {
        ParkResolution::Redirect { target, .. } => assert_eq!(target, id("s-tickets")),
        other => panic!("expected Redirect, got {other:?}"),
    }
}

fn answered(decision: &str, feedback: Option<&str>) -> GateDecision {
    GateDecision {
        id: "gd-1".into(),
        step_execution_id: StepExecutionId::from("se-1".to_string()),
        decision: Some(decision.to_string()),
        feedback: feedback.map(str::to_string),
        created_at: 1,
    }
}

#[test]
fn approve_completes_the_step() {
    assert_eq!(
        resolve_park(&park(Some("s-tickets")), Some(&answered("approve", None))),
        ParkResolution::Complete
    );
}

#[test]
fn redirect_sends_the_humans_words_to_the_target() {
    assert_eq!(
        resolve_park(
            &park(Some("s-tickets")),
            Some(&answered("redirect", Some("scope it to the toggle copy")))
        ),
        ParkResolution::Redirect {
            target: StepId::from("s-tickets".to_string()),
            feedback: "scope it to the toggle copy".to_string(),
        }
    );
}

/// A redirect with no comment still has to say *something* to the target,
/// or the producer re-runs with no idea why it was sent back.
#[test]
fn a_redirect_with_no_comment_falls_back_to_the_parks_reason() {
    match resolve_park(&park(Some("s-tickets")), Some(&answered("redirect", None))) {
        ParkResolution::Redirect { feedback, .. } => {
            assert_eq!(feedback, "the producer emitted no tickets");
        }
        other => panic!("expected Redirect, got {other:?}"),
    }
}

/// The resume guard's documented rule: its only question is "safe to re-run
/// here?", so a redirect is not an answer to it. Failing is what it did
/// before this shared table existed, and silently approving would resume a
/// node the human declined to resume.
#[test]
fn a_redirect_a_park_cannot_honour_fails_rather_than_approving() {
    match resolve_park(&park(None), Some(&answered("redirect", Some("go back")))) {
        ParkResolution::Fail(msg) => {
            assert!(msg.contains("cannot honour"), "{msg}");
            assert!(msg.contains("the producer emitted no tickets"), "{msg}");
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn cancel_fails_and_echoes_what_was_answered() {
    match resolve_park(&park(Some("s-tickets")), Some(&answered("cancel", None))) {
        ParkResolution::Fail(msg) => assert!(msg.contains("'cancel'"), "{msg}"),
        other => panic!("expected Fail, got {other:?}"),
    }
}

/// An undecided row is not an approval. This is the shape a stale or
/// half-written row takes, and reading it as consent would resume a step no
/// human ever looked at.
#[test]
fn an_undecided_row_fails_rather_than_approving() {
    let undecided = GateDecision {
        decision: None,
        ..answered("approve", None)
    };
    match resolve_park(&park(None), Some(&undecided)) {
        ParkResolution::Fail(msg) => assert!(msg.contains("'none'"), "{msg}"),
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn waking_with_no_decision_is_a_cancellation() {
    assert_eq!(
        resolve_park(&park(Some("s-tickets")), None),
        ParkResolution::Cancelled
    );
}
