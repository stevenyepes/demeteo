use super::SqliteAdapter;
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId, WorkflowId};
use crate::domain::models::{
    EffortLevel, Project, ProjectSettings, ProjectWorkflowOverride, Repository, WorktreeStrategy,
};
use crate::ports::db::ProjectRepository;
use rusqlite::Connection;

#[test]
fn test_update_and_delete_project() {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = SqliteAdapter::new(conn).unwrap();

    let project = Project {
        id: ProjectId::from("test_p1".to_string()),
        name: "Test Project".to_string(),
        compute_type: "local".to_string(),
        remote_host: None,
        status: "idle".to_string(),
        nodes: 4,
        spend: 0.0,
        tokens: 0,
        created_at: 123456,
    };
    adapter.add(project.clone()).unwrap();

    let projects = adapter.get_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "Test Project");

    let repo = Repository {
        id: RepositoryId::from("test_r1".to_string()),
        project_id: ProjectId::from("test_p1".to_string()),
        provider_id: ProviderId::from("github".to_string()),
        repo_path: "org/repo".to_string(),
    };
    adapter.add_repository(repo).unwrap();

    let repos = adapter
        .get_repositories_for(&ProjectId::from("test_p1".to_string()))
        .unwrap();
    assert_eq!(repos.len(), 1);

    let updated = Project {
        id: ProjectId::from("test_p1".to_string()),
        name: "Updated Project".to_string(),
        compute_type: "remote".to_string(),
        remote_host: Some(MachineId::from("machine_1".to_string())),
        status: "bootstrapping".to_string(),
        nodes: 8,
        spend: 10.5,
        tokens: 1000,
        created_at: 123456,
    };
    adapter.update(updated).unwrap();

    let projects = adapter.get_projects().unwrap();
    assert_eq!(projects[0].name, "Updated Project");
    assert_eq!(projects[0].compute_type, "remote");
    assert_eq!(
        projects[0].remote_host,
        Some(MachineId::from("machine_1".to_string()))
    );
    assert_eq!(projects[0].status, "bootstrapping");
    assert_eq!(projects[0].nodes, 0);

    adapter
        .delete_repositories_for(&ProjectId::from("test_p1".to_string()))
        .unwrap();
    let repos = adapter
        .get_repositories_for(&ProjectId::from("test_p1".to_string()))
        .unwrap();
    assert!(repos.is_empty());

    let repo = Repository {
        id: RepositoryId::from("test_r1_cascade".to_string()),
        project_id: ProjectId::from("test_p1".to_string()),
        provider_id: ProviderId::from("github".to_string()),
        repo_path: "org/repo-cascade".to_string(),
    };
    adapter.add_repository(repo).unwrap();

    adapter
        .delete(&ProjectId::from("test_p1".to_string()))
        .unwrap();
    let projects = adapter.get_projects().unwrap();
    assert!(projects.is_empty());

    let repos = adapter
        .get_repositories_for(&ProjectId::from("test_p1".to_string()))
        .unwrap();
    assert!(repos.is_empty());
}

fn ov(
    pid: &ProjectId,
    wid: &WorkflowId,
    step: Option<&str>,
    agent: Option<&str>,
    model: Option<&str>,
) -> ProjectWorkflowOverride {
    ProjectWorkflowOverride {
        effort: None,
        project_id: pid.clone(),
        workflow_id: wid.clone(),
        step_id: step.map(str::to_string),
        agent_kind: agent.map(str::to_string),
        model: model.map(str::to_string),
    }
}

