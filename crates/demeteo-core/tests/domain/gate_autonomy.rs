// Tests extracted from `crates/demeteo-core/src/domain/gate_autonomy.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn attended_approves_nothing() {
    assert!(!auto_approves(GateAutonomy::Attended, false));
    assert!(!auto_approves(GateAutonomy::Attended, true));
}

#[test]
fn review_approves_review_gates_but_holds_the_ship_gate() {
    assert!(auto_approves(GateAutonomy::Review, false));
    assert!(!auto_approves(GateAutonomy::Review, true));
}

#[test]
fn full_approves_the_ship_gate_too() {
    assert!(auto_approves(GateAutonomy::Full, false));
    assert!(auto_approves(GateAutonomy::Full, true));
}

/// Nobody attached means nobody to answer a review gate; a project that
/// already approves more keeps it.
#[test]
fn an_unattended_run_never_runs_below_the_floor() {
    assert_eq!(GateAutonomy::Attended.for_run(true), GateAutonomy::Review);
    assert_eq!(GateAutonomy::Review.for_run(true), GateAutonomy::Review);
    assert_eq!(GateAutonomy::Full.for_run(true), GateAutonomy::Full);
}

#[test]
fn an_attended_run_keeps_the_projects_setting() {
    for level in [
        GateAutonomy::Attended,
        GateAutonomy::Review,
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
        GateAutonomy::Full,
    ] {
        assert_eq!(
            serde_json::to_value(level).unwrap(),
            serde_json::json!(level.as_str())
        );
    }
}
