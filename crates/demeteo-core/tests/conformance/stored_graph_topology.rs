//! Stored-v2-topology gate (task P3.6, PRD §5.1/§5.3).
//!
//! V34 lets a version carry its schema-v2 document beside the v1 step list.
//! That only means something if the **engine schedules the stored edges**: a
//! graph the builder drew as a diamond must not quietly run as the line its
//! v1 projection flattens to, or persistence would be a lie the canvas tells.
//!
//! Two runs over the same four nodes:
//!
//! 1. **The stored graph is honored.** `plan → {left, right} → ship` runs all
//!    four nodes in an order that respects the drawn edges — proven from the
//!    durable `run_events` log (P1.13), which is the engine's own account of
//!    what it did, not a re-derivation.
//! 2. **A pre-P3.6 row still behaves.** The same workflow with
//!    `definition_json` left NULL migrates its step list, exactly as every
//!    version written before this column existed.
//!
//! `max_parallel_nodes` is still 1 (PRD §5.6), so "honors the edges" means a
//! valid topological order, not concurrency — that is P4.1's job.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::adapters::run_event_log::RunEventRecorder;
use crate::application::{bootstrap, projects};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProviderId, WorkflowId, WorkflowVersionId};
use crate::domain::models::workflow_migrate::project_v2_to_v1;
use crate::domain::models::workflow_v2::WorkflowDefinitionV2;
use crate::domain::models::{ProviderInstance, Workflow, WorkflowVersion};
use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;

const REPO_PATH: &str = "demeteo/stored-graph";
const PROVIDER_ID: &str = "stored-graph-provider";