#[test]
fn workflow_override_roundtrip_and_clear() {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = SqliteAdapter::new(conn).unwrap();
    let pid = ProjectId::from("p_ov".to_string());
    let wid = WorkflowId::from("wf_ov".to_string());

    let wf_level = || {
        adapter
            .list_overrides_for_workflow(&pid, &wid)
            .unwrap()
            .into_iter()
            .find(|o| o.step_id.is_none())
    };

    // No row initially.
    assert!(wf_level().is_none());
    assert!(adapter.list_workflow_overrides(&pid).unwrap().is_empty());

    // Upsert workflow-level with both fields set.
    adapter
        .upsert_workflow_override(ov(
            &pid,
            &wid,
            None,
            Some("claude-code"),
            Some("claude-opus-4-8"),
        ))
        .unwrap();
    let got = wf_level().unwrap();
    assert_eq!(got.step_id, None);
    assert_eq!(got.agent_kind.as_deref(), Some("claude-code"));
    assert_eq!(got.model.as_deref(), Some("claude-opus-4-8"));
    assert_eq!(adapter.list_workflow_overrides(&pid).unwrap().len(), 1);

    // Re-upsert (INSERT OR REPLACE) overwrites in place — still one row.
    adapter
        .upsert_workflow_override(ov(&pid, &wid, None, Some("opencode"), None))
        .unwrap();
    let got = wf_level().unwrap();
    assert_eq!(got.agent_kind.as_deref(), Some("opencode"));
    assert_eq!(got.model, None);
    assert_eq!(adapter.list_workflow_overrides(&pid).unwrap().len(), 1);

    // A step-level override coexists with the workflow-level row.
    adapter
        .upsert_workflow_override(ov(&pid, &wid, Some("s-impl"), Some("hermes"), None))
        .unwrap();
    let rows = adapter.list_overrides_for_workflow(&pid, &wid).unwrap();
    assert_eq!(rows.len(), 2);
    let step_row = rows
        .iter()
        .find(|o| o.step_id.as_deref() == Some("s-impl"))
        .unwrap();
    assert_eq!(step_row.agent_kind.as_deref(), Some("hermes"));

    // Clearing the step row leaves the workflow-level row intact.
    adapter
        .upsert_workflow_override(ov(&pid, &wid, Some("s-impl"), None, None))
        .unwrap();
    assert_eq!(
        adapter
            .list_overrides_for_workflow(&pid, &wid)
            .unwrap()
            .len(),
        1
    );

    // Clearing the workflow-level row empties the project.
    adapter
        .upsert_workflow_override(ov(&pid, &wid, None, None, None))
        .unwrap();
    assert!(wf_level().is_none());
    assert!(adapter.list_workflow_overrides(&pid).unwrap().is_empty());
}

/// The "all fields None → delete the row" rule must account for `effort` too:
/// an override that pins only an effort is a real override, and clearing the
/// agent/model of such a row must not take the effort down with it.
#[test]
fn workflow_override_with_only_an_effort_survives() {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = SqliteAdapter::new(conn).unwrap();
    let pid = ProjectId::from("p_eff_ov".to_string());
    let wid = WorkflowId::from("wf_eff_ov".to_string());

    let row = |o: &ProjectWorkflowOverride| ProjectWorkflowOverride {
        effort: o.effort,
        ..o.clone()
    };
    let mut only_effort = ov(&pid, &wid, None, None, None);
    only_effort.effort = Some(EffortLevel::Max);
    adapter.upsert_workflow_override(row(&only_effort)).unwrap();

    let rows = adapter.list_overrides_for_workflow(&pid, &wid).unwrap();
    assert_eq!(rows.len(), 1, "an effort-only override must not be deleted");
    assert_eq!(rows[0].effort, Some(EffortLevel::Max));
    assert_eq!(rows[0].agent_kind, None);

    // Genuinely empty (no agent, no model, no effort) still deletes.
    adapter
        .upsert_workflow_override(ov(&pid, &wid, None, None, None))
        .unwrap();
    assert!(adapter.list_workflow_overrides(&pid).unwrap().is_empty());
}

