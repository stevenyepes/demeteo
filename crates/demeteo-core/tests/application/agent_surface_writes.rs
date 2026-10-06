// Tests extracted from `crates/demeteo-core/src/application/agent_surface/writes.rs`
// (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::adapters::step_executor::setup::fetch_default_settings;
use crate::application::discovery::{create as open_discovery, NewDiscovery};
use crate::application::launch::tests::{harness, RunnerAt};
use crate::application::projects::RepositoryConfig;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::WorkflowId;
use crate::domain::models::{EffortLevel, Project, ProviderInstance, Ticket, TicketState};

/// A fully wired `AppContext` over a fresh temp-dir SQLite database — same
/// shape as `tests/application/agent_surface.rs`'s `fixture`.
fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-agent-surface-writes-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    )
}

fn project(id: &str) -> Project {
    Project {
        id: ProjectId::from(id.to_string()),
        name: format!("project {id}"),
        compute_type: "local".to_string(),
        remote_host: None,
        status: "idle".to_string(),
        nodes: 0,
        spend: 0.0,
        tokens: 0,
        created_at: 0,
    }
}

fn patch(default_effort: EffortLevel, commit_artifacts: bool) -> RunShapePatch {
    RunShapePatch {
        default_agent_kind: None,
        default_model: None,
        default_effort: Some(default_effort),
        default_workflow_id: None,
        artifact_subdir: "artifacts/".to_string(),
        commit_artifacts,
        sync_resolver_agent_kind: None,
        sync_resolver_model: None,
        sync_resolver_effort: None,
    }
}

#[tokio::test]
async fn missing_project_is_an_error_not_a_panic() {
    let ctx = fixture("missing");
    let project_id = ProjectId::from("p-none".to_string());

    let result = apply_run_shape_patch(&ctx, &project_id, patch(EffortLevel::Low, true));

    assert_eq!(result.unwrap_err(), "project not found");
}

#[tokio::test]
async fn patch_round_trips_through_get_and_save_settings() {
    let ctx = fixture("round-trip");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let mut initial = fetch_default_settings();
    initial.project_id = project_id.clone();
    initial.default_effort = Some(EffortLevel::High);
    initial.commit_artifacts = false;
    initial.worktree_strategy.default_branch = "trunk".to_string();
    initial.feature_lifecycle = "delete".to_string();
    ctx.projects.save_settings(initial.clone()).unwrap();

    let returned = apply_run_shape_patch(&ctx, &project_id, patch(EffortLevel::Low, true)).unwrap();

    assert_eq!(returned.default_effort, Some(EffortLevel::Low));
    assert!(returned.commit_artifacts);
    // Untouched fields — outside the nine `RunShapePatch` fields — survive.
    assert_eq!(returned.worktree_strategy.default_branch, "trunk");
    assert_eq!(returned.feature_lifecycle, "delete");

    let reloaded = ctx
        .projects
        .get_settings(&project_id)
        .unwrap()
        .expect("settings were persisted");
    assert_eq!(reloaded.default_effort, Some(EffortLevel::Low));
    assert!(reloaded.commit_artifacts);
    assert_eq!(reloaded.worktree_strategy.default_branch, "trunk");
    assert_eq!(reloaded.feature_lifecycle, "delete");
}

fn provider(id: &str) -> ProviderInstance {
    ProviderInstance {
        id: crate::domain::ids::ProviderId::from(id.to_string()),
        kind: "github".to_string(),
        host: "github.com".to_string(),
        username: "u".to_string(),
        avatar_url: String::new(),
        created_at: 0,
    }
}

fn config(compute_type: &str, remote_host: Option<&str>, provider_id: &str) -> ProjectConfig {
    ProjectConfig {
        name: "demo".to_string(),
        compute_type: compute_type.to_string(),
        remote_host: remote_host.map(str::to_string),
        repos: vec![RepositoryConfig {
            repo_path: "https://example.invalid/demo.git".to_string(),
            provider_id: provider_id.to_string(),
        }],
    }
}

