//! Coverage for the re-entry classification.
//!
//! The shape under test is the shipped pipeline's, because the distinction
//! that matters is a graph position and nothing else:
//!
//! ```text
//! research → spec → tickets → gate → implement → validate → critic
//! ```
//!
//! `tickets` produces the list, `implement` consumes it. `gate` is
//! downstream of `tickets` but *upstream of the consumer*; `validate` and
//! `critic` are downstream of the consumer. Every case below turns on which
//! side of `implement` the failing step is on — which is why the naive
//! "downstream of me" rule is the one thing these tests exist to refuse.

use super::*;
use crate::domain::models::workflow_v2::WorkflowDefinitionV2;

const IDS: [&str; 7] = [
    "research",
    "spec",
    "tickets",
    "gate",
    "implement",
    "validate",
    "critic",
];

/// The standard pipeline's shape, as a plain chain.
fn pipeline() -> WorkflowGraph {
    let nodes: Vec<serde_json::Value> = IDS
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "type": "agent",
                "title": id,
                "config": { "prompt_template": "p" }
            })
        })
        .collect();
    let edges: Vec<serde_json::Value> = IDS
        .windows(2)
        .map(|pair| serde_json::json!({ "from": pair[0], "to": pair[1] }))
        .collect();
    let def: WorkflowDefinitionV2 = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "wf-rework-test",
        "name": "Rework test",
        "nodes": nodes,
        "edges": edges
    }))
    .expect("fixture definition parses");
    WorkflowGraph::build(&def).expect("fixture graph builds")
}

/// The same pipeline as v1 step configs, so `task_list_consumer` has
/// something to read. Only `implement` binds a task list.
fn steps() -> Vec<StepConfig> {
    IDS.iter()
        .map(|id| {
            let mut step = StepConfig {
                id: StepId::from(*id),
                kind: "agent".to_string(),
                title: id.to_string(),
                ..Default::default()
            };
            if *id == "implement" {
                step.kind = "sequence".to_string();
                step.task_list_from = Some(StepId::from("tickets"));
            }
            step
        })
        .collect()
}

fn consumer() -> StepId {
    StepId::from("implement")
}

fn from(failing_step_id: &str) -> Option<RetryOrigin<'_>> {
    Some(RetryOrigin {
        failing_step_id,
        iteration: 1,
    })
}

// ---------- task_list_consumer ----------

#[test]
fn the_consumer_is_the_step_that_binds_the_list() {
    let steps = steps();
    assert_eq!(
        task_list_consumer(&steps, &"tickets".into()),
        Some(&StepId::from("implement"))
    );
}

#[test]
fn a_producer_nothing_binds_has_no_consumer() {
    let steps = steps();
    assert_eq!(task_list_consumer(&steps, &"spec".into()), None);
}

#[test]
fn an_empty_task_list_from_does_not_count_as_a_binding() {
    // The unset field serializes as an empty string in some hand-edited
    // definitions; treating that as a real binding would name a consumer
    // for every producer in the workflow.
    let mut steps = steps();
    steps[4].task_list_from = Some(StepId::from(""));
    assert_eq!(task_list_consumer(&steps, &"".into()), None);
}

// ---------- classify ----------

#[test]
fn no_retry_context_is_greenfield() {
    assert_eq!(
        classify(&pipeline(), &"tickets".into(), Some(&consumer()), None),
        ReworkMode::Greenfield
    );
}

#[test]
fn a_verdict_from_behind_the_consumer_is_rework() {
    // The case this module exists for: `validate` sits behind the step
    // that ran the decomposition, so that code is on the branch and the
    // decomposition step must emit a delta, not the whole list again.
    let g = pipeline();
    let c = consumer();
    assert_eq!(
        classify(&g, &"tickets".into(), Some(&c), from("validate")),
        ReworkMode::Rework
    );
    // Two steps further downstream is the same relationship.
    assert_eq!(
        classify(&g, &"tickets".into(), Some(&c), from("critic")),
        ReworkMode::Rework
    );
}

#[test]
fn a_gate_between_the_producer_and_the_consumer_is_revision() {
    // `gate` is downstream of `tickets` — so the naive "is the failure
    // downstream of me?" rule calls this rework — but it sits *in front
    // of* the consumer. A reviewer rejecting the decomposition there has
    // rejected a plan, not an implementation: the branch carries nothing,
    // and a delta list would emit tickets fixing code never written.
    assert_eq!(
        classify(
            &pipeline(),
            &"tickets".into(),
            Some(&consumer()),
            from("gate")
        ),
        ReworkMode::Revision
    );
}