/// The project-wide default effort survives the settings upsert/select pair,
/// and an unset one reads back as `None` (inherit → `EffortLevel::DEFAULT`).
#[test]
fn project_settings_default_effort_round_trips() {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = SqliteAdapter::new(conn).unwrap();
    let pid = ProjectId::from("p_settings_effort".to_string());
    adapter
        .add(Project {
            id: pid.clone(),
            name: "effort settings".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        })
        .unwrap();

    let settings = |effort: Option<EffortLevel>| ProjectSettings {
        project_id: pid.clone(),
        worktree_strategy: WorktreeStrategy {
            default_branch: "main".to_string(),
            branch_prefix: "feat/".to_string(),
            test_command: None,
            build_command: None,
            coverage_command: None,
            conventions_file: None,
            pr_template: None,
            harnesses: None,
            validation_gates: None,
            prepare_command: None,
            extra_writable_paths: Vec::new(),
        },
        conflict_policy: "manual".to_string(),
        feature_lifecycle: "keep".to_string(),
        default_agent_kind: None,
        default_model: None,
        default_effort: effort,
        default_workflow_id: None,
        default_loop_iterations: None,
        // A sub-dollar value that also exercises the REAL column's precision
        // alongside the effort round-trip (both are the last-added columns,
        // most exposed to a SELECT column-index slip).
        default_max_budget_usd: Some(7.25),
        artifact_subdir: "artifacts/".to_string(),
        commit_artifacts: false,
        review_entrypoint: None,
        sync_resolver_agent_kind: None,
        sync_resolver_model: None,
        sync_resolver_effort: None,
        sync_review_before_push: None,
    };

    adapter
        .save_settings(settings(Some(EffortLevel::Medium)))
        .unwrap();
    let saved = adapter.get_settings(&pid).unwrap().unwrap();
    assert_eq!(saved.default_effort, Some(EffortLevel::Medium));
    assert_eq!(saved.default_max_budget_usd, Some(7.25));

    adapter.save_settings(settings(None)).unwrap();
    assert_eq!(
        adapter.get_settings(&pid).unwrap().unwrap().default_effort,
        None
    );
}

/// The project's chosen default Workflow survives the settings upsert/select
/// pair, and an unset one reads back as `None` — the state the launch path
/// resolves to "first workflow in the list". The neighbouring columns are
/// asserted alongside it because this is the newest column on both statements
/// and therefore the one a positional-index slip in the row mapping would
/// shift everything else past.
#[test]
fn project_settings_default_workflow_id_round_trips() {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = SqliteAdapter::new(conn).unwrap();
    let pid = ProjectId::from("p_settings_wf".to_string());
    adapter
        .add(Project {
            id: pid.clone(),
            name: "default workflow settings".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        })
        .unwrap();

    let settings = |workflow: Option<String>| ProjectSettings {
        project_id: pid.clone(),
        worktree_strategy: WorktreeStrategy {
            default_branch: "main".to_string(),
            branch_prefix: "feat/".to_string(),
            test_command: None,
            build_command: None,
            coverage_command: None,
            conventions_file: None,
            pr_template: None,
            harnesses: None,
            validation_gates: None,
            prepare_command: Some("npm ci".to_string()),
            extra_writable_paths: Vec::new(),
        },
        conflict_policy: "manual".to_string(),
        feature_lifecycle: "keep".to_string(),
        default_agent_kind: None,
        default_model: None,
        default_effort: Some(EffortLevel::Low),
        default_workflow_id: workflow,
        default_loop_iterations: None,
        default_max_budget_usd: Some(3.5),
        artifact_subdir: "artifacts/".to_string(),
        commit_artifacts: false,
        review_entrypoint: None,
        sync_resolver_agent_kind: None,
        sync_resolver_model: None,
        sync_resolver_effort: None,
        sync_review_before_push: None,
    };

    adapter
        .save_settings(settings(Some("wf_starter_feature".to_string())))
        .unwrap();
    let saved = adapter.get_settings(&pid).unwrap().unwrap();
    assert_eq!(
        saved.default_workflow_id,
        Some("wf_starter_feature".to_string())
    );
    assert_eq!(saved.default_effort, Some(EffortLevel::Low));
    assert_eq!(saved.default_max_budget_usd, Some(3.5));
    assert_eq!(
        saved.worktree_strategy.prepare_command,
        Some("npm ci".to_string())
    );

    // A workflow id is never rewritten to something else on the way out, so a
    // project that clears its choice must read back unset, not the last one.
    adapter.save_settings(settings(None)).unwrap();
    assert_eq!(
        adapter
            .get_settings(&pid)
            .unwrap()
            .unwrap()
            .default_workflow_id,
        None
    );
}