/// The sequence a client follows: register, then patch. The project exists, so
/// the refusal must not call it missing, and must say what is actually absent.
#[tokio::test]
async fn a_registered_project_without_settings_is_not_reported_as_missing() {
    let ctx = fixture("create-then-patch");
    ctx.app_settings
        .add_provider_instance(provider("gh-1"))
        .unwrap();

    let created = create_workspace_project(&ctx, config("local", None, "gh-1")).unwrap();

    let err = apply_run_shape_patch(&ctx, &created.id, patch(EffortLevel::Low, true)).unwrap_err();
    assert_ne!(err, "project not found");
    assert!(err.contains("not been bootstrapped"), "got: {err}");
}

#[tokio::test]
async fn create_refuses_bad_input_before_inserting_anything() {
    let ctx = fixture("create-invalid");
    ctx.app_settings
        .add_provider_instance(provider("gh-1"))
        .unwrap();

    for bad in [
        config("cloud", None, "gh-1"),
        config("remote", None, "gh-1"),
        config("remote", Some("no-such-machine"), "gh-1"),
        config("local", Some("m-1"), "gh-1"),
        config("local", None, "no-such-provider"),
        ProjectConfig {
            name: "  ".to_string(),
            ..config("local", None, "gh-1")
        },
        ProjectConfig {
            repos: vec![RepositoryConfig {
                repo_path: " ".to_string(),
                provider_id: "gh-1".to_string(),
            }],
            ..config("local", None, "gh-1")
        },
    ] {
        assert!(
            create_workspace_project(&ctx, bad.clone()).is_err(),
            "{bad:?}"
        );
    }
    assert!(ctx.projects.get_projects().unwrap().is_empty());
}

/// A refused patch must not have touched what it was aimed at.
#[tokio::test]
async fn a_hostile_artifact_subdir_is_refused_and_nothing_is_saved() {
    let ctx = fixture("hostile-subdir");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let mut initial = fetch_default_settings();
    initial.project_id = project_id.clone();
    ctx.projects.save_settings(initial.clone()).unwrap();

    let mut hostile = patch(EffortLevel::Low, false);
    hostile.artifact_subdir = "x'; touch /tmp/pwned; '".to_string();

    assert!(apply_run_shape_patch(&ctx, &project_id, hostile).is_err());
    let reloaded = ctx.projects.get_settings(&project_id).unwrap().unwrap();
    assert_eq!(reloaded.artifact_subdir, initial.artifact_subdir);
}

fn agent_launch(machine_id: Option<&str>) -> AgentFeatureLaunch {
    AgentFeatureLaunch {
        project_id: "p-1".to_string(),
        workflow_id: "w-1".to_string(),
        title: "Ship it".to_string(),
        description: "Started over MCP".to_string(),
        machine_id: machine_id.map(str::to_string),
        target_repo_id: None,
        unattended: None,
        max_cost_usd: None,
        max_wall_clock_secs: None,
    }
}

fn feature_ids(ctx: &AppContext) -> Vec<String> {
    ctx.features
        .get_all_for_project(&ProjectId::from("p-1"))
        .unwrap()
        .into_iter()
        .map(|feature| feature.id.0)
        .collect()
}

#[tokio::test]
async fn start_feature_without_a_machine_hands_the_executor_the_launch_it_always_did() {
    let h = harness(RunnerAt::accepting());

    let feature = start_feature(&h.ctx, agent_launch(None))
        .await
        .expect("a local start reaches the executor")
        .feature;

    let received = h.spy.launched().expect("feature_start was called");
    let before = FeatureLaunch {
        project_id: "p-1".to_string(),
        workflow_id: "w-1".to_string(),
        title: "Ship it".to_string(),
        description: "Started over MCP".to_string(),
        ..FeatureLaunch::default()
    };
    assert_eq!(format!("{received:?}"), format!("{before:?}"));
    assert_eq!(feature.id.0, "f-1");
    assert_eq!(
        h.exec.calls(),
        Vec::<String>::new(),
        "no RPC on a local start"
    );
}

#[tokio::test]
async fn a_blank_or_local_machine_id_starts_locally() {
    for local in ["", "  ", "local"] {
        let h = harness(RunnerAt::accepting());

        start_feature(&h.ctx, agent_launch(Some(local)))
            .await
            .unwrap_or_else(|e| panic!("{local:?} means local, got: {e}"));

        assert!(h.spy.launched().is_some(), "{local:?}: executor not called");
        assert_eq!(h.exec.calls(), Vec::<String>::new(), "{local:?}");
    }
}

