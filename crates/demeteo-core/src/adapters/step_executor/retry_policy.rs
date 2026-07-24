//! Declarative per-class retry-policy evaluation (task P1.10, PRD §5.4).
//!
//! The engine has always *classified* failures — [`StepOutcome`] and
//! `VerifierError` carry the class — but the *response* was scattered
//! across `on_failure` goto edges, `max_iterations` precedence, the env
//! one-shot retry, and the engine default of 3. This module makes the
//! response a single declarative evaluation:
//!
//! ```text
//! failure class → policy rule → action (in_place | redirect | fail)
//! ```
//!
//! Two halves, both pure:
//!
//! - [`legacy_policy_for_step`] derives a **fully-resolved**
//!   [`RetryPolicy`] from a v1 [`StepConfig`] — the same mapping P1.2's
//!   `migrate_v1_to_v2` uses, with the run-time budget precedence
//!   (run override → project default → step `max_iterations` → engine
//!   default 3) folded in, exactly as `effective_loop_iterations`
//!   resolved it. When the driver walks native v2 graphs (P1.12), a v2
//!   deriver (node `retry` → workflow `defaults.retry` → these same
//!   engine defaults) replaces this one; [`evaluate`] stays.
//! - [`evaluate`] turns (policy, class, attempts consumed so far) into a
//!   [`RetryDecision`]. The decision's `rule_id` is recorded on the
//!   `step_attempts` row (`applied_rule`) so every failure in history
//!   names the policy rule that answered it (PRD §9 reliability metric).
//!
//! [`StepOutcome`]: super::steps::StepOutcome

use super::driver::resolution::resolve_loop_iterations;
use super::driver::DEFAULT_LOOP_ITERATIONS;
use crate::domain::ids::StepId;
use crate::domain::models::step_attempt::error_class;
use crate::domain::models::workflow_v2::{RetryPolicy, RetryRule, RetryStrategy};
use crate::domain::models::StepConfig;

/// Environment-class budget: the historical "one free in-place retry" —
/// two attempts of the node attributable to the environment, then fail.
pub(crate) const ENV_MAX_ATTEMPTS: u32 = 2;

/// Failure classes, mirroring [`error_class`]'s stored vocabulary 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    Environment,
    Verdict,
    AgentFailure,
    NonRetryable,
}

impl FailureClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FailureClass::Environment => error_class::ENVIRONMENT,
            FailureClass::Verdict => error_class::VERDICT,
            FailureClass::AgentFailure => error_class::AGENT_FAILURE,
            FailureClass::NonRetryable => error_class::NON_RETRYABLE,
        }
    }
}

/// What the driver should do about one failure occurrence.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RetryAction {
    /// Re-dispatch the same node.
    RetryInPlace { feedback: bool },
    /// Jump back to the (ancestor) target node.
    Redirect { target: StepId, feedback: bool },
    /// A retry rule existed but its attempt budget is spent. `target`
    /// is the redirect target when the exhausted rule was a redirect —
    /// the `RetryBudgetExhausted` notification/event names it.
    Exhausted { target: Option<StepId> },
    /// The policy says this class is not retried.
    Fail,
}

/// A fully-evaluated decision: the action plus the telemetry the driver
/// records and threads into `RetryContext`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RetryDecision {
    pub action: RetryAction,
    /// `<class>.<strategy>` (e.g. `verdict.redirect`,
    /// `environment.in_place`) — stored in `step_attempts.applied_rule`.
    pub rule_id: String,
    /// 1-based attempt number a granted retry would be starting.
    pub attempt: u32,
    /// Effective budget of the applied rule.
    pub max_attempts: u32,
}