/// The `harnesses` column carries two fields (HB5): the map, and the user's
/// ordered selection of which of them gate validation. Both shapes it may hold
/// must read back intact — and the legacy bare map, which is what every row
/// written before HB5 contains and what a project with no selection still
/// writes, must survive a shape it was never told about.
#[test]
fn project_settings_harnesses_and_validation_gates_round_trip() {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = SqliteAdapter::new(conn).unwrap();
    let pid = ProjectId::from("p_settings_gates".to_string());
    adapter
        .add(Project {
            id: pid.clone(),
            name: "gates settings".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        })
        .unwrap();

    let harnesses: std::collections::HashMap<String, String> = [
        ("lint".to_string(), "npm run lint".to_string()),
        ("unit".to_string(), "npm test".to_string()),
    ]
    .into_iter()
    .collect();

    let settings = |gates: Option<Vec<String>>| ProjectSettings {
        project_id: pid.clone(),
        worktree_strategy: WorktreeStrategy {
            default_branch: "main".to_string(),
            branch_prefix: "feat/".to_string(),
            test_command: None,
            build_command: None,
            coverage_command: None,
            conventions_file: None,
            pr_template: None,
            harnesses: Some(harnesses.clone()),
            validation_gates: gates,
            prepare_command: None,
            extra_writable_paths: Vec::new(),
        },
        conflict_policy: "manual".to_string(),
        feature_lifecycle: "keep".to_string(),
        default_agent_kind: None,
        default_model: None,
        default_effort: None,
        default_workflow_id: None,
        default_loop_iterations: None,
        default_max_budget_usd: None,
        artifact_subdir: "artifacts/".to_string(),
        commit_artifacts: false,
        review_entrypoint: None,
        sync_resolver_agent_kind: None,
        sync_resolver_model: None,
        sync_resolver_effort: None,
        sync_review_before_push: None,
    };

    // No selection: the column keeps its pre-HB5 bare-map shape, and the map
    // must not be swallowed by the reader that now also understands an envelope.
    adapter.save_settings(settings(None)).unwrap();
    let saved = adapter.get_settings(&pid).unwrap().unwrap();
    assert_eq!(saved.worktree_strategy.harnesses, Some(harnesses.clone()));
    assert_eq!(saved.worktree_strategy.validation_gates, None);

    // With a selection, both fields survive — and the order is the user's
    // (cheap gates first), so it must not be re-sorted or set-ified.
    adapter
        .save_settings(settings(Some(vec!["unit".to_string(), "lint".to_string()])))
        .unwrap();
    let saved = adapter.get_settings(&pid).unwrap().unwrap();
    assert_eq!(saved.worktree_strategy.harnesses, Some(harnesses));
    assert_eq!(
        saved.worktree_strategy.validation_gates,
        Some(vec!["unit".to_string(), "lint".to_string()])
    );
}

