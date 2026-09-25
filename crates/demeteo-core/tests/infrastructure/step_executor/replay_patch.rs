//! The two rewind decisions in `replay_steps_from`, pinned without a
//! `DagStepExecutor`.
//!
//! Both are reachable only through a call that ends in
//! `start_execution_loop`, which needs a resolved repo and a live driver —
//! so every earlier attempt to cover them ran the rewind and then had the
//! rollback undo it, leaving the interesting field unobserved. As free
//! functions they answer directly.

use super::{can_restore_successful_ancestor, rewind_patch, unwind_patch};
use crate::domain::ids::{FeatureId, StepExecutionId, StepId};
use crate::domain::models::{StepAttempt, StepExecution};
use crate::ports::db::{FeaturePatch, StepExecutionPatch};

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

/// A step's assignment pin ([`crate::domain::step_assignment`]) has to
/// survive the node being re-entered — a human pressing Retry after
/// swapping the agent must get the agent they swapped to, not the one that
/// just failed. It does, because the pin is a tier-1 entry on the
/// `features` row and this patch reaches only `step_executions`. This is
/// what says so out loud, so that opening a route appears here as a failure
/// rather than as a Retry that quietly reverts the user's choice. The
/// rewind's own writes to the `features` row are the test below.
///
/// The assertion is over the whole patch, not over the fields the rewind is
/// known to set, because the claim is about what it does *not* set:
/// `expected` spells the seven writes and leaves the rest to `Default`, so
/// a write to any other field — an assignment-bearing one added later
/// included — renders on one side only. `StepExecutionPatch` derives
/// `Debug` but not `PartialEq`; the rendering is the structural comparison.
#[test]
fn a_rewind_writes_no_assignment_bearing_field() {
    let expected = StepExecutionPatch {
        status: Some("pending".to_string()),
        iteration_count: Some(0),
        cost_usd: Some(Some(10.76)),
        tokens: Some(Some(110_846)),
        wall_clock_secs: Some(Some(42)),
        artifact_paths: Some(Vec::new()),
        error_message: Some(None),
        ..Default::default()
    };

    assert_eq!(
        format!("{:?}", rewind_patch(&step_with("failed", 5))),
        format!("{:?}", expected),
        "a rewind writes these `step_executions` fields and no others; anything \
         further is either assignment-bearing — and then a re-entry discards the \
         user's pin — or a deliberate widening that belongs in `expected` too"
    );
}

/// The other half of that claim, and the half with a real route to the pin:
/// the rewind writes the `features` row too — three times, for the tier-2
/// re-pin and for the status either side of arming the driver — and every
/// one of them spells `..Default::default()`. The repo emits
/// `step_overrides_json=?` only for a `Some`, so a `Default` that grew a
/// `Some(vec![])` would turn all three into a silent wipe of the user's
/// pins without a single call site being edited — and an empty vec *is*
/// this field's cleared state, which is what makes that edit look
/// reasonable.
#[test]
fn the_feature_row_writes_on_the_rewind_path_carry_no_assignment() {
    assert!(
        FeaturePatch::default().step_overrides.is_none(),
        "`replay_steps_from` keeps the pin only by leaving `step_overrides` to \
         `Default`; anything but `None` here writes the column from every \
         `..Default::default()` on the rewind path"
    );
}

fn attempt(status: &str) -> StepAttempt {
    StepAttempt {
        step_execution_id: StepExecutionId::from("se-1".to_string()),
        attempt_no: 1,
        status: status.to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_ms: None,
        error_class: None,
        failure_fingerprint: None,
        applied_rule: None,
        workspace_fingerprint: None,
        idempotency_key: None,
        started_at: 0,
        ended_at: Some(1),
    }
}

#[test]
fn downstream_replay_restores_only_an_ancestor_with_a_prior_success() {
    let failed = step_with("failed", 0);
    assert!(can_restore_successful_ancestor(
        &failed,
        &[attempt("completed"), attempt("failed")]
    ));
    assert!(!can_restore_successful_ancestor(
        &failed,
        &[attempt("failed")]
    ));
    assert!(!can_restore_successful_ancestor(
        &step_with("interrupted", 0),
        &[attempt("completed")]
    ));
}
