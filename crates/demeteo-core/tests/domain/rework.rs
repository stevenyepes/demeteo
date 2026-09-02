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
