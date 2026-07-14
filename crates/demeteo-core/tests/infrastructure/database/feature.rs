use rusqlite::Connection;

use super::super::super::SqliteAdapter;
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::Feature;
use crate::domain::models::Project;
use crate::domain::models::{EffortLevel, StepOverride};
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
            effort: None,
            id: fid.clone(),
            project_id: pid,
            workflow_id: None,
            title: "Test Feature".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: 1000,
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
        },
    )
    .unwrap();
    fid
}

#[test]
fn feature_description_round_trips_through_get_and_get_active() {
    let adapter = setup();
    let pid = ProjectId::from("p_desc".to_string());
    let fid = FeatureId::from("f_desc".to_string());
    let _ = ProjectRepository::add(
        &adapter,
        Project {
            id: pid.clone(),
            name: "desc project".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    );
    let body = "Implement OAuth\n\nWith PKCE and refresh tokens.";
    FeatureRepository::add(
        &adapter,
        Feature {
            effort: None,
            id: fid.clone(),
            project_id: pid.clone(),
            workflow_id: None,
            title: "Add OAuth".to_string(),
            description: body.to_string(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: 1000,
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
        },
    )
    .unwrap();

    // Persisted, and returned by the single-feature read the pipeline view uses.
    assert_eq!(adapter.get(&fid).unwrap().unwrap().description, body);
    // ...and by the active-list read the project home renders.
    let active = adapter.get_active(&pid).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].description, body);
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

/// AC7: a feature launched with an effort and a per-step effort override reads
/// back with exactly those values (the per-step one riding inside the
/// `step_overrides_json` blob, not a column of its own).
#[test]
fn feature_effort_and_step_override_effort_round_trip() {
    let adapter = setup();
    let pid = ProjectId::from("p_effort".to_string());
    let fid = FeatureId::from("f_effort".to_string());
    let _ = ProjectRepository::add(
        &adapter,
        Project {
            id: pid.clone(),
            name: "effort project".to_string(),
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
        &adapter,
        Feature {
            id: fid.clone(),
            project_id: pid.clone(),
            workflow_id: None,
            title: "Effort".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: 1000,
            agent_kind: None,
            model: None,
            effort: Some(EffortLevel::XHigh),
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            step_overrides: vec![StepOverride {
                step_id: "s-impl".to_string(),
                agent_kind: None,
                model: None,
                effort: Some(EffortLevel::Low),
            }],
            attachments: Vec::new(),
        },
    )
    .unwrap();

    let f = adapter.get(&fid).unwrap().unwrap();
    assert_eq!(f.effort, Some(EffortLevel::XHigh));
    assert_eq!(f.step_overrides[0].effort, Some(EffortLevel::Low));
    // ...and through the active-list read the project home uses.
    let active = adapter.get_active(&pid).unwrap();
    assert_eq!(active[0].effort, Some(EffortLevel::XHigh));
}

/// A row written before V29 has `effort` NULL, and a row carrying a value this
/// build doesn't know (a downgrade, a hand-edited DB) must degrade to "inherit"
/// rather than failing the read.
#[test]
fn feature_effort_is_none_when_absent_or_unknown() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f_stale", "p_stale");
    assert_eq!(adapter.get(&fid).unwrap().unwrap().effort, None);

    adapter
        .conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE features SET effort = 'ultra' WHERE id = ?1",
            rusqlite::params![fid.0],
        )
        .unwrap();
    assert_eq!(adapter.get(&fid).unwrap().unwrap().effort, None);
}

/// `Some(Some(v))` pins the effort; `Some(None)` clears it back to inherit;
/// a `None` patch field leaves the column alone.
#[test]
fn feature_patch_sets_and_clears_effort() {
    let adapter = setup();
    let fid = make_feature(&adapter, "f_patch_effort", "p_patch_effort");

    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            effort: Some(Some(EffortLevel::Max)),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        adapter.get(&fid).unwrap().unwrap().effort,
        Some(EffortLevel::Max)
    );

    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            status: Some("done".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        adapter.get(&fid).unwrap().unwrap().effort,
        Some(EffortLevel::Max)
    );

    FeatureRepository::update(
        &adapter,
        &fid,
        &FeaturePatch {
            effort: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(adapter.get(&fid).unwrap().unwrap().effort, None);
}