#[tokio::test]
async fn start_feature_on_an_unknown_machine_names_it_and_creates_nothing() {
    let h = harness(RunnerAt::accepting());

    let err = start_feature(&h.ctx, agent_launch(Some("ghost-box")))
        .await
        .expect_err("an unknown machine is refused");

    assert!(err.contains("ghost-box"), "got: {err}");
    assert!(h.spy.launched().is_none(), "the executor was not called");
    assert_eq!(h.exec.calls(), Vec::<String>::new(), "no RPC was issued");
    assert_eq!(feature_ids(&h.ctx), Vec::<String>::new());
}

#[tokio::test]
async fn detached_options_with_a_local_placement_are_refused() {
    let h = harness(RunnerAt::accepting());
    let launch = AgentFeatureLaunch {
        max_cost_usd: Some(5.0),
        ..agent_launch(Some(""))
    };

    let err = start_feature(&h.ctx, launch)
        .await
        .expect_err("a cap on a local run would not be honoured");

    assert!(err.contains("max_cost_usd"), "got: {err}");
    assert!(h.spy.launched().is_none());
}

#[tokio::test]
async fn start_feature_on_a_runner_submits_detached() {
    let h = harness(RunnerAt::accepting());
    let launch = AgentFeatureLaunch {
        max_cost_usd: Some(5.0),
        ..agent_launch(Some("runner-1"))
    };

    let feature = start_feature(&h.ctx, launch)
        .await
        .expect("a detached start is submitted")
        .feature;

    assert!(h.spy.launched().is_none(), "the executor was not called");
    assert!(h.exec.calls().contains(&"rpc submit_run".to_string()));
    assert_eq!(feature_ids(&h.ctx), [feature.id.0]);
}

/// A startable ticket in a fresh Discovery on the harness's project, with
/// `stored` as its own placement choice.
fn stored_ticket(ctx: &AppContext, stored: &str) -> Ticket {
    let discovery = open_discovery(
        ctx,
        NewDiscovery {
            project_id: "p-1".to_string(),
            title: "placed work".to_string(),
            agent_kind: "claude-code".to_string(),
            model: None,
            effort: None,
            machine_id: None,
            staged_attachments: Vec::new(),
        },
    )
    .expect("the discovery opens");
    let t = Ticket {
        id: TicketId::from("t-1".to_string()),
        discovery_id: discovery.id,
        seq: 1,
        title: "the ticket".to_string(),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: Some(WorkflowId::from("w-1".to_string())),
        agent_kind: None,
        model: None,
        effort: None,
        machine_id: Some(MachineId::from(stored)),
        attachments: Vec::new(),
        state: TicketState::Unstarted,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    };
    ctx.tickets
        .upsert_batch(std::slice::from_ref(&t))
        .expect("the ticket is stored");
    t
}

fn stored_machine(ctx: &AppContext, t: &Ticket) -> Option<MachineId> {
    ctx.tickets
        .get(&t.id)
        .expect("the ticket reads")
        .expect("the ticket exists")
        .machine_id
}

#[tokio::test]
async fn a_machine_override_starts_one_ticket_launch_detached_and_is_not_saved() {
    let h = harness(RunnerAt::accepting());
    let t = stored_ticket(&h.ctx, "local");

    let feature = start_ticket(&h.ctx, &t.id, Some("runner-1".to_string()))
        .await
        .expect("the ticket starts on the runner")
        .feature;

    assert!(h.spy.launched().is_none(), "the executor was not called");
    assert!(h.exec.calls().contains(&"rpc submit_run".to_string()));
    assert_eq!(feature_ids(&h.ctx), [feature.id.0]);
    assert_eq!(stored_machine(&h.ctx, &t), Some(MachineId::from("local")));
}

#[tokio::test]
async fn a_blank_machine_override_keeps_the_tickets_own_placement() {
    for blank in ["", "  "] {
        let h = harness(RunnerAt::accepting());
        let t = stored_ticket(&h.ctx, "runner-1");

        start_ticket(&h.ctx, &t.id, Some(blank.to_string()))
            .await
            .unwrap_or_else(|e| panic!("{blank:?} is no override, got: {e}"));

        assert!(h.spy.launched().is_none(), "{blank:?}: started locally");
        assert!(
            h.exec.calls().contains(&"rpc submit_run".to_string()),
            "{blank:?}"
        );
    }
}
