// Tests for `classify` in `src/adapters/step_executor/driver/run_loop/attempt.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// Pure policy — no ports, no runtime, no driver. It lives under
// `tests/infrastructure/` rather than `tests/domain/` only because
// `StepOutcome` is an adapter type (`step_executor::steps`), and a domain
// module reading it would invert the hexagon.
//
// The `Failed`-under-cancel arm is the one that matters: the step row calls
// that `interrupted`, and an attempt row that called it `failed` would tell
// the retry-policy audit a user's cancel was the agent's fault.

use super::*;
use crate::domain::models::step_attempt::error_class;
use crate::domain::verifier::VerdictFailure;

const WT: &str = "/home/u/wt/f-1";

fn classify_of(outcome: &StepOutcome) -> AttemptClassification {
    classify(outcome, false, false, WT)
}

// ── the terminal non-failures ───────────────────────────────────────────────

/// A completed attempt has nothing to classify and nothing to fingerprint —
/// a class on a green row would put it in the retry-policy's vocabulary.
#[test]
fn a_completed_attempt_carries_no_class_and_no_fingerprint() {
    let c = classify_of(&StepOutcome::Completed);
    assert_eq!(c.status, "completed");
    assert_eq!(c.error_class, None);
    assert_eq!(c.fingerprint, None);
}

/// A cancel is the user's decision, not a failure: no class, so no rule can
/// be evaluated against it later.
#[test]
fn a_cancelled_attempt_carries_no_class() {
    let c = classify_of(&StepOutcome::Cancelled);
    assert_eq!(c.status, "cancelled");
    assert_eq!(c.error_class, None);
    assert_eq!(c.fingerprint, None);
}

/// A gate redirect ends the attempt without anything having gone wrong.
#[test]
fn a_redirect_is_its_own_status_and_not_a_failure() {
    let c = classify_of(&StepOutcome::RedirectTo(2));
    assert_eq!(c.status, "redirected");
    assert_eq!(c.error_class, None);
    assert_eq!(c.fingerprint, None);
}

// ── the failure classes ─────────────────────────────────────────────────────

/// A plain failure with no structured verdict behind it is the agent's.
#[test]
fn a_bare_failure_is_an_agent_failure() {
    let c = classify_of(&StepOutcome::Failed("the build broke".to_string()));
    assert_eq!(c.status, "failed");
    assert_eq!(c.error_class, Some(error_class::AGENT_FAILURE));
    assert!(c.fingerprint.is_some());
}

/// The same message becomes a `verdict` when a structured verifier failure
/// rode alongside it. `dispatch_step` normalizes `VerdictFailed` into
/// `Failed` and keeps the structure aside, so this flag is the *only*
/// surviving evidence of which one it was.
#[test]
fn a_failure_with_a_verdict_behind_it_is_a_verdict() {
    let c = classify(
        &StepOutcome::Failed("the build broke".to_string()),
        true,
        false,
        WT,
    );
    assert_eq!(c.error_class, Some(error_class::VERDICT));
}

/// The pre-normalization variant classifies identically, fingerprinted off
/// the rendered feedback rather than the raw reason.
#[test]
fn an_un_normalized_verdict_failure_classifies_as_a_verdict() {
    let vf = VerdictFailure::from_reason("two suites are red");
    let c = classify_of(&StepOutcome::VerdictFailed(vf.clone()));
    assert_eq!(c.status, "failed");
    assert_eq!(c.error_class, Some(error_class::VERDICT));
    assert_eq!(
        c.fingerprint,
        classify_of(&StepOutcome::Failed(vf.to_feedback())).fingerprint,
        "the same failure fingerprints the same either side of normalization"
    );
}

/// The environment class is what earns the one free in-place retry, and
/// what must never consume the redirect budget.
#[test]
fn an_environmental_failure_is_classed_environment() {
    let c = classify_of(&StepOutcome::Environmental(
        "gdk-3.0 is missing".to_string(),
    ));
    assert_eq!(c.status, "failed");
    assert_eq!(c.error_class, Some(error_class::ENVIRONMENT));
}

/// Non-retryable is a terminal class: the policy consults no budget for it.
#[test]
fn a_non_retryable_failure_is_classed_non_retryable() {
    let c = classify_of(&StepOutcome::NonRetryable("verifier never ran".to_string()));
    assert_eq!(c.status, "failed");
    assert_eq!(c.error_class, Some(error_class::NON_RETRYABLE));
}

// ── the cancel that arrives mid-failure ─────────────────────────────────────

/// A step that was already failing when the user cancelled is recorded as
/// `interrupted` on the step row; the attempt row has to agree, or the audit
/// reads a user's stop as an agent's defect. Only the status moves — the
/// class and the fingerprint still describe what actually went wrong.
#[test]
fn a_failure_during_a_cancel_is_recorded_as_interrupted() {
    let c = classify(
        &StepOutcome::Failed("the build broke".to_string()),
        false,
        true,
        WT,
    );
    assert_eq!(c.status, "interrupted");
    assert_eq!(c.error_class, Some(error_class::AGENT_FAILURE));
    assert!(c.fingerprint.is_some());
}

/// The cancel flag reaches only the `Failed` arm. An environmental failure
/// under a cancel is still `failed` — the environment broke regardless of
/// what the user did next.
#[test]
fn the_cancel_flag_does_not_reach_the_other_failure_arms() {
    let c = classify(
        &StepOutcome::Environmental("gdk-3.0 is missing".to_string()),
        false,
        true,
        WT,
    );
    assert_eq!(c.status, "failed");
}

// ── the fingerprint ─────────────────────────────────────────────────────────

/// The fingerprint is what makes two attempts the *same* failure, so the
/// worktree path — which differs per feature and per retry — has to be
/// masked out of it before the comparison.
#[test]
fn the_fingerprint_is_normalized_against_the_worktree() {
    let msg = format!("error at {WT}/src/main.rs:12");
    let here = classify(&StepOutcome::Failed(msg.clone()), false, false, WT);
    let elsewhere = classify(
        &StepOutcome::Failed(msg.replace(WT, "/other/wt/f-2")),
        false,
        false,
        "/other/wt/f-2",
    );
    assert_eq!(here.fingerprint, elsewhere.fingerprint);
}
