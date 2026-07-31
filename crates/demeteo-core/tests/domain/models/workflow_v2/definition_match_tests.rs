// Tests for `definition_matches_steps` in
// `crates/demeteo-core/src/domain/models/workflow_v2.rs` (mirrored-tests
// convention). `super` = that module.
//
// A two-line predicate that decides the topology of an entire run: read a
// diamond back through `steps` alone and the engine runs it as a line. It was
// inline in a 305-line `async fn` and unreachable without a WorkflowRepository
// double.

use super::*;
use crate::domain::models::workflow::StepConfig;

fn node(id: &str) -> NodeConfig {
    NodeConfig {
        id: StepId::from(id.to_string()),
        node_type: "agent".to_string(),
        type_version: 1,
        title: id.to_string(),
        config: serde_json::json!({}),
        retry: None,
        join: None,
        position: None,
    }
}

fn definition(node_ids: &[&str]) -> WorkflowDefinitionV2 {
    WorkflowDefinitionV2 {
        schema_version: WORKFLOW_SCHEMA_V2,
        id: WorkflowId::from("wf-1".to_string()),
        name: "wf".to_string(),
        nodes: node_ids.iter().map(|i| node(i)).collect(),
        edges: Vec::new(),
        defaults: WorkflowDefaults::default(),
    }
}

fn steps(step_ids: &[&str]) -> Vec<StepConfig> {
    step_ids
        .iter()
        .map(|i| StepConfig {
            id: StepId::from(i.to_string()),
            kind: "agent".to_string(),
            title: i.to_string(),
            ..StepConfig::default()
        })
        .collect()
}

/// The ordinary case: both representations were written together, so they
/// agree and the stored document — which owns the edges — is used.
#[test]
fn equal_id_sets_match() {
    assert!(definition_matches_steps(
        &definition(&["spec", "implement", "review"]),
        &steps(&["spec", "implement", "review"]),
    ));
}

/// Order is not part of the question. The document owns the edges; `steps` is a
/// config list, and a differently-ordered list of the same ids describes the
/// same run.
#[test]
fn order_does_not_matter() {
    assert!(definition_matches_steps(
        &definition(&["review", "spec", "implement"]),
        &steps(&["spec", "implement", "review"]),
    ));
}

/// Same count, one node renamed. The document names a node the engine has no
/// config for, which would fail the whole run at schedule time — so the
/// migration is used instead.
#[test]
fn a_renamed_node_does_not_match() {
    assert!(!definition_matches_steps(
        &definition(&["spec", "impl", "review"]),
        &steps(&["spec", "implement", "review"]),
    ));
}

/// A node the step list does not have.
#[test]
fn an_extra_node_does_not_match() {
    assert!(!definition_matches_steps(
        &definition(&["spec", "implement", "review"]),
        &steps(&["spec", "implement"]),
    ));
}

/// The case the length check exists for, and the one the membership check
/// alone cannot see: every node is present in `steps`, but a step has no node.
/// Without the length check this passes and the run silently drops a step.
#[test]
fn an_extra_step_does_not_match() {
    assert!(!definition_matches_steps(
        &definition(&["spec", "implement"]),
        &steps(&["spec", "implement", "review"]),
    ));
}

/// Two empty sets agree — a degenerate workflow is caught by the caller's own
/// "Workflow has no steps." check, not by this one.
#[test]
fn two_empty_sets_agree() {
    assert!(definition_matches_steps(&definition(&[]), &steps(&[])));
}
