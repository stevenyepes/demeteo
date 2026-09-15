use crate::adapters::step_executor::setup::fetch_default_settings;
use crate::domain::models::{apply_run_shape_patch, EffortLevel, RunShapePatch};

fn arbitrary_patch() -> RunShapePatch {
    RunShapePatch {
        default_agent_kind: Some("claude-code".to_string()),
        default_model: Some("claude-opus".to_string()),
        default_effort: Some(EffortLevel::Low),
        default_workflow_id: Some("wf-1".to_string()),
        artifact_subdir: "reports/".to_string(),
        commit_artifacts: true,
        sync_resolver_agent_kind: Some("codex".to_string()),
        sync_resolver_model: Some("gpt-5".to_string()),
        sync_resolver_effort: Some(EffortLevel::Max),
    }
}

#[test]
fn apply_leaves_max_budget_untouched() {
    let mut settings = fetch_default_settings();
    settings.default_max_budget_usd = Some(42.5);

    let result = apply_run_shape_patch(settings, arbitrary_patch());

    assert_eq!(result.default_max_budget_usd, Some(42.5));
}

#[test]
fn apply_leaves_all_excluded_fields_untouched() {
    let mut settings = fetch_default_settings();
    settings.default_max_budget_usd = Some(42.5);
    settings.default_loop_iterations = Some(7);
    settings.sync_review_before_push = Some(false);
    settings.worktree_strategy.default_branch = "trunk".to_string();
    settings.feature_lifecycle = "delete".to_string();

    let result = apply_run_shape_patch(settings, arbitrary_patch());

    assert_eq!(result.default_max_budget_usd, Some(42.5));
    assert_eq!(result.default_loop_iterations, Some(7));
    assert_eq!(result.sync_review_before_push, Some(false));
    assert_eq!(result.worktree_strategy.default_branch, "trunk");
    assert_eq!(result.feature_lifecycle, "delete");
}

#[test]
fn apply_copies_all_nine_patch_fields() {
    let mut settings = fetch_default_settings();
    settings.default_agent_kind = Some("opencode".to_string());
    settings.default_model = Some("old-model".to_string());
    settings.default_effort = Some(EffortLevel::High);
    settings.default_workflow_id = Some("wf-old".to_string());
    settings.artifact_subdir = "artifacts/".to_string();
    settings.commit_artifacts = false;
    settings.sync_resolver_agent_kind = Some("hermes".to_string());
    settings.sync_resolver_model = Some("old-sync-model".to_string());
    settings.sync_resolver_effort = Some(EffortLevel::Low);

    let patch = arbitrary_patch();
    let result = apply_run_shape_patch(settings, patch.clone());

    assert_eq!(result.default_agent_kind, patch.default_agent_kind);
    assert_eq!(result.default_model, patch.default_model);
    assert_eq!(result.default_effort, patch.default_effort);
    assert_eq!(result.default_workflow_id, patch.default_workflow_id);
    assert_eq!(result.artifact_subdir, patch.artifact_subdir);
    assert_eq!(result.commit_artifacts, patch.commit_artifacts);
    assert_eq!(
        result.sync_resolver_agent_kind,
        patch.sync_resolver_agent_kind
    );
    assert_eq!(result.sync_resolver_model, patch.sync_resolver_model);
    assert_eq!(result.sync_resolver_effort, patch.sync_resolver_effort);
}
