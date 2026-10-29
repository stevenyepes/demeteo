//! What a view may call a run besides its ids: the Feature's title, its
//! Project's name and its Steps' titles, all read from the instance's last
//! snapshot (decision 70, `docs/HUB.md` §5).
//!
//! Every label is looked up, never inferred. A Feature the snapshot does not
//! name has no title; a Step is titled only from the very Workflow version
//! the Feature pins, because a node id in another version may name another
//! step. A missing label is `None`, which a view answers as `null`.
//!
//! Synchronous and free of the store, as [`run_state`](super::run_state) is.

use super::snapshot_view::{definition_of, GraphEdgeView, GraphNodeView};
use demeteo_hub_protocol::{FeatureSnapshot, Snapshot};
use serde::Serialize;
use std::collections::HashMap;

/// The schema-v2 document a run's canvas draws, cut to its graph by the
/// [`snapshot_view`](super::snapshot_view) rule: nothing a step runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunWorkflowView {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub nodes: Vec<GraphNodeView>,
    pub edges: Vec<GraphEdgeView>,
}

/// The labels one instance's snapshot gives, indexed for lookup by id.
#[derive(Debug, Default)]
pub struct RunLabels {
    features: HashMap<String, FeatureSnapshot>,
    projects: HashMap<String, String>,
    /// Keyed by `(workflow_id, version)`.
    graphs: HashMap<(String, u32), RunWorkflowView>,
}

impl RunLabels {
    /// [`Default`] — no label at all — for an instance with no readable
    /// snapshot.
    pub fn of(snapshot: Option<&Snapshot>) -> Self {
        let Some(snapshot) = snapshot else {
            return Self::default();
        };
        Self {
            features: snapshot
                .features
                .iter()
                .map(|feature| (feature.id.clone(), feature.clone()))
                .collect(),
            projects: snapshot
                .projects
                .iter()
                .map(|project| (project.id.clone(), project.name.clone()))
                .collect(),
            graphs: snapshot
                .workflows
                .iter()
                .filter_map(|version| {
                    let (name, graph) = definition_of(version)?;
                    let key = (version.workflow_id.clone(), version.version);
                    let view = RunWorkflowView {
                        schema_version: 2,
                        id: version.workflow_id.clone(),
                        name,
                        nodes: graph.nodes,
                        edges: graph.edges,
                    };
                    Some((key, view))
                })
                .collect(),
        }
    }

    pub fn title(&self, feature_id: &str) -> Option<String> {
        Some(self.features.get(feature_id)?.title.clone())
    }

    pub fn project_id(&self, feature_id: &str) -> Option<String> {
        Some(self.features.get(feature_id)?.project_id.clone())
    }

    /// `None` as well when the Feature names a Project the snapshot does not.
    pub fn project_name(&self, feature_id: &str) -> Option<String> {
        let project_id = &self.features.get(feature_id)?.project_id;
        self.projects.get(project_id).cloned()
    }

    /// The graph of the version the Feature pins, when the snapshot sent it.
    pub fn workflow(&self, feature_id: &str) -> Option<&RunWorkflowView> {
        let feature = self.features.get(feature_id)?;
        let key = (feature.workflow_id.clone()?, feature.workflow_version?);
        self.graphs.get(&key)
    }

