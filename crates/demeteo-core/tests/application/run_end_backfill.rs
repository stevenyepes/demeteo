// Tests for `src/application/run_end_backfill.rs` (mirrored-tests convention).

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::ProjectId;
use crate::domain::models::{Feature, Project};
use rusqlite::Connection;

fn store() -> SqliteAdapter {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    ProjectRepository::add(
        &db,
        Project {
            id: ProjectId::from("p".to_string()),
            name: "p".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 1,
        },
    )
    .unwrap();
    db
}

fn add_feature(db: &SqliteAdapter, id: &str, status: &str) {
    let feature: Feature = serde_json::from_value(serde_json::json!({
        "id": id,
        "project_id": "p",
        "workflow_id": null,
        "title": id,
        "status": status,
        "total_cost": 0.0,
        "duration": "",
        "created_at": 1,
        "mr_state": "none",
    }))
    .unwrap();
    FeatureRepository::add(db, feature).unwrap();
}

fn log(db: &SqliteAdapter, id: &str, kind: &str, payload: &str) {
    db.append(id, kind, Some(payload), 1).unwrap();
}

fn statuses(db: &SqliteAdapter, id: &str) -> Vec<String> {
    db.list_since(id, 0)
        .unwrap()
        .into_iter()
        .filter(|row| row.kind == "feature_status")
        .filter_map(|row| logged_status(row.payload_json.as_deref()))
        .collect()
}

fn backfill(db: &SqliteAdapter) -> usize {
    backfill_run_ends(db, db, db, 2).unwrap()
}

#[test]
fn a_completed_feature_whose_log_stops_at_a_gate_is_logged_completed() {
    let db = store();
    add_feature(&db, "f-gate", "completed");
    log(&db, "f-gate", "feature_status", r#"{"status":"running"}"#);
    log(
        &db,
        "f-gate",
        "gate_required",
        r#"{"step_execution_id":"se-1","step_id":"s-gate"}"#,
    );

    assert_eq!(backfill(&db), 1);
    assert_eq!(statuses(&db, "f-gate"), ["running", "completed"]);
}

#[test]
fn a_detached_shadow_with_only_step_rows_is_logged_with_its_end() {
    let db = store();
    add_feature(&db, "f-shadow", "completed");
    log(
        &db,
        "f-shadow",
        "step_progress",
        r#"{"status":"completed","step_id":"s-sync-manual"}"#,
    );

    assert_eq!(backfill(&db), 1);
    assert_eq!(statuses(&db, "f-shadow"), ["completed"]);
}

#[test]
fn a_deleted_feature_left_at_a_gate_is_logged_cancelled() {
    let db = store();
    add_feature(&db, "f-deleted", "deleted");
    log(
        &db,
        "f-deleted",
        "feature_status",
        r#"{"status":"awaiting_gate"}"#,
    );

    assert_eq!(backfill(&db), 1);
    assert_eq!(statuses(&db, "f-deleted"), ["awaiting_gate", "cancelled"]);
}

#[test]
fn a_feature_never_logged_and_a_live_one_are_left_alone() {
    let db = store();
    add_feature(&db, "f-silent", "completed");
    add_feature(&db, "f-live", "awaiting_gate");
    log(&db, "f-live", "feature_status", r#"{"status":"running"}"#);

    assert_eq!(backfill(&db), 0);
    assert!(statuses(&db, "f-silent").is_empty());
    assert_eq!(statuses(&db, "f-live"), ["running"]);
}

#[test]
fn a_second_pass_appends_nothing() {
    let db = store();
    add_feature(&db, "f-gate", "completed");
    log(&db, "f-gate", "feature_status", r#"{"status":"running"}"#);

    assert_eq!(backfill(&db), 1);
    assert_eq!(backfill(&db), 0);
    assert_eq!(statuses(&db, "f-gate"), ["running", "completed"]);
}
