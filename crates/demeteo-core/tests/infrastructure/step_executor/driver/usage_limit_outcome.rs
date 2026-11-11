// Tests for `usage_limit_outcome` in
// `src/adapters/step_executor/driver/run_loop/dispatch.rs` (mirrored-tests
// convention). `super` resolves to that module.

use super::*;

const CODEX: &str = "sequence task 'instance-metadata-projection': agent error: \
    You\u{2019}ve hit your usage limit. Upgrade to Pro (https://chatgpt.com/explore/pro), \
    visit https://chatgpt.com/settings/usage to purchase more credits or try again at 7:26 PM.";

fn usage_limit_text(outcome: &StepOutcome) -> Option<&str> {
    match outcome {
        StepOutcome::UsageLimit(msg) => Some(msg),
        _ => None,
    }
}

/// The failure that spent a real run's redirect budget: a codex quota
/// refusal, arriving as an ordinary agent error from a sequence ticket.
#[test]
fn a_codex_quota_refusal_becomes_a_usage_limit() {
    let out = usage_limit_outcome(StepOutcome::Failed(CODEX.to_string()), false);
    let msg = usage_limit_text(&out).expect("usage limit");
    assert!(msg.contains("try again at 7:26 PM"), "{msg}");
}

#[test]
fn an_environmental_quota_refusal_becomes_a_usage_limit() {
    let out = usage_limit_outcome(StepOutcome::Environmental(CODEX.to_string()), false);
    assert!(usage_limit_text(&out).is_some());
}

/// Any other failure keeps the class it arrived in — an environment failure
/// in particular keeps its in-place retry.
#[test]
fn other_failures_keep_their_class() {
    let wall = "agent error: Agent step exceeded wall clock cap (1800s / 1800s elapsed)";
    assert!(matches!(
        usage_limit_outcome(StepOutcome::Environmental(wall.to_string()), false),
        StepOutcome::Environmental(m) if m == wall
    ));
    assert!(matches!(
        usage_limit_outcome(StepOutcome::Failed(wall.to_string()), false),
        StepOutcome::Failed(m) if m == wall
    ));
    assert!(matches!(
        usage_limit_outcome(StepOutcome::NonRetryable(CODEX.to_string()), false),
        StepOutcome::NonRetryable(_)
    ));
}

/// A verdict's text is the project's own test output; a test named for rate
/// limiting must not end the run as the provider's refusal.
#[test]
fn a_verdict_is_never_read_as_a_usage_limit() {
    let feedback = "FAIL tests/api.rs::rate limit exceeded returns 429";
    assert!(matches!(
        usage_limit_outcome(StepOutcome::Failed(feedback.to_string()), true),
        StepOutcome::Failed(_)
    ));
}
