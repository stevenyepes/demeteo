//! [`explain_failure`] and [`tail_log`] pinned directly, without a repo or
//! an execution driver in sight — both are pure fns over data already in
//! hand (research-report.md's "pure decision fn beside the data it decides
//! about" pattern).

use crate::domain::ids::StepExecutionId;
use crate::domain::models::step_attempt::{
    error_class, explain_failure, tail_log, LOG_TAIL_BUDGET_BYTES,
};
use crate::domain::models::StepAttempt;

fn failed_attempt(attempt_no: u32, fingerprint: &str) -> StepAttempt {
    StepAttempt {
        step_execution_id: StepExecutionId::from("se-1".to_string()),
        attempt_no,
        status: "failed".to_string(),
        cost_usd: Some(0.42),
        tokens: Some(1_234),
        wall_clock_ms: Some(5_000),
        error_class: Some(error_class::VERDICT.to_string()),
        failure_fingerprint: Some(fingerprint.to_string()),
        applied_rule: Some("verdict.redirect".to_string()),
        workspace_fingerprint: None,
        idempotency_key: None,
        started_at: 0,
        ended_at: Some(1),
    }
}

#[test]
fn repeated_failure_is_true_when_the_two_most_recent_fingerprints_match() {
    let attempts = vec![
        failed_attempt(1, "fp-a"),
        failed_attempt(2, "fp-a"),
        failed_attempt(3, "fp-a"),
    ];
    assert_eq!(explain_failure(&attempts).repeated_failure, Some(true));
}

#[test]
fn repeated_failure_is_false_when_the_two_most_recent_fingerprints_differ() {
    let attempts = vec![
        failed_attempt(1, "fp-a"),
        failed_attempt(2, "fp-b"),
        failed_attempt(3, "fp-c"),
    ];
    assert_eq!(explain_failure(&attempts).repeated_failure, Some(false));
}

#[test]
fn non_failure_last_attempt_reports_no_error_but_keeps_its_spend() {
    let completed = StepAttempt {
        step_execution_id: StepExecutionId::from("se-1".to_string()),
        attempt_no: 1,
        status: "completed".to_string(),
        cost_usd: Some(1.75),
        tokens: Some(9_001),
        wall_clock_ms: Some(60_000),
        error_class: None,
        failure_fingerprint: None,
        applied_rule: None,
        workspace_fingerprint: None,
        idempotency_key: None,
        started_at: 0,
        ended_at: Some(1),
    };
    let verdict = explain_failure(std::slice::from_ref(&completed));
    assert_eq!(verdict.error_class, None);
    assert_eq!(verdict.applied_rule, None);
    assert_eq!(verdict.repeated_failure, None);
    assert_eq!(verdict.cost_usd, completed.cost_usd);
    assert_eq!(verdict.tokens, completed.tokens);
    assert_eq!(verdict.wall_clock_ms, completed.wall_clock_ms);
}

#[test]
fn failing_last_attempt_surfaces_its_own_error_class_and_applied_rule() {
    let mut last = failed_attempt(2, "fp-a");
    last.error_class = Some(error_class::NON_RETRYABLE.to_string());
    last.applied_rule = Some("verdict.stop".to_string());
    let attempts = vec![failed_attempt(1, "fp-a"), last.clone()];

    let verdict = explain_failure(&attempts);

    assert_eq!(verdict.error_class, last.error_class);
    assert_eq!(verdict.applied_rule, last.applied_rule);
}

#[test]
fn single_failed_attempt_cannot_compare_against_itself() {
    let attempts = vec![failed_attempt(1, "fp-a")];
    assert_eq!(explain_failure(&attempts).repeated_failure, None);
}

#[test]
fn tail_log_truncates_at_a_line_boundary_within_budget() {
    let line = "a line of log output that repeats to build up bulk\n";
    let body = line.repeat((LOG_TAIL_BUDGET_BYTES / line.len()) + 10);
    assert!(body.len() > LOG_TAIL_BUDGET_BYTES);

    let tail = tail_log(&body, LOG_TAIL_BUDGET_BYTES);

    assert!(tail.truncated);
    assert!(tail.text.len() <= LOG_TAIL_BUDGET_BYTES);
    assert!(tail.omitted_bytes > 0);
    assert_eq!(tail.omitted_bytes, body.len() - tail.text.len());
    assert!(body.ends_with(tail.text.as_str()));
    // Cut on a line boundary: what precedes the tail, if anything, ends with
    // a newline rather than a fragment of a line.
    let prefix = &body[..tail.omitted_bytes];
    assert!(prefix.is_empty() || prefix.ends_with('\n'));
}

#[test]
fn tail_log_leaves_a_short_body_untouched() {
    let body = "short log, well under budget";
    let tail = tail_log(body, LOG_TAIL_BUDGET_BYTES);
    assert!(!tail.truncated);
    assert_eq!(tail.omitted_bytes, 0);
    assert_eq!(tail.text, body);
}
