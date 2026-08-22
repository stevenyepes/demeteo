//! The active-predecessor guard: a stale retry or gate click must not unblock
//! the executor while an earlier step is still in flight.

use super::harness::build_test_executor;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{
    FeatureId, GateDecisionId, ProjectId, StepExecutionId, StepId, WorkflowId,
};
use crate::domain::models::{Feature, GateDecision, StepExecution};
use crate::error::AppError;
use crate::paths;
use crate::ports::db::{FeatureRepository, GateRepository, ProjectRepository};
use crate::ports::step_executor::{GatePresenter, StepExecutor};

/// `step_retry` on a failed step whose predecessor is still in
/// `running` (or any non-terminal) status must be rejected with
/// `AppError::validation` naming the blocking step. This is the core
/// race-guard: a stale retry click must not unblock the executor while
/// an earlier step is still in flight.
#[tokio::test]
async fn test_step_retry_blocked_by_active_predecessor() {
    let (executor, db, temp_dir) = build_test_executor("retry_blocked").await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-guard"),
            name: "guard-test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();

    features
        .add(Feature {
            effort: None,
            id: FeatureId::from("f-guard"),
            project_id: ProjectId::from("p-guard"),
            workflow_id: Some(WorkflowId::from("w-guard")),
            workflow_version_id: None,
            title: "guard feature".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            created_at: now,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .unwrap();

    // Three steps:
    //   index 0: completed        (terminal — not a blocker)
    //   index 1: running          (BLOCKER — must be named in the error)
    //   index 2: failed           (target for retry)
    for (idx, status) in [(0u32, "completed"), (1, "running"), (2, "failed")] {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(format!("se-guard-{idx}")),
                feature_id: FeatureId::from("f-guard"),
                step_id: StepId::from(format!("step-{idx}")),
                step_index: idx,
                step_kind: "agent".to_string(),
                status: status.to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    let err = executor
        .step_retry("se-guard-2", None, None, None)
        .await
        .expect_err("retry must be blocked by a running predecessor");
    match err {
        AppError::Validation { message } => {
            assert!(
                message.contains("step-1"),
                "blocking step id must be named in the message, got: {message}"
            );
            assert!(
                message.contains("running"),
                "blocking status must be named in the message, got: {message}"
            );
        }
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// `gate_decide` on an `awaiting_gate` step whose predecessor is still
/// in `running` must also be rejected with `AppError::validation`.
/// Same race surface, same guard, same message contract — but routed
/// through the `GatePresenter` trait instead of `StepExecutor`.
#[tokio::test]
async fn test_gate_decide_blocked_by_active_predecessor() {
    let (executor, db, temp_dir) = build_test_executor("gate_blocked").await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;
    let gates: &dyn GateRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-gg"),
            name: "gate-guard-test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();

    features
        .add(Feature {
            effort: None,
            id: FeatureId::from("f-gg"),
            project_id: ProjectId::from("p-gg"),
            workflow_id: Some(WorkflowId::from("w-gg")),
            workflow_version_id: None,
            title: "gate guard feature".to_string(),
            description: String::new(),
            status: "awaiting_gate".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            created_at: now,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .unwrap();

    // Predecessor still verifying (another non-terminal status the
    // guard must catch), gate step in awaiting_gate.
    for (idx, status) in [(0u32, "verifying"), (1, "awaiting_gate")] {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(format!("se-gg-{idx}")),
                feature_id: FeatureId::from("f-gg"),
                step_id: StepId::from(format!("step-{idx}")),
                step_index: idx,
                step_kind: if idx == 1 {
                    "gate".to_string()
                } else {
                    "agent".to_string()
                },
                status: status.to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    gates
        .create(GateDecision {
            id: GateDecisionId::from("gd-gg-1"),
            step_execution_id: StepExecutionId::from("se-gg-1"),
            decision: None,
            feedback: None,
            created_at: now,
        })
        .unwrap();

    let err = executor
        .gate_decide("se-gg-1", "approve", None)
        .await
        .expect_err("gate decide must be blocked by a verifying predecessor");
    match err {
        AppError::Validation { message } => {
            assert!(
                message.contains("step-0"),
                "blocking step id must be named in the message, got: {message}"
            );
            assert!(
                message.contains("verifying"),
                "blocking status must be named in the message, got: {message}"
            );
        }
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// Once the blocking predecessor transitions to a terminal state, the
/// guard must let the retry proceed. This is the symmetry check: the
/// guard rejects when it should, accepts when it should.
#[tokio::test]
async fn test_step_retry_unblocks_when_predecessor_is_terminal() {
    let (executor, db, temp_dir) = build_test_executor("retry_unblocks").await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-unb"),
            name: "unblock-test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();

    features
        .add(Feature {
            effort: None,
            id: FeatureId::from("f-unb"),
            project_id: ProjectId::from("p-unb"),
            workflow_id: Some(WorkflowId::from("w-unb")),
            workflow_version_id: None,
            title: "unblock feature".to_string(),
            description: String::new(),
            status: "failed".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            created_at: now,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .unwrap();

    // All earlier steps are terminal (completed, skipped, failed).
    // Only failed/skipped/completed are non-blocking; the guard must
    // short-circuit to Ok and let `replay_steps_from` take over (which
    // here will fail downstream because there's no real driver / no
    // git repo, but the guard itself must not be the one failing).
    for (idx, status) in [(0u32, "completed"), (1, "skipped"), (2, "failed")] {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(format!("se-unb-{idx}")),
                feature_id: FeatureId::from("f-unb"),
                step_id: StepId::from(format!("step-{idx}")),
                step_index: idx,
                step_kind: "agent".to_string(),
                status: status.to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    // The guard passes; what comes after is replay_steps_from which
    // expects a real git worktree + project setup. The fake exec
    // returns empty / stub data, so the call will error downstream —
    // but it MUST NOT be an AppError::Validation with the "still"
    // phrase (that would mean the guard fired when it shouldn't).
    let result = executor.step_retry("se-unb-2", None, None, None).await;
    if let Err(AppError::Validation { ref message }) = result {
        panic!("guard fired despite all predecessors being terminal: {message}");
    }
    // Any other Err (e.g. driver spawn failure, missing workflow) is
    // acceptable for this test — we only care that the guard didn't
    // false-positive.

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The `assert_no_active_predecessors` helper itself: directly
/// unit-test the precondition scan without going through the
/// `step_retry` / `gate_decide` plumbing. Easier to assert the exact
/// message format and the precedence rule (lower `step_index`
/// wins when multiple predecessors are non-terminal).
#[tokio::test]
async fn test_assert_no_active_predecessors_helper() {
    let (executor, db, temp_dir) = build_test_executor("helper").await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-h"),
            name: "helper-test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();

    features
        .add(Feature {
            effort: None,
            id: FeatureId::from("f-h"),
            project_id: ProjectId::from("p-h"),
            workflow_id: Some(WorkflowId::from("w-h")),
            workflow_version_id: None,
            title: "helper".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            created_at: now,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .unwrap();

    // 5 steps: targets index 4. Preds at index 0 (done), 1 (done),
    // 2 (running — the EARLIEST non-terminal — must be reported),
    // 3 (awaiting_gate — also non-terminal but later in the scan).
    for (idx, status) in [
        (0u32, "completed"),
        (1, "failed"),
        (2, "running"),
        (3, "awaiting_gate"),
        (4, "failed"),
    ] {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(format!("se-h-{idx}")),
                feature_id: FeatureId::from("f-h"),
                step_id: StepId::from(format!("step-{idx}")),
                step_index: idx,
                step_kind: "agent".to_string(),
                status: status.to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    let target = features
        .step_get(&StepExecutionId::from("se-h-4".to_string()))
        .unwrap()
        .unwrap();

    let err = executor
        .assert_no_active_predecessors(&target, "retrying this step")
        .expect_err("must report the earliest non-terminal predecessor");
    match err {
        AppError::Validation { message } => {
            // step-2 has the lowest step_index among non-terminal
            // predecessors, so it must be the one named.
            assert!(
                message.contains("step-2"),
                "expected step-2 to be named, got: {message}"
            );
            assert!(
                !message.contains("step-3"),
                "later non-terminal pred must not be picked, got: {message}"
            );
            assert!(
                message.contains("retrying this step"),
                "intent phrase must be threaded through, got: {message}"
            );
        }
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}
