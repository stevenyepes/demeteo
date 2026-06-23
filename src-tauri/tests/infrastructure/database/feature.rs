use rusqlite::Connection;

use super::super::super::SqliteAdapter;
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::Feature;
use crate::domain::models::Project;
use crate::ports::db::ProjectRepository;
use crate::ports::db::{FeaturePatch, FeatureRepository};

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

#[test]
fn feature_update_status_preserves_cost_and_duration() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f13", "p1");
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            status: Some("completed".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.status, "completed");
    assert_eq!(f.total_cost, 0.0);
    assert_eq!(f.duration, "0s");
}

#[test]
fn feature_update_cost_set_explicitly() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f14", "p1");
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            total_cost: Some(Some(99.9)),
            ..Default::default()
        },
    )
    .unwrap();
    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.total_cost, 99.9);
}

#[test]
fn feature_update_cost_skipped_with_none() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f15", "p1");
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            total_cost: None,
            ..Default::default()
        },
    )
    .unwrap();
    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.total_cost, 0.0);
}

#[test]
fn feature_update_cost_flattened_with_some_none() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f16", "p1");
    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            total_cost: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.total_cost, 0.0);
}
