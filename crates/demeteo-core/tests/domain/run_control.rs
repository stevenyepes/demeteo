//! The refusals, reached directly: no `DagStepExecutor`, no ports, no runtime.
//!
//! Every expected string below is the literal the adapter produced before the
//! rule moved here. They are pasted, not derived — four of the five shadow
//! messages share a prefix and differ only in the tail, and reproducing them
//! from the enum is exactly where a "harmless" wording tidy-up would hide.

use super::*;
use crate::domain::ids::{FeatureId, StepExecutionId, StepId};

fn step(step_id: &str, index: u32, status: &str) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from(format!("se-f-1-{step_id}")),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: StepId::from(step_id.to_string()),
        step_index: index,
        step_kind: "agent".to_string(),
        status: status.to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn ancestor_set(ids: &[&str]) -> HashSet<StepId> {
    ids.iter().map(|s| StepId::from(s.to_string())).collect()
}

// ── C4.2: a shadow is never driven from here ─────────────────────────────────

/// One assertion per action, against the exact sentence that action's call
/// site produced. Each tail names the remote route the user should take
/// instead; a tail that lost its route is a user told "no" and nothing else.
#[test]
fn every_action_refuses_a_shadow_in_the_words_its_call_site_used() {
    assert_eq!(
        shadow_refusal(RunAction::Drive, "f-123"),
        "Feature 'f-123' is a read-only shadow of a run owned by a demeteo-runner; \
         this machine never drives it (decide its gates via the remote run instead)"
    );
    assert_eq!(
        shadow_refusal(RunAction::Cancel, "f-123"),
        "Feature 'f-123' is a read-only shadow of a run owned by a demeteo-runner; \
         cancel it on the runner (remote_cancel_run), not locally"
    );
    assert_eq!(
        shadow_refusal(RunAction::Retry, "f-123"),
        "Feature 'f-123' is a read-only shadow of a run owned by a demeteo-runner; \
         this machine cannot retry its steps"
    );
    assert_eq!(
        shadow_refusal(RunAction::DecideGate, "f-123"),
        "Feature 'f-123' is a read-only shadow of a run owned by a demeteo-runner; \
         decide this gate on the runner (remote_decide_gate), not locally"
    );
    assert_eq!(
        shadow_refusal(RunAction::Replay, "f-123"),
        "Feature 'f-123' is a read-only shadow of a run owned by a demeteo-runner; \
         replay it on the runner, not here"
    );
}

/// The five share one prefix, and that is the part a reader recognises before
/// they have read the tail. A drift in it would go unnoticed in any single
/// message.
#[test]
fn the_five_refusals_share_one_prefix_and_differ_only_after_it() {
    const PREFIX: &str = "Feature 'f-9' is a read-only shadow of a run owned by a demeteo-runner; ";
    let tails: Vec<String> = [
        RunAction::Drive,
        RunAction::Cancel,
        RunAction::Retry,
        RunAction::DecideGate,
        RunAction::Replay,
    ]
    .into_iter()
    .map(|a| {
        let msg = shadow_refusal(a, "f-9");
        assert!(
            msg.starts_with(PREFIX),
            "{a:?} lost the shared prefix: {msg}"
        );
        msg[PREFIX.len()..].to_string()
    })
    .collect();

    let mut sorted = tails.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), tails.len(), "two actions refuse identically");
}

// ── Which statuses have a retry in them ──────────────────────────────────────

/// A retry rewinds the row and re-arms the driver, so it accepts a step that
/// already stopped and one that never started — and nothing in flight.
#[test]
fn only_a_stopped_or_unstarted_step_may_be_retried() {
    for permitted in ["failed", "interrupted", "pending"] {
        assert_eq!(
            retry_refusal(permitted),
            None,
            "{permitted} must be retryable"
        );
    }
}

#[test]
fn a_step_in_flight_or_already_done_is_refused_in_its_own_words() {
    assert_eq!(
        retry_refusal("running"),
        Some(
            "Cannot retry a step in 'running' status. \
             Only failed or interrupted steps can be retried."
                .to_string()
        )
    );
    for refused in ["verifying", "awaiting_gate", "completed", "cancelled"] {
        assert!(
            retry_refusal(refused).is_some_and(|m| m.contains(refused)),
            "{refused} must be refused, naming itself"
        );
    }
}

// ── Nothing upstream may still be working ────────────────────────────────────

const BLOCKING: [&str; 4] = ["pending", "running", "verifying", "awaiting_gate"];

