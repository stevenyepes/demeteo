// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/retry_policy.rs` (mirrored-tests convention). `super` = that module.
//
// Coverage matrix (task P1.10 "Done when"): every failure class ×
// strategy, plus the legacy-derivation parity that keeps v1 behavior
// byte-identical (budget precedence, empty/dangling targets, the
// environment one-shot, and the u32::MAX broken-accounting guard).

use super::*;
use crate::domain::models::workflow_v2::{RetryPolicy, RetryRule, RetryStrategy};

fn step(on_failure: Option<&str>, max_iterations: Option<u32>) -> StepConfig {
    serde_json::from_value(serde_json::json!({
        "id": "s-validate",
        "kind": "agent",
        "title": "Validate",
        "on_failure": on_failure,
        "max_iterations": max_iterations,
    }))
    .unwrap()
}

fn target(d: &RetryDecision) -> Option<&str> {
    match &d.action {
        RetryAction::Redirect { target, .. } => Some(target.0.as_str()),
        RetryAction::Exhausted { target } => target.as_ref().map(|t| t.0.as_str()),
        _ => None,
    }
}

// ── Legacy derivation ──────────────────────────────────────────────────

/// `on_failure` becomes a feedback redirect for *both* classes that
/// historically flowed through it — a verdict failure and a plain agent
/// failure took the same path in the v1 engine.
#[test]
fn legacy_on_failure_maps_to_verdict_and_agent_failure_redirects() {
    let policy = legacy_policy_for_step(&step(Some("s-implement"), Some(4)), None, None);
    for rule in [policy.verdict.as_ref(), policy.agent_failure.as_ref()] {
        let rule = rule.unwrap();
        assert_eq!(rule.strategy, RetryStrategy::Redirect);
        assert_eq!(rule.redirect_to.as_ref().unwrap().0, "s-implement");
        assert_eq!(rule.max_attempts, Some(4));
        assert!(rule.feedback);
    }
    let env = policy.environment.unwrap();
    assert_eq!(env.strategy, RetryStrategy::InPlace);
    assert_eq!(env.max_attempts, Some(ENV_MAX_ATTEMPTS));
    assert!(!env.feedback);
    assert_eq!(policy.non_retryable.unwrap().strategy, RetryStrategy::Fail);
}

/// No `on_failure` (or an empty one — the v1 empty-string check) means
/// the failing classes fail outright.
#[test]
fn legacy_without_on_failure_fails_the_failing_classes() {
    for step_conf in [step(None, None), step(Some(""), None)] {
        let policy = legacy_policy_for_step(&step_conf, None, None);
        assert_eq!(policy.verdict.unwrap().strategy, RetryStrategy::Fail);
        assert_eq!(policy.agent_failure.unwrap().strategy, RetryStrategy::Fail);
    }
}

/// The budget folds in the historical precedence: run override →
/// project default → step `max_iterations` → engine default 3.
#[test]
fn legacy_budget_precedence_matches_effective_loop_iterations() {
    let cases = [
        (Some(7), Some(5), Some(2), 7),
        (None, Some(5), Some(2), 5),
        (None, None, Some(2), 2),
        (None, None, None, DEFAULT_LOOP_ITERATIONS),
    ];
    for (run, project, step_max, expected) in cases {
        let policy = legacy_policy_for_step(&step(Some("s-implement"), step_max), run, project);
        assert_eq!(
            policy.verdict.unwrap().max_attempts,
            Some(expected),
            "run={run:?} project={project:?} step={step_max:?}"
        );
    }
}

// ── Evaluation: redirect strategy ──────────────────────────────────────

#[test]
fn redirect_within_budget_grants_the_retry() {
    let policy = legacy_policy_for_step(&step(Some("s-implement"), Some(3)), None, None);
    // No redirects consumed yet → attempt 1 of 3.
    let d = evaluate(&policy, FailureClass::Verdict, 0);
    assert_eq!(
        d.action,
        RetryAction::Redirect {
            target: crate::domain::ids::StepId::from("s-implement".to_string()),
            feedback: true
        }
    );
    assert_eq!(d.rule_id, "verdict.redirect");
    assert_eq!((d.attempt, d.max_attempts), (1, 3));

    // Last attempt in the budget is still granted (v1: already+1 > max).
    let d = evaluate(&policy, FailureClass::AgentFailure, 2);
    assert!(matches!(d.action, RetryAction::Redirect { .. }));
    assert_eq!(d.rule_id, "agent_failure.redirect");
    assert_eq!((d.attempt, d.max_attempts), (3, 3));
}

