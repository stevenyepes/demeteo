use super::{starters, Starter};
use crate::api::snapshot_view::WorkflowGraphView;
use crate::auth::handlers::{ApiError, CurrentSession};
use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct WorkflowsView {
    workflows: Vec<HubWorkflowView>,
}

/// Built field by field: a starter's `steps_json` carries every prompt and
/// command, and none of it reaches a browser.
#[derive(Debug, Serialize)]
pub struct HubWorkflowView {
    workflow_id: String,
    version: u32,
    name: String,
    description: String,
    graph: WorkflowGraphView,
}

/// Ordered by name, then by `workflow_id`.
pub fn workflows_view(starters: &[Starter]) -> WorkflowsView {
    let mut workflows: Vec<HubWorkflowView> = starters
        .iter()
        .map(|starter| HubWorkflowView {
            workflow_id: starter.id.clone(),
            version: starter.version,
            name: starter.name.clone(),
            description: starter.description.clone(),
            graph: starter.graph.clone(),
        })
        .collect();
    workflows.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.workflow_id.cmp(&b.workflow_id))
    });
    WorkflowsView { workflows }
}

/// `GET /v1/workflows`. The session is asked for and not read: the starters
/// are every tenant's, and only a signed-in one is shown them.
pub async fn get_workflows(_: CurrentSession) -> Result<Json<WorkflowsView>, ApiError> {
    Ok(Json(workflows_view(starters())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::graph::graph_of;
    use serde_json::json;

    fn starter(id: &str, name: &str) -> Starter {
        let steps =
            json!([{"id": "s", "kind": "agent", "title": "S", "prompt_template": "secret plan"}]);
        Starter {
            id: id.to_owned(),
            name: name.to_owned(),
            description: format!("{name} described"),
            version: 7,
            steps_json: steps.to_string(),
            graph: graph_of(&steps).unwrap(),
        }
    }

    #[test]
    fn the_list_is_ordered_by_name_then_id_and_sends_no_step_config() {
        let view = workflows_view(&[
            starter("wf-starter-b", "Same"),
            starter("wf-starter-c", "Alpha"),
            starter("wf-starter-a", "Same"),
        ]);
        let body = serde_json::to_value(&view).unwrap();

        let ids: Vec<&str> = body["workflows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["workflow_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["wf-starter-c", "wf-starter-a", "wf-starter-b"]);
        assert_eq!(
            body["workflows"][0],
            json!({
                "workflow_id": "wf-starter-c",
                "version": 7,
                "name": "Alpha",
                "description": "Alpha described",
                "graph": {
                    "nodes": [{"id": "s", "type": "agent", "title": "S", "position": {"x": 0.0, "y": 0.0}}],
                    "edges": [],
                },
            })
        );
        assert!(!body.to_string().contains("secret plan"));
    }
}
