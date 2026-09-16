// Tests extracted from `crates/demeteo-core/src/application/agent_surface/writes.rs`
// (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::adapters::step_executor::setup::fetch_default_settings;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::models::{EffortLevel, Project};

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
