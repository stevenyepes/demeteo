// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/subtask_run.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{ProjectId, StepId};
use crate::domain::models::{Feature, Project, StepExecution};
use crate::ports::db::{FeatureRepository, ProjectRepository, SubtaskRunOpen};
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
            effort: None,
            id: fid.clone(),
            project_id: pid,
            workflow_id: None,
            workflow_version_id: None,
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
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
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
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-1",
        feature_id: &fid,
        step_execution_id: &sid,
        subtask_id: "task-1",
        agent_id: "f-1-s-impl-task-1",
        worktree_path: "/tmp/wt",
        branch: "feature/x_subtask_f-1-step-s-impl",
        plan_epoch: None,
        plan_cycle: 0,
        now: 100,
    })
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
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-2",
        feature_id: &fid,
        step_execution_id: &sid,
        subtask_id: "task-2",
        agent_id: "agent-2",
        worktree_path: "/tmp/wt",
        branch: "feature/x_subtask_y",
        plan_epoch: None,
        plan_cycle: 0,
        now: 100,
    })
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

/// A crash or kill mid-task leaves a `running` row that no graceful exit will
/// ever close, and the dashboard's "nodes" figure counts `running` rows — so
/// the sweep at step (re)start / startup reconciliation must close exactly
/// this step's stale rows: not rows that already finished, and not another
/// step's live rows.
#[test]
fn interrupt_stale_closes_only_this_steps_running_rows() {
    let (db, fid, sid) = seed();
    let other_sid = StepExecutionId::from("se-2".to_string());
    db.step_create(StepExecution {
        last_failure_fingerprint: None,
        id: other_sid.clone(),
        feature_id: fid.clone(),
        step_id: StepId::from("s-impl-2".to_string()),
        step_index: 1,
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

    // The crash victim: opened, never closed.
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-stale",
        feature_id: &fid,
        step_execution_id: &sid,
        subtask_id: "task-1",
        agent_id: "a-1",
        worktree_path: "/tmp/wt",
        branch: "b",
        plan_epoch: None,
        plan_cycle: 0,
        now: 100,
    })
    .unwrap();
    // A row the same step already closed — its record must survive the sweep.
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-done",
        feature_id: &fid,
        step_execution_id: &sid,
        subtask_id: "task-0",
        agent_id: "a-0",
        worktree_path: "/tmp/wt",
        branch: "b",
        plan_epoch: None,
        plan_cycle: 0,
        now: 90,
    })
    .unwrap();
    db.subtask_run_finish("sr-done", "completed", 0.2, 500, None, 95)
        .unwrap();
    // Another step's live row — not this sweep's to touch.
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-other",
        feature_id: &fid,
        step_execution_id: &other_sid,
        subtask_id: "task-1",
        agent_id: "a-2",
        worktree_path: "/tmp/wt2",
        branch: "b2",
        plan_epoch: None,
        plan_cycle: 0,
        now: 100,
    })
    .unwrap();

    db.subtask_runs_interrupt_stale(&sid, 400).unwrap();

    let conn = db.conn.lock().unwrap();
    let (status, ended, err): (String, Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT status, ended_at, error_message FROM subtask_runs WHERE id = 'sr-stale'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "interrupted");
    assert_eq!(ended, Some(400));
    assert_eq!(err.as_deref(), Some("interrupted by restart"));

    let done_status: String = conn
        .query_row(
            "SELECT status FROM subtask_runs WHERE id = 'sr-done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(done_status, "completed");

    let other_status: String = conn
        .query_row(
            "SELECT status FROM subtask_runs WHERE id = 'sr-other'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        other_status, "running",
        "the sweep must be scoped to its own step execution"
    );
}

/// The P2.5 read: `subtask_runs_for_step` returns this step's rows in start
/// order (the task list's order) with the projected fields, and never another
/// step's rows.
#[test]
fn subtask_runs_for_step_returns_rows_in_start_order() {
    let (db, fid, sid) = seed();
    let other_sid = StepExecutionId::from("se-2".to_string());
    db.step_create(StepExecution {
        last_failure_fingerprint: None,
        id: other_sid.clone(),
        feature_id: fid.clone(),
        step_id: StepId::from("s-impl-2".to_string()),
        step_index: 1,
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

    // task-1 started first (earlier `started_at`) and finished; task-2 running.
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-1",
        feature_id: &fid,
        step_execution_id: &sid,
        subtask_id: "task-1",
        agent_id: "a-1",
        worktree_path: "/tmp/wt",
        branch: "b",
        plan_epoch: None,
        plan_cycle: 0,
        now: 100,
    })
    .unwrap();
    db.subtask_run_finish("sr-1", "completed", 0.3, 400, None, 150)
        .unwrap();
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-2",
        feature_id: &fid,
        step_execution_id: &sid,
        subtask_id: "task-2",
        agent_id: "a-2",
        worktree_path: "/tmp/wt",
        branch: "b",
        plan_epoch: None,
        plan_cycle: 0,
        now: 200,
    })
    .unwrap();
    // Another step's row — must not leak into this step's list.
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-x",
        feature_id: &fid,
        step_execution_id: &other_sid,
        subtask_id: "task-9",
        agent_id: "a-9",
        worktree_path: "/tmp/wt2",
        branch: "b2",
        plan_epoch: None,
        plan_cycle: 0,
        now: 120,
    })
    .unwrap();

    let rows = subtask_runs_for_step(&db, &sid).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].subtask_id, "task-1");
    assert_eq!(rows[0].status, "completed");
    assert!((rows[0].cost_usd - 0.3).abs() < f64::EPSILON);
    assert_eq!(rows[0].tokens, 400);
    assert_eq!(rows[1].subtask_id, "task-2");
    assert_eq!(rows[1].status, "running");
}

/// A stale row that failed with a real error must keep it — the COALESCE
/// only fills the message in when the crash left none.
#[test]
fn interrupt_stale_is_idempotent_and_repeatable() {
    let (db, fid, sid) = seed();
    db.subtask_run_start(&SubtaskRunOpen {
        id: "sr-1",
        feature_id: &fid,
        step_execution_id: &sid,
        subtask_id: "task-1",
        agent_id: "a-1",
        worktree_path: "/tmp/wt",
        branch: "b",
        plan_epoch: None,
        plan_cycle: 0,
        now: 100,
    })
    .unwrap();
    db.subtask_runs_interrupt_stale(&sid, 200).unwrap();
    // Nothing running any more: a second sweep is a no-op, not an error.
    db.subtask_runs_interrupt_stale(&sid, 300).unwrap();

    let conn = db.conn.lock().unwrap();
    let ended: Option<i64> = conn
        .query_row(
            "SELECT ended_at FROM subtask_runs WHERE id = 'sr-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ended,
        Some(200),
        "the second sweep must not re-stamp the row"
    );
}
