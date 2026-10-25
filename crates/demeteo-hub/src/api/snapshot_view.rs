//! What `GET /v1/instances/{instance_id}` shows of a snapshot's workflows and
//! run shape: enough to choose a workflow, draw its graph and pre-fill a
//! dispatch, and nothing a step runs.
//!
//! The snapshot carries each step's prompt templates and commands
//! (`docs/HUB.md` §11 item 11 asks whether they may reach the Hub at all).
//! They reach no browser: a definition is read through [`Definition`], whose
//! types name only the graph, so anything else in the document is dropped by
//! construction. Do not add `config`, `retry`, `join` or `when` to them, and
//! do not give them `deny_unknown_fields` — a document carrying those must
//! still project.

use demeteo_hub_protocol::{Effort, RunShapeSettings, WorkflowVersionSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A workflow as a dispatch may name it: its latest version only, since a
/// dispatch names a `workflow_id` and no version.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowView {
    pub workflow_id: String,
    pub version: u32,
    /// The definition's `name`, or the `workflow_id` when it has none.
    pub name: String,
    /// `None` when the version carries no readable schema-v2 document, as
    /// one stored before V34 does not. The Hub does not depend on
    /// `demeteo-core`, so it cannot derive one from `steps_json`.
    pub graph: Option<WorkflowGraphView>,
}

/// The shape of a workflow and nothing a step runs: see the [module
/// doc](self) before adding a field here or to [`GraphNodeView`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowGraphView {
    pub nodes: Vec<GraphNodeView>,
    pub edges: Vec<GraphEdgeView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphNodeView {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub position: Option<GraphPosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphEdgeView {
    pub from: String,
    pub to: String,
}

/// One project's dispatch defaults. `default_loop_iterations` is left out:
/// a dispatch has no field it could pre-fill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunShapeView {
    pub project_id: String,
    pub default_agent_kind: Option<String>,
    pub default_model: Option<String>,
    pub default_effort: Option<Effort>,
    pub default_workflow_id: Option<String>,
    pub default_max_budget_cents: Option<u64>,
}

#[derive(Deserialize)]
struct Definition {
    name: Option<String>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    position: Option<GraphPosition>,
}

#[derive(Deserialize)]
struct Edge {
    from: String,
    to: String,
}

/// Ordered by name, then by `workflow_id`.
pub(super) fn workflows(versions: Vec<WorkflowVersionSnapshot>) -> Vec<WorkflowView> {
    let mut latest: HashMap<String, WorkflowVersionSnapshot> = HashMap::new();
    for version in versions {
        match latest.get(&version.workflow_id) {
            Some(held) if held.version >= version.version => {}
            _ => {
                latest.insert(version.workflow_id.clone(), version);
            }
        }
    }
    let mut views: Vec<WorkflowView> = latest.into_values().map(workflow).collect();
    views.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.workflow_id.cmp(&b.workflow_id))
    });
    views
}

pub(super) fn run_shape(settings: Vec<RunShapeSettings>) -> Vec<RunShapeView> {
    settings
        .into_iter()
        .map(|shape| RunShapeView {
            project_id: shape.project_id,
            default_agent_kind: shape.default_agent_kind,
            default_model: shape.default_model,
            default_effort: shape.default_effort,
            default_workflow_id: shape.default_workflow_id,
            default_max_budget_cents: shape.default_max_budget_cents,
        })
        .collect()
}

fn workflow(version: WorkflowVersionSnapshot) -> WorkflowView {
    let definition = version
        .definition_json
        .as_deref()
        .and_then(|text| serde_json::from_str::<Definition>(text).ok());
    let name = definition
        .as_ref()
        .and_then(|definition| definition.name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| version.workflow_id.clone());
    WorkflowView {
        workflow_id: version.workflow_id,
        version: version.version,
        name,
        graph: definition.map(graph),
    }
}

fn graph(definition: Definition) -> WorkflowGraphView {
    WorkflowGraphView {
        nodes: definition
            .nodes
            .into_iter()
            .map(|node| GraphNodeView {
                id: node.id,
                kind: node.kind,
                title: node.title,
                position: node.position,
            })
            .collect(),
        edges: definition
            .edges
            .into_iter()
            .map(|edge| GraphEdgeView {
                from: edge.from,
                to: edge.to,
            })
            .collect(),
    }
}
