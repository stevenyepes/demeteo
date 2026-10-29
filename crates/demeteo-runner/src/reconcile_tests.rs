// Tests extracted from `crates/demeteo-runner/src/reconcile.rs` (mirrored-tests convention). `super` = that module.

use super::settle_restart;
use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::domain::feature_origin::FeatureOrigin;
use demeteo_core::domain::ids::{FeatureId, GateDecisionId, ProjectId, StepExecutionId, StepId};
use demeteo_core::domain::models::{Feature, GateDecision, Project, StepExecution};
use demeteo_core::domain::reboot_budget::REBOOT_RETRY_BUDGET;
use demeteo_core::ports::db::{FeatureRepository, GateRepository, ProjectRepository};
use demeteo_core::ports::runner_run::{RunnerRun, RunnerRunPort};

const PROCESS_STARTED: i64 = 5_000;
const NOW: i64 = 6_000;

fn spec_json() -> String {
    serde_json::json!({
        "title": "t",
        "description": "d",
        "provider": { "kind": "github", "host": "github.com" },
        "repo_path": "o/r",
        "workflow_json": {},
        "agent_kind": null,
        "model": null,
        "loop_iterations": null
    })
    .to_string()
}

/// A run with a feature whose one step is a gate, already charged
/// `resume_count` restarts.
fn run_at_a_gate(label: &str, resume_count: i64) -> SqliteAdapter {
    let dir = demeteo_core::test_dir::scratch(&format!("demeteo_runner_reconcile_{label}"));
    let db = SqliteAdapter::new(demeteo_core::db::init_db(dir).expect("init_db"))
        .expect("migrations run");
    ProjectRepository::add(
        &db,
        Project {
            id: ProjectId::from("p-1"),
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
    .expect("seed the project");
    FeatureRepository::add(
        &db,
        Feature {
            id: FeatureId::from("f-1"),
            project_id: ProjectId::from("p-1"),
            workflow_id: None,
            workflow_version_id: None,
            title: "t".to_string(),
            description: String::new(),
            status: "awaiting_gate".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            created_at: 1,
            agent_kind: None,
            model: None,
            effort: None,
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
            origin: FeatureOrigin::default(),
            diff_base_branch: None,
            resolved_branch: None,
            launched_by: Default::default(),
        },
    )
    .expect("seed the feature");
    db.step_create(StepExecution {
        id: StepExecutionId::from("se-ship"),
        feature_id: FeatureId::from("f-1"),
        step_id: StepId::from("s-ship"),
        step_index: 0,
        step_kind: "gate".to_string(),
        status: "awaiting_gate".to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 1,
        updated_at: 1,
    })
    .expect("seed the step");

    db.get_or_create("run-1", &spec_json(), "", 1)
        .expect("seed the run");
    RunnerRunPort::update_status(
        &db,
        "run-1",
        "interrupted",
        Some("p-1"),
        Some("f-1"),
        None,
        None,
        1,
    )
    .expect("attach the feature");
    for _ in 0..resume_count {
        db.bump_resume_count("run-1").expect("charge a restart");
    }
    db
}

fn ask_the_gate(db: &SqliteAdapter, asked_at: i64) {
    GateRepository::create(
        db,
        GateDecision {
            id: GateDecisionId::from("gd-se-ship"),
            step_execution_id: StepExecutionId::from("se-ship"),
            decision: None,
            feedback: None,
            created_at: asked_at,
            auto_approved: false,
        },
    )
    .expect("seed the gate");
}

fn run(db: &SqliteAdapter) -> RunnerRun {
    RunnerRunPort::get(db, "run-1").expect("read").expect("row")
}

/// The incident: a run that sat at its ship gate while the runner restarted
/// more often than the budget allows.
#[test]
fn a_run_waiting_at_a_gate_is_reattached_however_often_the_host_restarts() {
    let db = run_at_a_gate("waiting", 0);
    ask_the_gate(&db, PROCESS_STARTED - 1);

    for restart in 0..REBOOT_RETRY_BUDGET + 2 {
        db.mark_all_running_interrupted(NOW).expect("shut down");
        let spec = settle_restart(&db, &db, &run(&db), PROCESS_STARTED, NOW);

        assert!(spec.is_some(), "restart {restart} left the run unattached");
        assert_eq!(run(&db).status, "running");
        assert_eq!(run(&db).resume_count, 0);
    }
}

#[test]
fn a_spent_budget_still_reattaches_a_run_that_was_only_waiting() {
    let db = run_at_a_gate("spent_waiting", REBOOT_RETRY_BUDGET);
    ask_the_gate(&db, PROCESS_STARTED - 1);

    assert!(settle_restart(&db, &db, &run(&db), PROCESS_STARTED, NOW).is_some());
    assert_eq!(run(&db).status, "running");
    assert_eq!(run(&db).error, None);
}

/// The engine's reconciliation asks "may this resume?" of the step it found
/// running — inside this process, so after `PROCESS_STARTED`.
#[test]
fn a_question_this_restart_posed_is_charged() {
    let db = run_at_a_gate("charged", 0);
    ask_the_gate(&db, PROCESS_STARTED + 1);

    assert!(settle_restart(&db, &db, &run(&db), PROCESS_STARTED, NOW).is_some());
    assert_eq!(run(&db).resume_count, 1);
}

#[test]
fn a_run_with_nothing_pending_is_charged() {
    let db = run_at_a_gate("no_question", 0);

    assert!(settle_restart(&db, &db, &run(&db), PROCESS_STARTED, NOW).is_some());
    assert_eq!(run(&db).resume_count, 1);
}

#[test]
fn interrupted_work_on_a_spent_budget_parks_the_run_as_failed() {
    let db = run_at_a_gate("spent_working", REBOOT_RETRY_BUDGET);
    ask_the_gate(&db, PROCESS_STARTED + 1);

    assert!(settle_restart(&db, &db, &run(&db), PROCESS_STARTED, NOW).is_none());
    let parked = run(&db);
    assert_eq!(parked.status, "failed");
    assert_eq!(
        parked.error.as_deref(),
        Some("unstable host: exceeded reboot-retry budget")
    );
    assert_eq!(parked.resume_count, REBOOT_RETRY_BUDGET);
}
