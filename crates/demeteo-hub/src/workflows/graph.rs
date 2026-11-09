//! A starter's graph, derived from its v1 step list the way core's
//! `migrate_v1_to_v2` derives it (`crates/demeteo-core/src/domain/models/workflow_migrate.rs`).
//! The Hub does not link core, so this is a second copy of that rule, and it
//! is only trustworthy while `src-tauri/tests/hub_starters.rs` holds the two
//! equal over every starter. Change one, and that test says where the other
//! is wrong.

use crate::api::snapshot_view::{GraphEdgeView, GraphNodeView, GraphPosition, WorkflowGraphView};
use serde::Deserialize;

/// Core's `VERTICAL_SPACING`.
const VERTICAL_SPACING: f64 = 160.0;

/// What of a step the graph reads; everything else in it is dropped here.
#[derive(Deserialize)]
struct Step {
    id: String,
    kind: String,
    title: String,
    #[serde(default)]
    task_list_from: Option<String>,
}

/// `None` when `steps` is not a list of steps that each carry an id, a kind
/// and a title: core would read no step out of it either.
pub fn graph_of(steps: &serde_json::Value) -> Option<WorkflowGraphView> {
    let steps = Vec::<Step>::deserialize(steps).ok()?;
    let nodes = steps
        .iter()
        .enumerate()
        .map(|(index, step)| GraphNodeView {
            id: step.id.clone(),
            kind: node_type(&step.kind).to_owned(),
            title: step.title.clone(),
            position: Some(GraphPosition {
                x: 0.0,
                y: index as f64 * VERTICAL_SPACING,
            }),
        })
        .collect();
    let mut edges: Vec<GraphEdgeView> = steps
        .windows(2)
        .map(|pair| GraphEdgeView {
            from: pair[0].id.clone(),
            to: pair[1].id.clone(),
        })
        .collect();
    for (index, step) in steps.iter().enumerate() {
        let Some(source) = &step.task_list_from else {
            continue;
        };
        let exists = edges
            .iter()
            .any(|edge| edge.from == *source && edge.to == step.id);
        let earlier = steps[..index].iter().any(|s| s.id == *source);
        if !exists && earlier {
            edges.push(GraphEdgeView {
                from: source.clone(),
                to: step.id.clone(),
            });
        }
    }
    Some(WorkflowGraphView { nodes, edges })
}

/// `parallel` is the superseded spelling of `sequence`.
fn node_type(kind: &str) -> &str {
    match kind {
        "parallel" => "sequence",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edges(graph: &WorkflowGraphView) -> Vec<(&str, &str)> {
        graph
            .edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.to.as_str()))
            .collect()
    }

    #[test]
    fn a_chain_is_laid_out_in_a_column_and_parallel_reads_as_sequence() {
        let graph = graph_of(&json!([
            {"id": "a", "kind": "agent", "title": "A", "prompt_template": "never read"},
            {"id": "b", "kind": "parallel", "title": "B"},
            {"id": "c", "kind": "gate", "title": "C"},
        ]))
        .unwrap();

        let nodes: Vec<_> = graph
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.kind.as_str(), node.position))
            .collect();
        assert_eq!(
            nodes,
            [
                ("a", "agent", Some(GraphPosition { x: 0.0, y: 0.0 })),
                ("b", "sequence", Some(GraphPosition { x: 0.0, y: 160.0 })),
                ("c", "gate", Some(GraphPosition { x: 0.0, y: 320.0 })),
            ]
        );
        assert_eq!(edges(&graph), [("a", "b"), ("b", "c")]);
    }

    #[test]
    fn task_list_from_adds_an_edge_from_an_earlier_step_only() {
        let graph = graph_of(&json!([
            {"id": "plan", "kind": "agent", "title": "Plan"},
            {"id": "spec", "kind": "agent", "title": "Spec"},
            {"id": "build", "kind": "sequence", "title": "Build", "task_list_from": "plan"},
            {"id": "again", "kind": "sequence", "title": "Again", "task_list_from": "build"},
            {"id": "self", "kind": "sequence", "title": "Self", "task_list_from": "self"},
            {"id": "later", "kind": "sequence", "title": "Later", "task_list_from": "end"},
            {"id": "blank", "kind": "sequence", "title": "Blank", "task_list_from": ""},
            {"id": "end", "kind": "finalize", "title": "End", "task_list_from": null},
        ]))
        .unwrap();

        assert_eq!(
            edges(&graph),
            [
                ("plan", "spec"),
                ("spec", "build"),
                ("build", "again"),
                ("again", "self"),
                ("self", "later"),
                ("later", "blank"),
                ("blank", "end"),
                ("plan", "build"),
            ]
        );
    }

    #[test]
    fn steps_core_would_not_read_give_no_graph() {
        for steps in [
            json!({"id": "a"}),
            json!([{"id": "a", "kind": "agent"}]),
            json!([{"id": 1, "kind": "agent", "title": "A"}]),
        ] {
            assert_eq!(graph_of(&steps), None, "{steps}");
        }
    }
}