#[test]
fn the_consumers_own_failure_is_revision() {
    // A sequence step that fails on its own rolls every task's commits
    // back on the way out, so the branch is at its pre-step tip and there
    // is nothing for a delta to build on.
    assert_eq!(
        classify(
            &pipeline(),
            &"tickets".into(),
            Some(&consumer()),
            from("implement")
        ),
        ReworkMode::Revision
    );
}

#[test]
fn a_steps_own_failure_is_revision() {
    assert_eq!(
        classify(
            &pipeline(),
            &"tickets".into(),
            Some(&consumer()),
            from("tickets")
        ),
        ReworkMode::Revision
    );
}

#[test]
fn a_producer_with_no_consumer_is_never_rework() {
    // Nothing in the workflow turns this node's output into commits, so
    // there is no implementation to emit a delta against.
    assert_eq!(
        classify(&pipeline(), &"spec".into(), None, from("validate")),
        ReworkMode::Revision
    );
}

#[test]
fn an_unnamed_failing_step_is_revision() {
    // The synthesized per-task context carries no failing step id. It
    // cannot be shown to be downstream, and the uncertain answer has to
    // be the one that re-runs rather than the one that skips.
    let g = pipeline();
    let c = consumer();
    assert_eq!(
        classify(&g, &"tickets".into(), Some(&c), from("")),
        ReworkMode::Revision
    );
    assert_eq!(
        classify(&g, &"tickets".into(), Some(&c), from("   ")),
        ReworkMode::Revision
    );
}

#[test]
fn a_failing_step_the_graph_does_not_contain_is_revision() {
    // Nothing can be proven about a node the graph never heard of, and
    // the safe floor applies: reissue the whole list rather than skip
    // work on a claim the graph cannot support.
    assert_eq!(
        classify(
            &pipeline(),
            &"tickets".into(),
            Some(&consumer()),
            from("ghost")
        ),
        ReworkMode::Revision
    );
}

#[test]
fn a_consumer_the_graph_does_not_contain_is_revision() {
    assert_eq!(
        classify(
            &pipeline(),
            &"tickets".into(),
            Some(&StepId::from("ghost")),
            from("validate")
        ),
        ReworkMode::Revision
    );
}

#[test]
fn mode_reports_itself() {
    assert_eq!(ReworkMode::Greenfield.as_str(), "greenfield");
    assert_eq!(ReworkMode::Revision.as_str(), "revision");
    assert_eq!(ReworkMode::Rework.as_str(), "rework");
    assert!(ReworkMode::Rework.is_rework());
    assert!(!ReworkMode::Revision.is_rework());
    assert!(!ReworkMode::Greenfield.is_rework());
}

// --- when the retry feedback goes stale --------------------------------------

/// No retry in flight: there is no loop to close and nothing to carry.
#[test]
fn no_retry_context_is_already_closed() {
    assert!(retry_loop_closed(None, "s-implement"));
}

/// The step that opened the loop succeeded — the feedback describes a
/// failure that no longer stands.
#[test]
fn the_failing_step_succeeding_closes_the_loop() {
    assert!(retry_loop_closed(Some("s-implement"), "s-implement"));
}

/// Everything between the redirect target and the failing step still needs
/// the feedback; clearing it here is how a re-run step goes in blind.
#[test]
fn an_intermediate_step_leaves_the_loop_open() {
    assert!(!retry_loop_closed(Some("s-validate"), "s-tickets"));
    assert!(!retry_loop_closed(Some("s-validate"), "s-implement"));
}

/// The legacy shape — a row from before the failing step was recorded —
/// clears after the next completed step rather than pinning the feedback to
/// a step id that was never written.
#[test]
fn an_unrecorded_failing_step_closes_after_the_next_completion() {
    assert!(retry_loop_closed(Some(""), "s-implement"));
}

// ---------- producer_fault_retry ----------
//
// A consumer that rejects its producer's list inside a rework cycle must
// not become the loop's origin: `classify` reads the consumer's own failure
// as revision, so the producer would re-decompose everything.

fn ctx(origin: &str, feedback: &str, iteration: u32, max: u32) -> RetryContext {
    RetryContext {
        feedback: feedback.to_string(),
        iteration,
        max,
        failing_tests: vec![format!("{origin}::t")],
        implicated_files: Vec::new(),
        failing_step_id: origin.to_string(),
    }
}