#[test]
fn redirect_over_budget_is_exhausted_and_names_the_target() {
    let policy = legacy_policy_for_step(&step(Some("s-implement"), Some(3)), None, None);
    let d = evaluate(&policy, FailureClass::Verdict, 3);
    assert_eq!(target(&d), Some("s-implement"));
    assert!(matches!(d.action, RetryAction::Exhausted { .. }));
    assert_eq!(d.rule_id, "verdict.redirect");
    assert_eq!((d.attempt, d.max_attempts), (4, 3));
}

/// A redirect rule with a missing or empty target degrades to a plain
/// fail — the budget is never consulted.
#[test]
fn redirect_without_target_fails() {
    let policy = RetryPolicy {
        verdict: Some(RetryRule {
            strategy: RetryStrategy::Redirect,
            max_attempts: Some(3),
            backoff_secs: None,
            feedback: true,
            redirect_to: None,
        }),
        ..Default::default()
    };
    let d = evaluate(&policy, FailureClass::Verdict, 0);
    assert_eq!(d.action, RetryAction::Fail);
    assert_eq!(d.rule_id, "verdict.redirect");
}

// ── Evaluation: in-place strategy ──────────────────────────────────────

/// The environment one-shot, verbatim: the first environment-classed
/// failure retries in place, the second fails.
#[test]
fn environment_grants_exactly_one_free_in_place_retry() {
    let policy = legacy_policy_for_step(&step(Some("s-implement"), Some(9)), None, None);

    let first = evaluate(&policy, FailureClass::Environment, 1);
    assert_eq!(first.action, RetryAction::RetryInPlace { feedback: false });
    assert_eq!(first.rule_id, "environment.in_place");
    assert_eq!((first.attempt, first.max_attempts), (2, ENV_MAX_ATTEMPTS));

    let second = evaluate(&policy, FailureClass::Environment, 2);
    assert_eq!(second.action, RetryAction::Exhausted { target: None });
}

/// Broken attempt accounting passes `u32::MAX`: the budget must read as
/// spent (never an unbounded in-place loop), and the arithmetic must
/// saturate instead of overflowing.
#[test]
fn broken_accounting_saturates_to_exhausted() {
    let policy = legacy_policy_for_step(&step(None, None), None, None);
    let d = evaluate(&policy, FailureClass::Environment, u32::MAX);
    assert_eq!(d.action, RetryAction::Exhausted { target: None });
    assert_eq!(d.attempt, u32::MAX);
}

/// An in-place rule without `max_attempts` uses the engine default.
#[test]
fn in_place_default_budget_is_the_engine_default() {
    let policy = RetryPolicy {
        agent_failure: Some(RetryRule {
            strategy: RetryStrategy::InPlace,
            max_attempts: None,
            backoff_secs: None,
            feedback: true,
            redirect_to: None,
        }),
        ..Default::default()
    };
    let d = evaluate(&policy, FailureClass::AgentFailure, 2);
    assert_eq!(d.action, RetryAction::RetryInPlace { feedback: true });
    assert_eq!(d.max_attempts, DEFAULT_LOOP_ITERATIONS);
    assert!(matches!(
        evaluate(&policy, FailureClass::AgentFailure, 3).action,
        RetryAction::Exhausted { target: None }
    ));
}

// ── Evaluation: fail strategy + floors ─────────────────────────────────

#[test]
fn non_retryable_always_fails() {
    let policy = legacy_policy_for_step(&step(Some("s-implement"), Some(9)), None, None);
    let d = evaluate(&policy, FailureClass::NonRetryable, 0);
    assert_eq!(d.action, RetryAction::Fail);
    assert_eq!(d.rule_id, "non_retryable.fail");
}

/// A class missing from the policy entirely falls to the safe floor:
/// fail, with a rule id that still names the class.
#[test]
fn missing_class_rule_fails_safely() {
    let policy = RetryPolicy::default();
    for (class, rule_id) in [
        (FailureClass::Environment, "environment.fail"),
        (FailureClass::Verdict, "verdict.fail"),
        (FailureClass::AgentFailure, "agent_failure.fail"),
        (FailureClass::NonRetryable, "non_retryable.fail"),
    ] {
        let d = evaluate(&policy, class, 0);
        assert_eq!(d.action, RetryAction::Fail);
        assert_eq!(d.rule_id, rule_id);
    }
}

/// The stored vocabulary and the enum can never drift.
#[test]
fn class_names_match_the_stored_error_class_vocabulary() {
    assert_eq!(FailureClass::Environment.as_str(), error_class::ENVIRONMENT);
    assert_eq!(FailureClass::Verdict.as_str(), error_class::VERDICT);
    assert_eq!(
        FailureClass::AgentFailure.as_str(),
        error_class::AGENT_FAILURE
    );
    assert_eq!(
        FailureClass::NonRetryable.as_str(),
        error_class::NON_RETRYABLE
    );
}
