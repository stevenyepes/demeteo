#![cfg(target_os = "linux")]
//! The Hub's embedded starters, held to core. The Hub does not link core, so
//! two things it does on its own are only right while these pass: what it
//! signs for a runner must be a start the runner accepts, and the graph it
//! draws must be the one core migrates from the same steps
//! (`crates/demeteo-hub/src/workflows/graph.rs`). The header of
//! `src-tauri/tests/hub_pairing.rs` says why this evidence lives here.

use demeteo_core::domain::hub_request::{runner_run_spec, runner_start_refusal};
use demeteo_core::domain::ids::WorkflowId;
use demeteo_core::domain::models::workflow_migrate::migrate_v1_to_v2;
use demeteo_core::domain::models::StepConfig;
use demeteo_hub::api::snapshot_view::{GraphEdgeView, GraphNodeView, GraphPosition};
use demeteo_hub::protocol::{RunnerRepository, RunnerStart, RunnerWorkflow, StartFeature};
use demeteo_hub::workflows::{starters, Starter};

fn start_from(starter: &Starter) -> StartFeature {
    StartFeature {
        project_id: "00000000-0000-4000-8000-000000000001".into(),
        workflow_id: starter.id.clone(),
        title: "From the Hub".into(),
        description: "A starter, as the Hub signs it".into(),
        agent_kind: None,
        model: None,
        effort: None,
        step_overrides: Vec::new(),
        max_budget_cents: Some(250),
        max_wall_clock_secs: None,
        attachments: Vec::new(),
        runner: Some(Box::new(RunnerStart {
            source_instance_id: "hub".into(),
            repository: RunnerRepository {
                provider_kind: "github".into(),
                host: "github.com".into(),
                full_name: "acme/widgets".into(),
            },
            workflow: RunnerWorkflow {
                workflow_id: starter.id.clone(),
                version: starter.version,
                name: starter.name.clone(),
                steps_json: starter.steps_json.clone(),
            },
        })),
    }
}

#[test]
fn the_hub_embeds_starters() {
    assert!(starters().len() >= 2, "{}", starters().len());
}

#[test]
fn a_runner_accepts_every_starter_the_hub_signs() {
    for starter in starters() {
        let start = start_from(starter);
        assert_eq!(runner_start_refusal(&start), None, "{}", starter.id);
        let runner = start.runner.as_deref().expect("a runner start");
        let spec = runner_run_spec(&start, runner, "f-hub", Vec::new());
        let spec = spec.unwrap_or_else(|refusal| panic!("{}: {refusal:?}", starter.id));
        assert_eq!(spec.workflow_json["name"], starter.name.as_str());
    }
}

#[test]
fn the_hubs_graph_is_cores_migration_of_the_same_steps() {
    for starter in starters() {
        let steps: Vec<StepConfig> =
            serde_json::from_str(&starter.steps_json).expect("core reads the steps");
        let migrated =
            migrate_v1_to_v2(WorkflowId::from(starter.id.clone()), &starter.name, &steps);

        let nodes: Vec<GraphNodeView> = migrated
            .nodes
            .iter()
            .map(|node| GraphNodeView {
                id: node.id.0.clone(),
                kind: node.node_type.clone(),
                title: node.title.clone(),
                position: node
                    .position
                    .as_ref()
                    .map(|p| GraphPosition { x: p.x, y: p.y }),
            })
            .collect();
        let edges: Vec<GraphEdgeView> = migrated
            .edges
            .iter()
            .map(|edge| GraphEdgeView {
                from: edge.from.0.clone(),
                to: edge.to.0.clone(),
            })
            .collect();

        assert_eq!(starter.graph.nodes, nodes, "{}", starter.id);
        assert_eq!(starter.graph.edges, edges, "{}", starter.id);
    }
}
