// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/subtask_run.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::{ProjectId, StepId};
use crate::domain::models::{Feature, Project, StepExecution};
use crate::ports::db::{FeatureRepository, ProjectRepository};
use rusqlite::Connection;

/// Minimal parent rows: `subtask_runs` carries enforced foreign keys to both
/// `features` and `step_executions`.
fn seed() -> (SqliteAdapter, FeatureId, StepExecutionId) {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    let pid = ProjectId::from("p-1".to_string());
    let fid = FeatureId::from("f-1".to_string());
    let sid = StepExecutionId::from("se-1".to_string());
    ProjectRepository::add(
        &db,
        Project {
            id: pid.clone(),
            name: "p".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    )
    .unwrap();
    FeatureRepository::add(
        &db,
        Feature {
            id: fid.clone(),
            project_id: pid,
            workflow_id: None,
            title: "f".to_string(),
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
    db.step_create(StepExecution {
        last_failure_fingerprint: None,
        id: sid.clone(),
        feature_id: fid.clone(),
        step_id: StepId::from("s-impl".to_string()),
        step_index: 0,
        step_kind: "sequence".to_string(),
        status: "running".to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: vec![],
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        created_at: 1000,
        updated_at: 1000,
    })
    .unwrap();
    (db, fid, sid)
}

fn count_with_status(db: &SqliteAdapter, status: &str) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM subtask_runs WHERE status = ?1",
        params![status],
        |r| r.get(0),
    )
    .unwrap()
}

/// The project dashboard counts `running` rows into its live "nodes" figure,
/// so the row must be `running` between start and finish and never after.
#[test]
fn a_task_run_opens_running_and_closes_completed() {
    let (db, fid, sid) = seed();
    db.subtask_run_start(
        "sr-1",
        &fid,
        &sid,
        "task-1",
        "f-1-s-impl-task-1",
        "/tmp/wt",
        "feature/x_subtask_f-1-step-s-impl",
        100,
    )
    .unwrap();
    assert_eq!(count_with_status(&db, "running"), 1);

    db.subtask_run_finish("sr-1", "completed", 0.42, 1234, None, 200)
        .unwrap();
    assert_eq!(count_with_status(&db, "running"), 0);
    assert_eq!(count_with_status(&db, "completed"), 1);

    let conn = db.conn.lock().unwrap();
    let (cost, tokens, ended): (f64, i64, Option<i64>) = conn
        .query_row(
            "SELECT cost_usd, tokens, ended_at FROM subtask_runs WHERE id = 'sr-1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!((cost - 0.42).abs() < f64::EPSILON);
    assert_eq!(tokens, 1234);
    assert_eq!(ended, Some(200));
}

#[test]
fn a_failed_task_records_its_error() {
    let (db, fid, sid) = seed();
    db.subtask_run_start(
        "sr-2",
        &fid,
        &sid,
        "task-2",
        "agent-2",
        "/tmp/wt",
        "feature/x_subtask_y",
        100,
    )
    .unwrap();
    db.subtask_run_finish("sr-2", "failed", 0.1, 50, Some("agent error: timeout"), 300)
        .unwrap();

    let conn = db.conn.lock().unwrap();
    let err: Option<String> = conn
        .query_row(
            "SELECT error_message FROM subtask_runs WHERE id = 'sr-2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(err.as_deref(), Some("agent error: timeout"));
}
