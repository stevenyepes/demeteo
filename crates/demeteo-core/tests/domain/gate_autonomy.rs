// Tests extracted from `crates/demeteo-core/src/domain/gate_autonomy.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn attended_approves_nothing() {
    assert_eq!(
        policy_answer(GateAutonomy::Attended, false),
        PolicyAnswer::Ask
    );
    assert_eq!(
        policy_answer(GateAutonomy::Attended, true),
        PolicyAnswer::Ask
    );
}

#[test]
fn review_approves_review_gates_but_holds_the_ship_gate() {
    assert_eq!(
        policy_answer(GateAutonomy::Review, false),
        PolicyAnswer::Approve
    );
    assert_eq!(policy_answer(GateAutonomy::Review, true), PolicyAnswer::Ask);
}

#[test]
fn full_approves_the_ship_gate_too() {
    assert_eq!(
        policy_answer(GateAutonomy::Full, false),
        PolicyAnswer::Approve
    );
    assert_eq!(
        policy_answer(GateAutonomy::Full, true),
        PolicyAnswer::Approve
    );
}

#[test]
fn clean_approves_review_gates_and_makes_the_ship_gate_turn_on_its_review() {
    assert_eq!(
        policy_answer(GateAutonomy::Clean, false),
        PolicyAnswer::Approve
    );
    assert_eq!(
        policy_answer(GateAutonomy::Clean, true),
        PolicyAnswer::ApproveIfReviewClean
    );
}

/// `for_run` takes a floor with `max`, so the declaration order is the policy.
#[test]
fn the_levels_are_ordered_by_how_much_they_approve() {
    assert!(GateAutonomy::Attended < GateAutonomy::Review);
    assert!(GateAutonomy::Review < GateAutonomy::Clean);
    assert!(GateAutonomy::Clean < GateAutonomy::Full);
}

/// Nobody attached means nobody to answer a review gate; a project that
/// already approves more keeps it.
#[test]
fn an_unattended_run_never_runs_below_the_floor() {
    assert_eq!(GateAutonomy::Attended.for_run(true), GateAutonomy::Review);
    assert_eq!(GateAutonomy::Review.for_run(true), GateAutonomy::Review);
    assert_eq!(GateAutonomy::Clean.for_run(true), GateAutonomy::Clean);
    assert_eq!(GateAutonomy::Full.for_run(true), GateAutonomy::Full);
}

#[test]
fn an_attended_run_keeps_the_projects_setting() {
    for level in [
        GateAutonomy::Attended,
        GateAutonomy::Review,
        GateAutonomy::Clean,
        GateAutonomy::Full,
    ] {
        assert_eq!(level.for_run(false), level);
    }
}

#[test]
fn an_unreadable_column_falls_toward_asking() {
    assert_eq!(GateAutonomy::from_column(None), GateAutonomy::Attended);
    assert_eq!(
        GateAutonomy::from_column(Some("yolo")),
        GateAutonomy::Attended
    );
    for level in [
        GateAutonomy::Attended,
        GateAutonomy::Review,
        GateAutonomy::Clean,
        GateAutonomy::Full,
    ] {
        assert_eq!(GateAutonomy::from_column(Some(level.as_str())), level);
    }
}

#[test]
fn serde_spells_what_the_column_stores() {
    for level in [
        GateAutonomy::Attended,
        GateAutonomy::Review,
        GateAutonomy::Clean,
        GateAutonomy::Full,
    ] {
        assert_eq!(
            serde_json::to_value(level).unwrap(),
            serde_json::json!(level.as_str())
        );
    }
}