struct NoopNotif;
impl NotificationPort for NoopNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// A diamond: `plan` fans out to two independent artifact-producing nodes,
/// which fan back in to `ship`. Every node is a stub agent step, so the run is
/// deterministic and free.
fn diamond(id: &str) -> WorkflowDefinitionV2 {
    let agent = |node_id: &str, title: &str, x: f64, y: f64| {
        serde_json::json!({
            "id": node_id,
            "type": "agent",
            "title": title,
            "config": {
                "agent_kind": "stub",
                "capability": "artifacts",
                "prompt_template":
                    format!("Do {node_id}.\n@stub-write artifacts/{node_id}.md\n"),
                "artifacts": [
                    {
                        "name": node_id,
                        "capture": { "kind": "last_write_to", "path": format!("artifacts/{node_id}.md") },
                        "mode": "full"
                    }
                ]
            },
            "position": { "x": x, "y": y }
        })
    };
    serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": id,
        "name": "Diamond",
        "nodes": [
            agent("plan", "Plan", 0.0, 0.0),
            agent("left", "Left branch", -160.0, 160.0),
            agent("right", "Right branch", 160.0, 160.0),
            agent("ship", "Ship", 0.0, 320.0),
        ],
        "edges": [
            { "from": "plan", "to": "left" },
            { "from": "plan", "to": "right" },
            { "from": "left", "to": "ship" },
            { "from": "right", "to": "ship" }
        ]
    }))
    .expect("diamond is a valid v2 definition")
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_local_repo(workspace_dir: &Path, project_id: &str) {
    let dir = paths::repo_target_dir_local(workspace_dir, project_id, REPO_PATH);
    std::fs::create_dir_all(&dir).expect("create repo dir");
    git(&dir, &["init", "-b", "main"]);
    git(&dir, &["config", "user.email", "demeteo@local"]);
    git(&dir, &["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# stored graph fixture\n").expect("seed README");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-m", "seed"]);
}

async fn poll_terminal(ctx: &AppContext, feature_id: &FeatureId) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = ctx
            .features
            .get(feature_id)
            .ok()
            .flatten()
            .map(|f| f.status)
            .unwrap_or_default();
        if matches!(
            status.as_str(),
            "completed" | "failed" | "cancelled" | "awaiting_mr"
        ) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "feature did not settle (last: {status})"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The order the engine actually dispatched, read out of the durable event
/// log: each node's first `running` transition, in append order.
fn dispatch_order(ctx: &AppContext, feature_id: &FeatureId) -> Vec<String> {
    let events = ctx
        .run_events
        .list_since(feature_id.as_str(), 0)
        .expect("read run_events");
    let mut order: Vec<String> = Vec::new();
    for event in events {
        if event.kind != "step_progress" {
            continue;
        }
        let Some(payload) = event
            .payload_json
            .as_deref()
            .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
        else {
            continue;
        };
        if payload.get("status").and_then(|s| s.as_str()) != Some("running") {
            continue;
        }
        let Some(step_id) = payload.get("step_id").and_then(|s| s.as_str()) else {
            continue;
        };
        if !order.iter().any(|s| s == step_id) {
            order.push(step_id.to_string());
        }
    }
    order
}

/// Seed a project + a workflow whose single version carries `definition_json`
/// only when `store_v2` is set, then start a feature on it.
async fn run_diamond(tag: &str, store_v2: bool) -> (AppContext, PathBuf, FeatureId) {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-stored-graph-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).expect("create app data dir");
    // Wire exactly like `src-tauri/lib.rs`: the recorder decorates the UI
    // notifier and is late-bound to `ctx.run_events`, which is what makes the
    // durable log the account of the run this test reads back (P1.13).
    let recorder = Arc::new(RunEventRecorder::new(Arc::new(NoopNotif)));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        recorder.clone(),
        tokio::runtime::Handle::current(),
    );
    recorder.wire(ctx.run_events.clone());

    ctx.app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from(PROVIDER_ID),
            kind: "github".to_string(),
            host: "github.com".to_string(),
            username: String::new(),
            avatar_url: String::new(),
            created_at: paths::now_ms(),
        })
        .expect("register provider");
    let project = projects::create(
        &ctx,
        projects::ProjectConfig {
            name: format!("stored-graph-{tag}"),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");
    init_local_repo(&ctx.workspace_dir, project.id.as_str());
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    // Store the workflow the way `workflow_save` does: the v2 document plus
    // its v1 projection (or projection only, for the pre-P3.6 control).
    let wf_id = WorkflowId::from(format!("wf-diamond-{tag}"));
    let def = diamond(wf_id.as_str());
    let steps = project_v2_to_v1(&def);
    let now = paths::now_ms();
    ctx.workflows
        .create(Workflow {
            id: wf_id.clone(),
            name: "Diamond".to_string(),
            description: "fan-out / fan-in".to_string(),
            is_starter: false,
            created_at: now,
            updated_at: now,
            schedule: None,
        })
        .expect("create workflow");
    ctx.workflows
        .save_version(WorkflowVersion {
            id: WorkflowVersionId::from(format!("{}-v1", wf_id.as_str())),
            workflow_id: wf_id.clone(),
            version: 1,
            steps_json: serde_json::to_string(&steps).expect("serialize projection"),
            definition_json: store_v2.then(|| serde_json::to_string(&def).expect("serialize v2")),
            note: None,
            created_at: now,
        })
        .expect("save version");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            wf_id.as_str(),
            "Diamond run",
            "Run a stored fan-out / fan-in graph.",
            Some("stub"),
            None,
            None,
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start");

    let status = poll_terminal(&ctx, &feature.id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "the diamond must run to success; got {status}"
    );
    (ctx, tmp, feature.id)
}

/// The gate: the drawn edges are the edges that ran.
#[tokio::test]
async fn a_stored_v2_graph_schedules_its_own_edges() {
    let (ctx, tmp, feature_id) = run_diamond("stored", true).await;

    let steps = ctx
        .features
        .steps_for_feature(&feature_id)
        .expect("steps for feature");
    assert_eq!(steps.len(), 4, "every node got an execution row");
    for step in &steps {
        assert_eq!(
            step.status, "completed",
            "node '{}' did not complete",
            step.step_id.0
        );
    }

    let order = dispatch_order(&ctx, &feature_id);
    assert_eq!(order.len(), 4, "each node dispatched once: {order:?}");
    let at = |id: &str| {
        order
            .iter()
            .position(|s| s == id)
            .unwrap_or_else(|| panic!("'{id}' never ran; order was {order:?}"))
    };
    assert!(at("plan") < at("left"), "left waited for plan: {order:?}");
    assert!(at("plan") < at("right"), "right waited for plan: {order:?}");
    assert!(at("left") < at("ship"), "ship waited for left: {order:?}");
    assert!(at("right") < at("ship"), "ship waited for right: {order:?}");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Control: a version written before V34 still runs, on the migration of its
/// step list — the fallback every pre-P3.6 row depends on.
#[tokio::test]
async fn a_version_without_a_stored_document_still_runs_its_step_list() {
    let (ctx, tmp, feature_id) = run_diamond("legacy", false).await;

    let steps = ctx
        .features
        .steps_for_feature(&feature_id)
        .expect("steps for feature");
    assert_eq!(steps.len(), 4);
    for step in &steps {
        assert_eq!(step.status, "completed", "{}", step.step_id.0);
    }

    // Without the document the engine sees the projected chain, so the order
    // is the projection's — still a valid order over the same four nodes.
    let order = dispatch_order(&ctx, &feature_id);
    assert_eq!(order.len(), 4, "{order:?}");
    assert_eq!(order[0], "plan", "the chain still starts at the root");

    let _ = std::fs::remove_dir_all(&tmp);
}
