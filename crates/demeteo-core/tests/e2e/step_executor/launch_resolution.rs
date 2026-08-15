//! What a launch resolves to by the time it reaches the agent: the effort
//! chain, and the workflow version it is pinned against.
//!
//! These two build a real `build_core_context` with the real local
//! `ExecutionPort` and the deterministic `StubRuntime`, so they never reach
//! the doubles in [`harness`](super::harness).

use std::sync::Arc;

use super::harness::{build_test_executor, FakeNotif};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::Feature;
use crate::paths;
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::step_executor::StepExecutor;

// ─────────────────────────────────────────────────────────────────────────
// Effort resolution, end to end: a project default of `medium` and a
// per-step launch override of `max` must reach the agent as exactly that.
//
// Driven through the real engine (`build_core_context`) with the
// deterministic `StubRuntime`, whose test-only `SPAWN_LOG` records the
// `AgentContext` the driver handed it per spawn — the only place outside a
// real CLI where the resolved effort is observable.
// ─────────────────────────────────────────────────────────────────────────

/// Both steps are unique to this test, so the shared `SPAWN_LOG` can be
/// filtered by title even when other stub-driven tests run concurrently.
const EFFORT_STEP_ONE: &str = "effort-e2e: inherit the project default";
const EFFORT_STEP_TWO: &str = "effort-e2e: per-step override";

fn effort_workflow() -> serde_json::Value {
    let step = |id: &str, title: &str, artifact: &str| {
        serde_json::json!({
            "id": id,
            "kind": "agent",
            "title": title,
            "agent_kind": "stub",
            "prompt_template": format!("Write the report.\n\n@stub-write {artifact}\n"),
            "capability": "artifacts",
            "allow_shell": true,
            "artifacts": [{
                "name": id,
                "capture": { "kind": "last_write_to", "path": artifact },
                "mode": "full"
            }],
            "on_failure": null,
            "max_iterations": 1
        })
    };
    serde_json::json!({
        "name": "Effort Resolution",
        "description": "Two agent steps that differ only in their resolved effort.",
        "steps": [
            step("s-one", EFFORT_STEP_ONE, "artifacts/one.md"),
            step("s-two", EFFORT_STEP_TWO, "artifacts/two.md"),
        ]
    })
}

fn init_effort_repo(workspace_dir: &std::path::Path, project_id: &str, repo_path: &str) {
    let dir = paths::repo_target_dir_local(workspace_dir, project_id, repo_path);
    std::fs::create_dir_all(&dir).expect("create repo dir");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run git");
        assert!(out.status.success(), "git {:?} failed", args);
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# effort fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
}

