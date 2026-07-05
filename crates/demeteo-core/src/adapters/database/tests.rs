use super::SqliteAdapter;
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, Repository};
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
