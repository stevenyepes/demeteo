//! The two rewind decisions in `replay_steps_from`, pinned without a
//! `DagStepExecutor`.
//!
//! Both are reachable only through a call that ends in
//! `start_execution_loop`, which needs a resolved repo and a live driver —
//! so every earlier attempt to cover them ran the rewind and then had the
//! rollback undo it, leaving the interesting field unobserved. As free
//! functions they answer directly.

use super::{rewind_patch, unwind_patch};
use crate::domain::ids::{FeatureId, StepExecutionId, StepId};
use crate::domain::models::StepExecution;

fn step_with(status: &str, iteration_count: u32) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-1".to_string()),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: StepId::from("s-implement".to_string()),
        step_index: 5,
        step_kind: "sequence".to_string(),
        status: status.to_string(),
        cost_usd: Some(10.76),
        tokens: Some(110_846),
        wall_clock_secs: Some(42),
        artifact_path: Some("/w/code-diff.diff".to_string()),
        artifact_paths: vec!["/w/code-diff.diff".to_string()],
        error_message: Some("could not read a task list".to_string()),
        iteration_count,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// The regression this exists for. A step that spent its whole redirect
/// budget and was then rewound by a human pressing Retry or Replay must
/// come back with a fresh one: the retry policy compares
/// `iteration_count + 1` against the budget, so carrying 5-of-5 across the
/// rewind means the very first failure answers `Exhausted` and the run
/// dies without ever redirecting to its `on_failure` target.
#[test]
fn a_rewind_clears_the_spent_redirect_budget() {
    let patch = rewind_patch(&step_with("failed", 5));
    assert_eq!(
        patch.iteration_count,
        Some(0),
        "a rewound step must start its redirect budget over, not inherit a spent one"
    );
    assert_eq!(patch.status.as_deref(), Some("pending"));
}

/// A step that never redirected still gets an explicit zero rather than
/// `None` — "leave it alone" and "set it to what it already is" are the
/// same write here, and the unconditional form is what makes the reset
/// impossible to skip for the one row that needed it.
#[test]
fn a_rewind_writes_the_budget_even_when_it_was_already_clean() {
    assert_eq!(
        rewind_patch(&step_with("completed", 0)).iteration_count,
        Some(0)
    );
}

/// Spend is not refunded by a rewind: the run really did burn it, and the
/// feature-level total is assembled from these rows.
#[test]
fn a_rewind_preserves_what_the_step_already_spent() {
    let patch = rewind_patch(&step_with("failed", 5));
    assert_eq!(patch.cost_usd, Some(Some(10.76)));
    assert_eq!(patch.tokens, Some(Some(110_846)));
    assert_eq!(patch.wall_clock_secs, Some(Some(42)));
    // The failure text and artifacts *are* dropped — they describe the
    // attempt being rewound away.
    assert_eq!(patch.error_message, Some(None));
    assert_eq!(patch.artifact_paths, Some(Vec::new()));
}

/// The mirror image: when arming the driver fails, the rewind is undone in
/// full. Restoring `pending`/0 instead would hand the run a set of retries
/// it was never granted — the same over-permissive end state the reset
/// above is careful to grant only on a rewind that actually happened.
#[test]
fn a_failed_arm_puts_the_spent_budget_back() {
    let patch = unwind_patch("failed", 5);
    assert_eq!(patch.iteration_count, Some(5));
    assert_eq!(patch.status.as_deref(), Some("failed"));
    // Spend was never touched on the way in, so it must not be touched on
    // the way out either.
    assert_eq!(patch.cost_usd, None);
    assert_eq!(patch.tokens, None);
}
