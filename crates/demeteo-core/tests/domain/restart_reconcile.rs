//! Restart reconciliation, reached directly: no ports, no runtime, no driver.
//!
//! Every string below is the literal the watchdog wrote before the rule moved
//! here. Nothing asserted any of them previously — reaching the decision meant
//! six stubbed ports and four nested loops — so these are the first tests that
//! can tell a reworded message from a reclassified step.

use super::*;

// ── Work a dead process was in the middle of ─────────────────────────────────

/// A gate the user never got to answer. The wait was interrupted, not the
/// step's own work, and the row already has the gate that was waiting — so
/// nothing is synthesised for it.
#[test]
fn an_awaiting_gate_step_reports_the_gate_and_synthesises_nothing() {
    let out = interrupted_by_restart("awaiting_gate").expect("an in-flight gate reconciles");

    assert_eq!(out.message, "Gate interrupted by system restart");
    assert!(
        !out.synthesise_gate_decision,
        "the step already has the gate row it was waiting on; a second would \
         overwrite the real decision's slot"
    );
}

/// A step that was mid-work. It has no gate row of its own, so recovery
/// synthesises one — that pending row is what backs the prompt asking the user
/// what to do with the interrupted step.
#[test]
fn a_running_step_reports_the_step_and_gets_a_synthesised_gate() {
    let out = interrupted_by_restart("running").expect("in-flight work reconciles");

    assert_eq!(out.message, "Step interrupted by system restart");
    assert!(out.synthesise_gate_decision);
}

/// Everything else is either terminal or not yet started. `pending` is the one
/// worth naming: under a live feature it is a healthy row the scheduler has
/// not reached, and closing it here would end runs that were about to continue.
#[test]
fn no_other_status_is_interrupted_work() {
    for status in [
        "pending",
        "verifying",
        "completed",
        "failed",
        "interrupted",
        "cancelled",
        "skipped",
        "",
    ] {
        assert_eq!(
            interrupted_by_restart(status),
            None,
            "{status} must not be reconciled as interrupted"
        );
    }
}

/// The two arms are distinguishable in the timeline. A single shared sentence
/// would leave a user unable to tell "the agent was killed mid-turn" from "we
/// were waiting on you" — different things to do next.
#[test]
fn the_two_arms_do_not_share_a_sentence() {
    let gate = interrupted_by_restart("awaiting_gate").expect("gate");
    let step = interrupted_by_restart("running").expect("step");

    assert_ne!(gate.message, step.message);
    assert_ne!(gate.synthesise_gate_decision, step.synthesise_gate_decision);
}

// ── Rows behind the one that died ────────────────────────────────────────────

/// A step the scheduler never reached, under a feature that has already
/// stopped. It can never advance, so it is closed with a message that says why
/// rather than left rendering a spinner forever.
#[test]
fn a_pending_step_under_an_ended_feature_is_orphaned() {
    for feature_status in ["cancelled", "failed"] {
        assert_eq!(
            orphaned_by_feature_end(feature_status, "pending"),
            Some("Step orphaned: feature ended before step ran".to_string()),
            "a pending step under a {feature_status} feature can never run"
        );
    }
}

/// The feature's status is an input, not an assumption. The identical `pending`
/// row under a live feature is exactly what the run is about to execute.
#[test]
fn a_pending_step_under_a_live_feature_is_left_alone() {
    for feature_status in [
        "running",
        "gated",
        "awaiting_gate",
        "bootstrapping",
        "completed",
        "",
    ] {
        assert_eq!(
            orphaned_by_feature_end(feature_status, "pending"),
            None,
            "a {feature_status} feature's pending steps are not orphans"
        );
    }
}

/// Only `pending` is orphaned. A `running` or `awaiting_gate` row under an
/// ended feature is the first pass's business, and a terminal one is nobody's.
#[test]
fn only_a_pending_step_is_orphaned_by_the_feature_ending() {
    for step_status in [
        "running",
        "awaiting_gate",
        "verifying",
        "completed",
        "failed",
        "interrupted",
    ] {
        assert_eq!(
            orphaned_by_feature_end("cancelled", step_status),
            None,
            "{step_status} is not an orphan"
        );
    }
}

/// The predicate the adapter uses to decide whether a feature's step rows are
/// worth reading at all — spelled once so it cannot disagree with the rule
/// above it.
#[test]
fn only_a_cancelled_or_failed_feature_has_ended() {
    assert!(feature_ended("cancelled"));
    assert!(feature_ended("failed"));
    for live in ["running", "gated", "awaiting_gate", "bootstrapping", ""] {
        assert!(!feature_ended(live), "{live} is not an ended feature");
    }
}