    pub fn step_title(&self, feature_id: &str, step_id: Option<&str>) -> Option<String> {
        let step_id = step_id?;
        self.workflow(feature_id)?
            .nodes
            .iter()
            .find(|node| node.id == step_id)
            .map(|node| node.title.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use demeteo_hub_protocol::{ProjectSnapshot, WorkflowVersionSnapshot};
    use serde_json::json;

    fn version(version: u32, title: &str) -> WorkflowVersionSnapshot {
        WorkflowVersionSnapshot {
            id: format!("wf@{version}"),
            workflow_id: "wf".to_owned(),
            version,
            steps_json: "[]".to_owned(),
            definition_json: Some(
                json!({
                    "name": "Ship",
                    "nodes": [{"id": "s-gate", "type": "gate", "title": title,
                               "config": {"prompt_template": "SECRET"}}],
                    "edges": [],
                })
                .to_string(),
            ),
            note: None,
            created_at: 0,
        }
    }

    fn feature(id: &str, project_id: &str, version: Option<u32>) -> FeatureSnapshot {
        FeatureSnapshot {
            id: id.to_owned(),
            project_id: project_id.to_owned(),
            title: format!("{id} title"),
            workflow_id: version.map(|_| "wf".to_owned()),
            workflow_version: version,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            v: 1,
            projects: vec![ProjectSnapshot {
                id: "p-1".to_owned(),
                name: "checkout-api".to_owned(),
                remote_url: None,
            }],
            workflows: vec![version(1, "Old review"), version(2, "Review")],
            run_shape: Vec::new(),
            features: vec![
                feature("f-pinned-1", "p-1", Some(1)),
                feature("f-pinned-3", "p-1", Some(3)),
                feature("f-orphan", "p-gone", None),
            ],
            harnesses: None,
        }
    }

    #[test]
    fn a_feature_the_snapshot_names_has_its_title_and_project_name() {
        let labels = RunLabels::of(Some(&snapshot()));
        assert_eq!(
            labels.title("f-pinned-1").as_deref(),
            Some("f-pinned-1 title")
        );
        assert_eq!(labels.project_id("f-pinned-1").as_deref(), Some("p-1"));
        assert_eq!(
            labels.project_name("f-pinned-1").as_deref(),
            Some("checkout-api")
        );
    }

    #[test]
    fn a_feature_the_snapshot_does_not_name_has_no_label() {
        let labels = RunLabels::of(Some(&snapshot()));
        assert_eq!(labels.title("f-unknown"), None);
        assert_eq!(labels.project_name("f-unknown"), None);
        assert_eq!(labels.step_title("f-unknown", Some("s-gate")), None);
        assert!(labels.workflow("f-unknown").is_none());
    }

    #[test]
    fn a_project_the_snapshot_does_not_name_has_no_name() {
        let labels = RunLabels::of(Some(&snapshot()));
        assert_eq!(labels.project_id("f-orphan").as_deref(), Some("p-gone"));
        assert_eq!(labels.project_name("f-orphan"), None);
    }

    #[test]
    fn a_step_is_titled_from_the_pinned_version_and_no_other() {
        let labels = RunLabels::of(Some(&snapshot()));
        assert_eq!(
            labels.step_title("f-pinned-1", Some("s-gate")).as_deref(),
            Some("Old review")
        );
        assert_eq!(labels.step_title("f-pinned-3", Some("s-gate")), None);
        assert_eq!(labels.step_title("f-orphan", Some("s-gate")), None);
        assert_eq!(labels.step_title("f-pinned-1", Some("s-other")), None);
        assert_eq!(labels.step_title("f-pinned-1", None), None);
    }

    #[test]
    fn the_pinned_workflow_is_a_drawable_graph_holding_nothing_a_step_runs() {
        let labels = RunLabels::of(Some(&snapshot()));
        let workflow = labels.workflow("f-pinned-1").expect("a graph");
        let text = serde_json::to_string(workflow).unwrap();
        assert!(!text.contains("SECRET"), "{text}");
        assert_eq!(
            serde_json::to_value(workflow).unwrap(),
            json!({
                "schema_version": 2,
                "id": "wf",
                "name": "Ship",
                "nodes": [{"id": "s-gate", "type": "gate", "title": "Old review", "position": null}],
                "edges": [],
            })
        );
    }

    #[test]
    fn no_snapshot_gives_no_label() {
        let labels = RunLabels::of(None);
        assert_eq!(labels.title("f-pinned-1"), None);
        assert!(labels.workflow("f-pinned-1").is_none());
    }
}
