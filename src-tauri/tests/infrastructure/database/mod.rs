use super::SqliteAdapter;
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId, WorkflowId};
use crate::domain::models::{Project, ProjectWorkflowOverride, Repository};
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