/// Both column lists are written out in full, so a new column reaches the row
/// through two edits that must agree. When they don't, every value after the
/// disagreement reads back as its neighbour — which is why the two adjacent
/// TEXT columns are given values that would still be a legal answer for each
/// other.
#[test]
fn project_settings_review_entrypoint_round_trips() {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = SqliteAdapter::new(conn).unwrap();
    let pid = ProjectId::from("p_settings_review".to_string());
    adapter
        .add(Project {
            id: pid.clone(),
            name: "review entrypoint settings".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        })
        .unwrap();

    let settings = |entrypoint: Option<String>| ProjectSettings {
        project_id: pid.clone(),
        worktree_strategy: WorktreeStrategy {
            default_branch: "main".to_string(),
            branch_prefix: "feat/".to_string(),
            test_command: None,
            build_command: None,
            coverage_command: None,
            conventions_file: None,
            pr_template: None,
            harnesses: None,
            validation_gates: None,
            prepare_command: None,
            extra_writable_paths: Vec::new(),
        },
        conflict_policy: "manual".to_string(),
        feature_lifecycle: "keep".to_string(),
        default_agent_kind: None,
        default_model: None,
        default_effort: None,
        default_workflow_id: Some("wf_starter_code_review".to_string()),
        default_loop_iterations: None,
        default_max_budget_usd: None,
        artifact_subdir: "artifacts/".to_string(),
        commit_artifacts: false,
        review_entrypoint: entrypoint,
        sync_resolver_agent_kind: None,
        sync_resolver_model: None,
        sync_resolver_effort: None,
        sync_review_before_push: None,
    };

    adapter
        .save_settings(settings(Some("/code-review".to_string())))
        .unwrap();
    let saved = adapter.get_settings(&pid).unwrap().unwrap();
    assert_eq!(saved.review_entrypoint, Some("/code-review".to_string()));
    assert_eq!(
        saved.default_workflow_id,
        Some("wf_starter_code_review".to_string())
    );

    adapter.save_settings(settings(None)).unwrap();
    let cleared = adapter.get_settings(&pid).unwrap().unwrap();
    assert_eq!(cleared.review_entrypoint, None);
    assert_eq!(
        cleared.default_workflow_id,
        Some("wf_starter_code_review".to_string())
    );
}

/// The V44 triple, through the same two column lists — and read back beside a
/// value that would be a legal answer for either of its TEXT neighbours, so an
/// off-by-one in the positional read is a failure rather than a plausible row.
#[test]
fn project_settings_sync_resolver_default_round_trips() {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = SqliteAdapter::new(conn).unwrap();
    let pid = ProjectId::from("p_settings_resolver".to_string());
    adapter
        .add(Project {
            id: pid.clone(),
            name: "sync resolver settings".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        })
        .unwrap();

    let mut settings = crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = pid.clone();
    settings.review_entrypoint = Some("/code-review".to_string());
    settings.default_agent_kind = Some("opencode".to_string());
    settings.default_model = Some("sonnet".to_string());
    settings.default_effort = Some(EffortLevel::Max);
    settings.sync_resolver_agent_kind = Some("codex".to_string());
    settings.sync_resolver_model = Some("gpt-5-codex".to_string());
    settings.sync_resolver_effort = Some(EffortLevel::Low);
    // `false` and unset are different answers and the column is the only thing
    // that keeps them apart — a boolean written through an INTEGER column is
    // the one place `Some(false)` can come back as `None` unnoticed.
    settings.sync_review_before_push = Some(false);
    adapter.save_settings(settings.clone()).unwrap();

    let saved = adapter.get_settings(&pid).unwrap().unwrap();
    assert_eq!(saved.sync_resolver_agent_kind.as_deref(), Some("codex"));
    assert_eq!(saved.sync_resolver_model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(saved.sync_resolver_effort, Some(EffortLevel::Low));
    assert_eq!(saved.sync_review_before_push, Some(false));
    assert_eq!(saved.review_entrypoint.as_deref(), Some("/code-review"));
    assert_eq!(saved.default_agent_kind.as_deref(), Some("opencode"));
    assert_eq!(saved.default_model.as_deref(), Some("sonnet"));
    assert_eq!(saved.default_effort, Some(EffortLevel::Max));

    settings.sync_resolver_agent_kind = None;
    settings.sync_resolver_model = None;
    settings.sync_resolver_effort = None;
    settings.sync_review_before_push = None;
    adapter.save_settings(settings).unwrap();
    let cleared = adapter.get_settings(&pid).unwrap().unwrap();
    assert_eq!(cleared.sync_resolver_agent_kind, None);
    assert_eq!(cleared.sync_resolver_model, None);
    assert_eq!(cleared.sync_resolver_effort, None);
    assert_eq!(cleared.sync_review_before_push, None);
    assert_eq!(cleared.review_entrypoint.as_deref(), Some("/code-review"));
}
