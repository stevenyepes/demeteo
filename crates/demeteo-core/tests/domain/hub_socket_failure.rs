use super::*;
use serde_json::json;

fn row(offset: i64, kind: &str, payload: Value) -> RunEvent {
    RunEvent {
        offset,
        run_id: "f-1".into(),
        kind: kind.into(),
        payload_json: Some(payload.to_string()),
        created_at: 1_700_000_000_000,
    }
}

fn step(offset: i64, step_id: &str, status: &str) -> RunEvent {
    row(
        offset,
        "step_progress",
        json!({ "step_id": step_id, "status": status, "cost_usd": 0.1 }),
    )
}

fn feature(offset: i64, status: &str) -> RunEvent {
    row(offset, "feature_status", json!({ "status": status }))
}

fn decision(offset: i64, step_id: &str, class: &str, reason: &str) -> RunEvent {
    row(
        offset,
        "retry_decision",
        json!({
            "step_id": step_id,
            "error_class": class,
            "rule_id": format!("{class}.fail"),
            "action": "fail",
            "reason": reason,
        }),
    )
}

#[test]
fn each_error_class_maps_to_its_own_reason() {
    let table = [
        ("usage_limit", FailureReason::UsageLimit),
        (error_class::VERDICT, FailureReason::VerdictFailed),
        (error_class::ENVIRONMENT, FailureReason::Environment),
        (error_class::AGENT_FAILURE, FailureReason::AgentError),
        (error_class::NON_RETRYABLE, FailureReason::NotRetryable),
        ("a_class_from_a_newer_build", FailureReason::Other),
        ("", FailureReason::Other),
    ];
    for (class, expected) in table {
        assert_eq!(classified(class, "tests failed"), expected, "{class}");
    }
}

#[test]
fn a_tripped_wall_clock_cap_is_named_whatever_class_it_failed_under() {
    let turn = format!("Agent step {WALL_CLOCK_CAP_PHRASE} (1800s / 1801s elapsed)");
    let verifier = format!("Verifier {WALL_CLOCK_CAP_PHRASE} (600s)");
    for (class, reason) in [
        (error_class::ENVIRONMENT, turn.as_str()),
        (error_class::AGENT_FAILURE, verifier.as_str()),
    ] {
        assert_eq!(classified(class, reason), FailureReason::WallClockCap);
    }
    assert_eq!(
        classified("usage_limit", &turn),
        FailureReason::UsageLimit,
        "a usage limit is the provider's answer and outranks the text"
    );
}

#[test]
fn a_step_failure_carries_the_decision_that_answered_it() {
    let before = [
        step(1, "implement", "running"),
        decision(2, "implement", error_class::VERDICT, "2 tests failed"),
    ];
    assert_eq!(
        failure_reason(&step(3, "implement", "failed"), &before),
        Some(FailureReason::VerdictFailed)
    );
}

#[test]
fn a_decision_from_an_earlier_attempt_does_not_answer_a_later_failure() {
    let before = [
        step(1, "implement", "running"),
        decision(2, "implement", error_class::VERDICT, "2 tests failed"),
        step(3, "implement", "failed"),
        step(4, "implement", "running"),
    ];
    assert_eq!(
        failure_reason(&step(5, "implement", "failed"), &before),
        Some(FailureReason::Other)
    );
}

#[test]
fn another_steps_decision_does_not_answer_this_one() {
    let before = [
        decision(1, "validate", error_class::VERDICT, "criteria unmet"),
        step(2, "implement", "running"),
    ];
    assert_eq!(
        failure_reason(&step(3, "implement", "failed"), &before),
        Some(FailureReason::Other)
    );
}

#[test]
fn rows_at_or_after_the_failure_are_not_its_context() {
    let failed = step(5, "implement", "failed");
    let later = [decision(6, "implement", error_class::VERDICT, "late")];
    assert_eq!(failure_reason(&failed, &later), Some(FailureReason::Other));
}

#[test]
fn a_triage_verdict_of_a_broken_machine_outranks_the_class_except_a_usage_limit() {
    let not_ready = row(
        2,
        "env_not_ready",
        json!({ "step_id": "harness", "reason": "missing gdk-3.0" }),
    );
    let non_retryable = [
        not_ready.clone(),
        decision(3, "harness", error_class::NON_RETRYABLE, "missing gdk-3.0"),
    ];
    assert_eq!(
        failure_reason(&step(4, "harness", "failed"), &non_retryable),
        Some(FailureReason::Environment)
    );
    let usage = [not_ready, decision(3, "harness", "usage_limit", "quota")];
    assert_eq!(
        failure_reason(&step(4, "harness", "failed"), &usage),
        Some(FailureReason::UsageLimit)
    );
}

#[test]
fn a_feature_failure_carries_the_reason_of_the_step_that_failed_it() {
    let before = [
        step(1, "implement", "running"),
        decision(2, "implement", "usage_limit", "429"),
        step(3, "implement", "failed"),
    ];
    assert_eq!(
        failure_reason(&feature(4, "failed"), &before),
        Some(FailureReason::UsageLimit)
    );
}

#[test]
fn a_feature_that_failed_in_bootstrap_failed_on_its_environment() {
    let before = [row(
        1,
        "bootstrap_progress",
        json!({ "phase": "clone", "label": "Clone", "status": "failed", "detail": "x" }),
    )];
    assert_eq!(
        failure_reason(&feature(2, "failed"), &before),
        Some(FailureReason::Environment)
    );
}

#[test]
fn a_feature_failure_with_nothing_before_it_reports_no_reason() {
    assert_eq!(failure_reason(&feature(9, "failed"), &[]), None);
    assert_eq!(
        failure_reason(&feature(9, "failed"), &[feature(1, "running")]),
        None
    );
}

#[test]
fn a_runner_cap_names_which_cap_cancelled_the_run() {
    let cost = row(1, "over_budget", json!("cost $5.0100 exceeded cap $5.0000"));
    let wall = row(
        1,
        "over_budget",
        json!(format!("{OVER_WALL_CLOCK_CAP} 3601s exceeded cap 3600s")),
    );
    assert_eq!(
        failure_reason(&feature(2, "cancelled"), &[cost]),
        Some(FailureReason::BudgetCap)
    );
    assert_eq!(
        failure_reason(&feature(2, "cancelled"), &[wall]),
        Some(FailureReason::WallClockCap)
    );
    assert_eq!(failure_reason(&feature(2, "cancelled"), &[]), None);
}

#[test]
fn only_failures_and_cancels_carry_a_reason() {
    let before = [decision(1, "implement", error_class::VERDICT, "x")];
    for not_a_failure in [
        step(2, "implement", "running"),
        step(2, "implement", "completed"),
        feature(2, "running"),
        feature(2, "completed"),
        decision(2, "implement", error_class::VERDICT, "x"),
    ] {
        assert!(!may_carry_failure(&not_a_failure), "{}", not_a_failure.kind);
        assert_eq!(failure_reason(&not_a_failure, &before), None);
    }
    for failure in [
        step(2, "implement", "failed"),
        feature(2, "failed"),
        feature(2, "cancelled"),
    ] {
        assert!(may_carry_failure(&failure));
    }
}
