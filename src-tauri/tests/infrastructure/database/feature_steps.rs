use rusqlite::Connection;

use super::super::super::SqliteAdapter;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId};
use crate::domain::models::Project;
use crate::domain::models::{Feature, StepExecution};
use crate::ports::db::ProjectRepository;
use crate::ports::db::{FeatureRepository, StepExecutionPatch};

fn setup() -> SqliteAdapter {
    let conn = Connection::open_in_memory().unwrap();
    SqliteAdapter::new(conn).unwrap()
}

fn make_feature(adapter: &SqliteAdapter, id: &str, project_id: &str) -> FeatureId {
    let fid = FeatureId::from(id.to_string());
    let pid = ProjectId::from(project_id.to_string());
    let _ = ProjectRepository::add(
        adapter,
        Project {
            id: pid.clone(),
            name: format!("project_{}", project_id),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    );
    FeatureRepository::add(
        adapter,
        Feature {
            id: fid.clone(),
            project_id: pid,
            workflow_id: None,
            title: "Test Feature".to_string(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: 1000,
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
        },
    )
    .unwrap();
    fid
}

fn make_step(
    adapter: &SqliteAdapter,
    id: &str,
    feature_id: &FeatureId,
    error_message: Option<&str>,
) -> StepExecutionId {
    let sid = StepExecutionId::from(id.to_string());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    adapter
        .step_create(StepExecution {
            id: sid.clone(),
            feature_id: feature_id.clone(),
            step_id: StepId::from("s1".to_string()),
            step_index: 0,
            step_kind: "agent".to_string(),
            status: "failed".to_string(),
            cost_usd: Some(1.0),
            tokens: Some(0),
            wall_clock_secs: Some(60),
            artifact_path: Some("/tmp/artifact".to_string()),
            artifact_paths: vec![],
            error_message: error_message.map(|s| s.to_string()),
            iteration_count: 0,
            created_at: now,
            updated_at: now,
        })
        .unwrap();
    sid
}

fn read_error(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Option<String> {
    adapter.step_get(sid).unwrap().unwrap().error_message
}

fn read_cost(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Option<f64> {
    adapter.step_get(sid).unwrap().unwrap().cost_usd
}

fn read_wall_clock(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Option<u64> {
    adapter.step_get(sid).unwrap().unwrap().wall_clock_secs
}

fn read_artifact(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Option<String> {
    adapter.step_get(sid).unwrap().unwrap().artifact_path
}

fn read_artifact_paths(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Vec<String> {
    adapter.step_get(sid).unwrap().unwrap().artifact_paths
}

#[test]
fn step_error_set_some_some() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f1", "p1");
    let sid = make_step(&adapter, "s1", &fid, None);
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                error_message: Some(Some("new error".to_string())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(read_error(&adapter, &sid), Some("new error".to_string()));
}

#[test]
fn step_error_clear_with_some_none() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f2", "p1");
    let sid = make_step(&adapter, "s2", &fid, Some("old error"));
    assert_eq!(read_error(&adapter, &sid), Some("old error".to_string()));
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                error_message: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(read_error(&adapter, &sid), None);
}

#[test]
fn step_error_none_leaves_unchanged() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f3", "p1");
    let sid = make_step(&adapter, "s3", &fid, Some("persistent"));
    adapter
        .step_update(&sid, &StepExecutionPatch::default())
        .unwrap();
    assert_eq!(read_error(&adapter, &sid), Some("persistent".to_string()));
}

#[test]
fn step_cost_set_some_some() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f4", "p1");
    let sid = make_step(&adapter, "s4", &fid, None);
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                cost_usd: Some(Some(42.5)),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(read_cost(&adapter, &sid), Some(42.5));
}

#[test]
fn step_cost_clear_with_some_none() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f5", "p1");
    let sid = make_step(&adapter, "s5", &fid, None);
    assert_eq!(read_cost(&adapter, &sid), Some(1.0));
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                cost_usd: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(read_cost(&adapter, &sid), None);
}

#[test]
fn step_cost_none_leaves_unchanged() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f6", "p1");
    let sid = make_step(&adapter, "s6", &fid, None);
    adapter
        .step_update(&sid, &StepExecutionPatch::default())
        .unwrap();
    assert_eq!(read_cost(&adapter, &sid), Some(1.0));
}

#[test]
fn step_wall_set_some_some() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f7", "p1");
    let sid = make_step(&adapter, "s7", &fid, None);
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                wall_clock_secs: Some(Some(120)),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(read_wall_clock(&adapter, &sid), Some(120));
}

#[test]
fn step_wall_clear_with_some_none() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f8", "p1");
    let sid = make_step(&adapter, "s8", &fid, None);
    assert_eq!(read_wall_clock(&adapter, &sid), Some(60));
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                wall_clock_secs: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(read_wall_clock(&adapter, &sid), None);
}

#[test]
fn step_wall_none_leaves_unchanged() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f9", "p1");
    let sid = make_step(&adapter, "s9", &fid, None);
    adapter
        .step_update(&sid, &StepExecutionPatch::default())
        .unwrap();
    assert_eq!(read_wall_clock(&adapter, &sid), Some(60));
}

#[test]
fn step_artifact_set_some_some() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f10", "p1");
    let sid = make_step(&adapter, "s10", &fid, None);
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                artifact_path: Some(Some("/new/path".to_string())),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(read_artifact(&adapter, &sid), Some("/new/path".to_string()));
}

#[test]
fn step_artifact_clear_with_some_none() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f11", "p1");
    let sid = make_step(&adapter, "s11", &fid, None);
    assert_eq!(
        read_artifact(&adapter, &sid),
        Some("/tmp/artifact".to_string())
    );
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                artifact_path: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(read_artifact(&adapter, &sid), None);
}

#[test]
fn step_artifact_none_leaves_unchanged() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f12", "p1");
    let sid = make_step(&adapter, "s12", &fid, None);
    adapter
        .step_update(&sid, &StepExecutionPatch::default())
        .unwrap();
    assert_eq!(
        read_artifact(&adapter, &sid),
        Some("/tmp/artifact".to_string())
    );
}

#[test]
fn step_artifact_paths_set_replaces_list_and_mirrors_legacy() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f100", "p1");
    let sid = make_step(&adapter, "s100", &fid, None);
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                artifact_paths: Some(vec!["/a/x.md".to_string(), "/a/y.diff".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();
    let paths = read_artifact_paths(&adapter, &sid);
    assert_eq!(paths, vec!["/a/x.md".to_string(), "/a/y.diff".to_string()]);
    assert_eq!(read_artifact(&adapter, &sid), Some("/a/x.md".to_string()));
}

#[test]
fn step_artifact_paths_clears_legacy_when_empty() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f101", "p1");
    let sid = make_step(&adapter, "s101", &fid, None);
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                artifact_paths: Some(vec![]),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(read_artifact_paths(&adapter, &sid).is_empty());
    assert_eq!(read_artifact(&adapter, &sid), None);
}

#[test]
fn step_artifact_path_only_mirrors_into_list() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f102", "p1");
    let sid = make_step(&adapter, "s102", &fid, None);
    adapter
        .step_update(
            &sid,
            &StepExecutionPatch {
                artifact_path: Some(Some("/legacy/only".to_string())),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(read_artifact_paths(&adapter, &sid).is_empty());
    assert_eq!(
        read_artifact(&adapter, &sid),
        Some("/legacy/only".to_string())
    );
}