const REPLAY: &str = "this is the list I already ran last cycle";

#[test]
fn a_producer_fault_in_rework_keeps_the_verdict_that_opened_it() {
    let kept = producer_fault_retry(
        Some(ctx("critic", "the empty state is missing", 2, 3)),
        true,
        ctx("implement", REPLAY, 1, 2),
    );
    assert_eq!(kept.failing_step_id, "critic");
    assert!(
        kept.feedback.contains("the empty state is missing"),
        "{}",
        kept.feedback
    );
    assert!(kept.feedback.contains(REPLAY), "{}", kept.feedback);
    assert_eq!((kept.iteration, kept.max), (2, 3));
    assert_eq!(kept.failing_tests, ["critic::t"]);
    assert_eq!(
        classify(
            &pipeline(),
            &"tickets".into(),
            Some(&consumer()),
            from(&kept.failing_step_id)
        ),
        ReworkMode::Rework
    );
}

#[test]
fn outside_rework_a_producer_fault_originates_at_the_consumer() {
    let own = ctx("implement", "duplicate task id", 1, 2);
    assert_eq!(
        producer_fault_retry(Some(ctx("gate", "split it", 1, 1)), false, own.clone()),
        own
    );
    assert_eq!(producer_fault_retry(None, true, own.clone()), own);
    assert_eq!(
        producer_fault_retry(Some(ctx("", "legacy", 1, 1)), true, own.clone()),
        own
    );
}

/// Each fault replaces the last one's reason: the producer re-ran in
/// between, so the older reason is answered, and a stale "you replayed the
/// list" must not follow a producer that has since fixed it.
#[test]
fn a_second_producer_fault_replaces_the_first_reason() {
    let verdict = "the empty state is missing";
    let first = producer_fault_retry(
        Some(ctx("critic", verdict, 2, 3)),
        true,
        ctx("implement", REPLAY, 1, 2),
    );
    let second = producer_fault_retry(
        Some(first.clone()),
        true,
        ctx("implement", "duplicate task id 'fix-2'", 2, 2),
    );
    assert!(!second.feedback.contains(REPLAY), "{}", second.feedback);
    assert!(
        second.feedback.contains("duplicate task id"),
        "{}",
        second.feedback
    );
    assert_eq!(
        second.feedback.matches(verdict).count(),
        1,
        "{}",
        second.feedback
    );
    assert_eq!(
        second.feedback.len(),
        first.feedback.len() - REPLAY.len() + 25
    );
}

// ── restore_retry_context ─────────────────────────────────────────────────────