#[tokio::test]
async fn effort_resolution_reaches_the_agent_per_step() {
    use crate::adapters::agent::stub_runtime::{SPAWN_LOG, STUB_AGENT_ENV};
    use crate::application::{bootstrap, projects, workflows};
    use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
    use crate::domain::ids::ProviderId;
    use crate::domain::models::{EffortLevel, ProviderInstance, StepOverride};

    std::env::set_var(STUB_AGENT_ENV, "1");
    const REPO_PATH: &str = "demeteo/effort";
    let tmp = std::env::temp_dir().join(format!("demeteo-effort-e2e-{}", paths::now_ms()));
    std::fs::create_dir_all(&tmp).expect("app data dir");

    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(FakeNotif),
        tokio::runtime::Handle::current(),
    );

    ctx.app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from("effort-provider"),
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
            name: "effort".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: "effort-provider".to_string(),
            }],
        },
    )
    .expect("create project");

    init_effort_repo(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    // Tier 4: the project-wide default. `bootstrap_project` detects a
    // strategy but persists no settings row, so seed one from the same
    // defaults the executor would otherwise fall back to.
    let mut settings = ctx
        .projects
        .get_settings(&project.id)
        .expect("settings read")
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
    settings.project_id = project.id.clone();
    settings.default_effort = Some(EffortLevel::Medium);
    ctx.projects.save_settings(settings).expect("save settings");

    let workflow_id =
        workflows::create_from_json(&ctx.workflows, &effort_workflow()).expect("ingest workflow");

    // Tier 1: a per-step launch override on the second step only.
    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Effort Feature",
            "Exercise the effort resolution chain.",
            Some("stub"),
            None,
            None,
            None,
            None,
            None,
            vec![StepOverride {
                step_id: "s-two".to_string(),
                agent_kind: None,
                model: None,
                effort: Some(EffortLevel::Max),
            }],
            vec![],
        )
        .await
        .expect("feature_start");

    // Drive to terminal.
    let started = std::time::Instant::now();
    loop {
        let f = ctx
            .features
            .get(&feature.id)
            .expect("feature read")
            .expect("feature exists");
        if matches!(
            f.status.as_str(),
            "completed" | "awaiting_mr" | "failed" | "interrupted"
        ) {
            let steps = ctx
                .features
                .steps_for_feature(&feature.id)
                .unwrap_or_default();
            assert!(
                steps.iter().all(|s| s.status == "completed"),
                "both steps must complete; got {:?}",
                steps
                    .iter()
                    .map(|s| (
                        s.step_id.0.clone(),
                        s.status.clone(),
                        s.error_message.clone()
                    ))
                    .collect::<Vec<_>>()
            );
            break;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "feature did not settle; status {}",
            f.status
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let spawned: Vec<(String, Option<EffortLevel>)> = SPAWN_LOG
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(title, effort)| {
            let t = title.clone()?;
            (t == EFFORT_STEP_ONE || t == EFFORT_STEP_TWO).then_some((t, *effort))
        })
        .collect();

    assert_eq!(
        spawned,
        vec![
            // No workflow/feature/step override → the project default.
            (EFFORT_STEP_ONE.to_string(), Some(EffortLevel::Medium)),
            // The per-step launch override beats it — and reaches the agent
            // rather than being swallowed by a reused session (the effort is
            // part of the session key).
            (EFFORT_STEP_TWO.to_string(), Some(EffortLevel::Max)),
        ],
        "each step's AgentContext must carry its own resolved effort",
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Decision 38 (V33, task P1.15): `feature_start` resolves the workflow's
/// latest version once and pins it on the row; the run path reads the pin
/// — so saving a newer version after launch demonstrably does not change
/// the graph a running feature resolves.
#[tokio::test]
async fn test_feature_start_pins_workflow_version() {
    let (executor, db, temp_dir) = build_test_executor("wf_pin").await;

    let workflows: Arc<dyn crate::ports::db::WorkflowRepository> = db.clone();
    let wf_id = crate::application::workflows::create_from_json(
        &workflows,
        &serde_json::json!({
            "name": "Pin Test",
            "steps": [ { "id": "s1", "kind": "sync", "title": "S1" } ]
        }),
    )
    .expect("create workflow");
    let v1 = workflows
        .latest_version(&wf_id)
        .unwrap()
        .expect("initial version");

    // Seed the project (FK target); it has no repository, so the
    // bootstrap tail fails later — the eager pin happens before the tail
    // spawns and must already be set.
    let projects: &dyn ProjectRepository = &*db;
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-pin"),
            name: "pin-test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: paths::now_ms(),
        })
        .unwrap();

    let feature = executor
        .feature_start(
            None,
            "p-pin",
            wf_id.as_str(),
            "Pin Feature",
            "a description",
            None,
            None,
            None,
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start returns the eager row");
    assert_eq!(
        feature.workflow_version_id.as_ref(),
        Some(&v1.id),
        "feature_start pins the latest version at launch"
    );

    // "Edit" the workflow: save a second version with a different graph.
    workflows
        .save_version(crate::domain::models::WorkflowVersion {
            id: crate::domain::ids::WorkflowVersionId::from(format!("{}-v2", wf_id.as_str())),
            workflow_id: wf_id.clone(),
            version: 2,
            steps_json: serde_json::json!([
                { "id": "s1", "kind": "sync", "title": "S1" },
                { "id": "s2", "kind": "sync", "title": "S2" }
            ])
            .to_string(),
            definition_json: None,
            note: None,
            created_at: paths::now_ms(),
        })
        .expect("save v2");

    // The row keeps its pin, and the run path resolves the *pinned*
    // version, not the new latest.
    let features: &dyn FeatureRepository = &*db;
    let row = features.get(&feature.id).unwrap().expect("feature row");
    assert_eq!(row.workflow_version_id.as_ref(), Some(&v1.id));
    let resolved = executor
        .resolve_pinned_version(feature.id.as_str(), &wf_id)
        .expect("resolve pinned");
    assert_eq!(
        (resolved.id, resolved.version),
        (v1.id.clone(), 1),
        "a mid-run workflow edit must not change the resolved graph"
    );

    // Backfill: a pre-V33 row (no pin) resolves latest once and pins it.
    features
        .add(Feature {
            effort: None,
            id: FeatureId::from("f-prepin"),
            project_id: ProjectId::from("p-pin"),
            workflow_id: Some(wf_id.clone()),
            workflow_version_id: None,
            title: "legacy".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            created_at: paths::now_ms(),
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .unwrap();
    let resolved = executor
        .resolve_pinned_version("f-prepin", &wf_id)
        .expect("backfill resolve");
    assert_eq!(resolved.version, 2, "an unpinned row resolves latest once");
    let row = features
        .get(&FeatureId::from("f-prepin"))
        .unwrap()
        .expect("legacy row");
    assert_eq!(
        row.workflow_version_id.map(|v| v.0),
        Some(format!("{}-v2", wf_id.as_str())),
        "the backfill pins what it resolved"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