/// Derive the fully-resolved policy for a v1 step. Behavior is the v1
/// engine's, verbatim:
///
/// - `verdict` / `agent_failure`: both classes flowed through the same
///   `on_failure` path — a redirect (with feedback) when the step
///   declares a non-empty target, a plain fail otherwise. The budget
///   folds in the historical precedence: run override → project default
///   → step `max_iterations` → [`DEFAULT_LOOP_ITERATIONS`].
/// - `environment`: one free in-place retry ([`ENV_MAX_ATTEMPTS`]),
///   never consuming the redirect budget, no feedback (the environment
///   can't read it).
/// - `non_retryable`: fail.
pub(crate) fn legacy_policy_for_step(
    step_conf: &StepConfig,
    run_override: Option<u32>,
    project_default: Option<u32>,
) -> RetryPolicy {
    let failed_rule = match step_conf.on_failure.as_ref().filter(|t| !t.0.is_empty()) {
        Some(target) => RetryRule {
            strategy: RetryStrategy::Redirect,
            max_attempts: Some(resolve_loop_iterations(
                run_override,
                project_default,
                step_conf.max_iterations,
            )),
            backoff_secs: None,
            feedback: true,
            redirect_to: Some(target.clone()),
        },
        None => fail_rule(),
    };

    RetryPolicy {
        environment: Some(RetryRule {
            strategy: RetryStrategy::InPlace,
            max_attempts: Some(ENV_MAX_ATTEMPTS),
            backoff_secs: None,
            feedback: false,
            redirect_to: None,
        }),
        verdict: Some(failed_rule.clone()),
        agent_failure: Some(failed_rule),
        non_retryable: Some(fail_rule()),
    }
}

fn fail_rule() -> RetryRule {
    RetryRule {
        strategy: RetryStrategy::Fail,
        max_attempts: None,
        backoff_secs: None,
        feedback: false,
        redirect_to: None,
    }
}

/// Evaluate one failure occurrence against a policy.
///
/// `attempts_used` counts what this class has already consumed,
/// *including the failure being evaluated* where the class's counter is
/// attempt-based:
///
/// - redirect rules: the step's `iteration_count` — redirect loops
///   granted so far (the current failure hasn't consumed one yet);
/// - in-place rules: failures of this class observed so far including
///   the current one (derived from `step_attempts.error_class`).
///
/// Both meet the same arithmetic: a retry is granted while
/// `attempts_used + 1 <= max_attempts`. Callers with broken attempt
/// accounting pass `u32::MAX` to force exhaustion (never an unbounded
/// in-place loop); the addition saturates.
///
/// Total: a class missing from the policy fails (derivers always
/// produce complete policies; this is the safe floor, not a path the
/// legacy deriver can reach).
pub(crate) fn evaluate(
    policy: &RetryPolicy,
    class: FailureClass,
    attempts_used: u32,
) -> RetryDecision {
    let rule = match class {
        FailureClass::Environment => policy.environment.as_ref(),
        FailureClass::Verdict => policy.verdict.as_ref(),
        FailureClass::AgentFailure => policy.agent_failure.as_ref(),
        FailureClass::NonRetryable => policy.non_retryable.as_ref(),
    };

    let (strategy, rule) = match rule {
        Some(r) => (r.strategy, r),
        None => {
            return RetryDecision {
                action: RetryAction::Fail,
                rule_id: format!("{}.fail", class.as_str()),
                attempt: attempts_used.saturating_add(1),
                max_attempts: 0,
            }
        }
    };

    let strategy_name = match strategy {
        RetryStrategy::InPlace => "in_place",
        RetryStrategy::Redirect => "redirect",
        RetryStrategy::Fail => "fail",
    };
    let rule_id = format!("{}.{}", class.as_str(), strategy_name);
    let max = rule.max_attempts.unwrap_or(DEFAULT_LOOP_ITERATIONS);
    let attempt = attempts_used.saturating_add(1);

    let action = match strategy {
        RetryStrategy::Fail => RetryAction::Fail,
        RetryStrategy::Redirect => {
            // A redirect with no (or an empty) target degrades to a plain
            // fail, mirroring the v1 empty-`on_failure` check.
            match rule.redirect_to.as_ref().filter(|t| !t.0.is_empty()) {
                None => RetryAction::Fail,
                Some(target) if attempt > max => RetryAction::Exhausted {
                    target: Some(target.clone()),
                },
                Some(target) => RetryAction::Redirect {
                    target: target.clone(),
                    feedback: rule.feedback,
                },
            }
        }
        RetryStrategy::InPlace => {
            if attempt > max {
                RetryAction::Exhausted { target: None }
            } else {
                RetryAction::RetryInPlace {
                    feedback: rule.feedback,
                }
            }
        }
    };

    RetryDecision {
        action,
        rule_id,
        attempt,
        max_attempts: max,
    }
}

#[cfg(test)]
#[path = "../../../tests/adapters/step_executor/retry_policy_tests.rs"]
mod retry_policy_tests;