fn row(step_id: &str, status: &str, iteration_count: u32) -> StepExecution {
    use crate::domain::ids::{FeatureId, StepExecutionId};
    StepExecution {
        id: StepExecutionId::from(format!("se-f-1-{step_id}")),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: StepId::from(step_id),
        step_index: IDS.iter().position(|i| *i == step_id).unwrap_or(0) as u32,
        step_kind: "agent".to_string(),
        status: status.to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn persisted(failing_step_id: &str, iteration: u32) -> RetryContext {
    RetryContext {
        feedback: "3 tests fail in src/lib.rs".to_string(),
        iteration,
        max: 3,
        failing_tests: vec!["tests::a".to_string()],
        implicated_files: vec!["src/lib.rs".to_string()],
        failing_step_id: failing_step_id.to_string(),
    }
}

/// The restart the table exists for: `implement` was interrupted mid-rework
/// with `validate`'s verdict in flight, and comes back carrying all of it.
#[test]
fn an_open_loop_is_restored_whole() {
    let rows = [
        row("tickets", "completed", 0),
        row("implement", "interrupted", 0),
        row("validate", "pending", 2),
    ];
    assert_eq!(
        restore_retry_context(Some(persisted("validate", 2)), &rows, &pipeline()),
        Some(persisted("validate", 2))
    );
}

#[test]
fn nothing_persisted_restores_nothing() {
    assert_eq!(
        restore_retry_context(None, &[row("validate", "pending", 2)], &pipeline()),
        None
    );
}

/// The failing step completing is what closes the loop; a row outliving
/// that is a lost clear, and restoring it would reopen a closed loop.
#[test]
fn a_completed_origin_is_dropped() {
    assert_eq!(
        restore_retry_context(
            Some(persisted("validate", 2)),
            &[row("validate", "completed", 2)],
            &pipeline(),
        ),
        None
    );
}

/// A node the current workflow version no longer has cannot be shown to
/// be anywhere relative to the consumer.
#[test]
fn an_origin_the_graph_lacks_is_dropped() {
    assert_eq!(
        restore_retry_context(Some(persisted("s-gone", 2)), &[], &pipeline()),
        None
    );
}

#[test]
fn a_blank_origin_is_dropped() {
    assert_eq!(
        restore_retry_context(Some(persisted("  ", 2)), &[], &pipeline()),
        None
    );
}

/// The step row is written before the context row, so a crash between the
/// two leaves the row ahead; the prompt follows the larger.
#[test]
fn the_iteration_is_the_larger_of_the_row_and_the_budget_counter() {
    let restored = restore_retry_context(
        Some(persisted("validate", 2)),
        &[row("validate", "pending", 3)],
        &pipeline(),
    )
    .expect("open loop");
    assert_eq!(restored.iteration, 3);

    let restored = restore_retry_context(
        Some(persisted("validate", 2)),
        &[row("validate", "pending", 1)],
        &pipeline(),
    )
    .expect("open loop");
    assert_eq!(restored.iteration, 2);
}

/// A gate's context is persisted as `1 of 1`; restored over an origin whose
/// counter reads 3, the prompt must not say "attempt 3 of 1".
#[test]
fn a_restored_iteration_never_exceeds_its_max() {
    let gate_ctx = RetryContext {
        iteration: 1,
        max: 1,
        ..persisted("validate", 1)
    };
    let restored = restore_retry_context(
        Some(gate_ctx),
        &[row("validate", "pending", 3)],
        &pipeline(),
    )
    .expect("open loop");
    assert_eq!((restored.iteration, restored.max), (3, 3));
}

// ---------- gate_redirect_retry ----------
//
// A review gate between the producer and its consumer that redirects inside
// a rework cycle must not become the origin: `classify` reads it as
// revision, and the producer would re-decompose everything.

#[test]
fn a_gate_between_producer_and_consumer_keeps_the_verdict_that_opened_the_loop() {
    let kept = gate_redirect_retry(
        Some(ctx("critic", "the empty state is missing", 2, 3)),
        true,
        ctx("gate", "split the migration ticket", 1, 1),
    );
    assert_eq!(kept.failing_step_id, "critic");
    assert!(
        kept.feedback.contains("the empty state is missing"),
        "{}",
        kept.feedback
    );
    assert!(
        kept.feedback.contains("split the migration ticket"),
        "{}",
        kept.feedback
    );
    assert_eq!((kept.iteration, kept.max), (2, 3));
    assert_eq!(
        classify(
            &pipeline(),
            &"tickets".into(),
            Some(&consumer()),
            from(&kept.failing_step_id)
        ),
        ReworkMode::Rework
    );
}

#[test]
fn a_gate_outside_the_span_originates_at_the_gate() {
    let own = ctx("gate", "redo", 1, 1);
    assert_eq!(
        gate_redirect_retry(Some(ctx("critic", "verdict", 2, 3)), false, own.clone()),
        own
    );
}

#[test]
fn a_gate_with_nothing_in_flight_originates_at_the_gate() {
    let own = ctx("gate", "redo", 1, 1);
    assert_eq!(gate_redirect_retry(None, true, own.clone()), own);
    assert_eq!(
        gate_redirect_retry(Some(ctx(" ", "legacy", 1, 1)), true, own.clone()),
        own
    );
}

/// The same reviewer note given twice is one note, and a rejection the
/// producer has since answered does not survive the gate that reviewed its
/// next list.
#[test]
fn a_gate_neither_stacks_its_own_note_nor_keeps_an_answered_rejection() {
    let verdict = "the empty state is missing";
    let note = "split the migration ticket";
    let once = gate_redirect_retry(
        Some(ctx("critic", verdict, 2, 3)),
        true,
        ctx("gate", note, 1, 1),
    );
    let twice = gate_redirect_retry(Some(once.clone()), true, ctx("gate", note, 1, 1));
    assert_eq!(twice.feedback, once.feedback);

    let rejected = producer_fault_retry(Some(once.clone()), true, ctx("implement", REPLAY, 1, 2));
    let reviewed = gate_redirect_retry(Some(rejected), true, ctx("gate", note, 1, 1));
    assert_eq!(reviewed.feedback, once.feedback);
}