/// The explicit-ancestor case: only the upstream cone blocks, so an
/// independent branch running in parallel is not a reason to refuse. That is
/// the whole difference between a DAG and the chain this guard used to assume.
#[test]
fn only_a_graph_ancestor_blocks_when_the_graph_resolved() {
    let target = step("implement", 2, "failed");
    let siblings = vec![
        step("spec", 0, "running"),
        step("unrelated", 1, "running"),
        target.clone(),
    ];
    let ancestors = ancestor_set(&["spec"]);

    assert_eq!(
        active_predecessor_refusal(&target, &siblings, Some(&ancestors), "retrying this step"),
        Some(
            "Step 'spec' is still running; wait for it to finish before retrying this step."
                .to_string()
        )
    );

    let unrelated_only = ancestor_set(&["nothing-here"]);
    assert_eq!(
        active_predecessor_refusal(
            &target,
            &siblings,
            Some(&unrelated_only),
            "retrying this step"
        ),
        None,
        "a sibling on an independent branch must not block"
    );
}

/// The fallback the guard exists to have: an unresolvable graph blocks on the
/// index ordering rather than failing open. A legacy feature with no matching
/// workflow still gets a guard.
#[test]
fn an_unresolvable_graph_falls_back_to_the_index_rather_than_failing_open() {
    let target = step("implement", 2, "failed");
    let siblings = vec![
        step("spec", 0, "running"),
        step("later", 3, "running"),
        target.clone(),
    ];

    assert_eq!(
        active_predecessor_refusal(&target, &siblings, None, "deciding this gate"),
        Some(
            "Step 'spec' is still running; wait for it to finish before deciding this gate."
                .to_string()
        ),
        "a lower-index step still blocks with no graph"
    );

    let only_later = vec![step("later", 3, "running"), target.clone()];
    assert_eq!(
        active_predecessor_refusal(&target, &only_later, None, "deciding this gate"),
        None,
        "a higher-index step is downstream and must not block"
    );
}

/// All four non-terminal statuses block, and nothing else does. `pending` is
/// the one that looks harmless: it is a step the scheduler has not reached,
/// which is exactly the dependency a retry would race.
#[test]
fn every_non_terminal_ancestor_status_blocks_and_no_other_does() {
    let target = step("implement", 1, "failed");
    let ancestors = ancestor_set(&["spec"]);

    for status in BLOCKING {
        let siblings = vec![step("spec", 0, status), target.clone()];
        assert_eq!(
            active_predecessor_refusal(&target, &siblings, Some(&ancestors), "retrying this step"),
            Some(format!(
                "Step 'spec' is still {status}; wait for it to finish before retrying this step."
            )),
        );
    }

    for status in ["completed", "failed", "interrupted", "cancelled", "skipped"] {
        let siblings = vec![step("spec", 0, status), target.clone()];
        assert_eq!(
            active_predecessor_refusal(&target, &siblings, Some(&ancestors), "retrying this step"),
            None,
            "a terminal ancestor in {status} must not block"
        );
    }
}

/// The target is skipped by row id, not by step id — a step is never its own
/// blocking ancestor, and a `pending` target is a legitimate retry.
#[test]
fn the_target_never_blocks_itself() {
    let target = step("implement", 0, "pending");
    let ancestors = ancestor_set(&["implement"]);

    assert_eq!(
        active_predecessor_refusal(
            &target,
            std::slice::from_ref(&target),
            Some(&ancestors),
            "retrying this step"
        ),
        None
    );
}

/// The first blocking ancestor answers; the sibling order is the row order the
/// caller read, which is the order the user sees in the timeline.
#[test]
fn the_first_blocking_ancestor_in_row_order_is_the_one_named() {
    let target = step("implement", 3, "failed");
    let siblings = vec![
        step("spec", 0, "completed"),
        step("plan", 1, "running"),
        step("review", 2, "awaiting_gate"),
        target.clone(),
    ];

    assert_eq!(
        active_predecessor_refusal(&target, &siblings, None, "retrying this step"),
        Some(
            "Step 'plan' is still running; wait for it to finish before retrying this step."
                .to_string()
        )
    );
}

/// The manual sync writes a real step row so the inspector can stream it, and
/// the inspector then offers it every action a node gets. Retry and Replay make
/// their target the pivot of a graph walk this row is not in, so both fall back
/// to comparing `step_index` against `u32::MAX`: the rewind takes only this row
/// and every real node reads as an ancestor to be restored. The refusal is the
/// only thing between that and a finished run being re-armed.
#[test]
fn an_out_of_band_row_is_neither_retried_nor_replayed_from() {
    use crate::domain::run_control::out_of_band_refusal;
    use crate::domain::step_seed::MANUAL_SYNC_STEP_ID;

    for action in [RunAction::Retry, RunAction::Replay] {
        let refusal = out_of_band_refusal(action, MANUAL_SYNC_STEP_ID)
            .unwrap_or_else(|| panic!("{action:?} was allowed on the manual sync row"));
        assert!(refusal.contains(MANUAL_SYNC_STEP_ID), "{refusal}");
        assert!(refusal.contains("sync banner"), "{refusal}");
        assert_eq!(out_of_band_refusal(action, "s-implement"), None);
        assert_eq!(out_of_band_refusal(action, "s-sync"), None);
    }
}
