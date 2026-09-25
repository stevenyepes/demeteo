//! `step_set_assignment` against a real repository: which tier it writes, and
//! which rows it refuses.
//!
//! The upsert itself is covered without ports in `tests/domain/step_assignment.rs`
//! and the wording of each refusal in `tests/domain/run_control.rs`. What only a
//! wired executor can show is that the adapter reaches those decisions at all,
//! and that the write lands on `features.step_overrides_json` and nowhere else —
//! the feature-wide columns next to it are the tier this control must never
//! touch, and writing them would still look like a working pin from the UI.

use std::sync::Arc;

use super::harness::{build_test_executor_with_agents, scratch_dir, FakeNotif};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::adapters::step_executor::DagStepExecutor;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId, WorkflowId};
use crate::domain::models::{EffortLevel, Feature, StepExecution, StepOverride};
use crate::domain::run_control::{shadow_refusal, RunAction};
use crate::domain::step_assignment::StepAssignment;
use crate::domain::step_seed::MANUAL_SYNC_STEP_ID;
use crate::error::AppError;
use crate::paths;
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::remote_run_mirror::RemoteRunMirrorPort;
use crate::ports::step_executor::StepExecutor;

const PROJECT: &str = "p-assign";
const FEATURE: &str = "f-assign";

/// An executor whose registry holds the two harnesses these tests pin. The
/// shared builder's registry is empty, which the harness guard would refuse
/// before any of the guards under test were reached.
async fn build_test_executor(
    label: &str,
) -> (DagStepExecutor, Arc<SqliteAdapter>, std::path::PathBuf) {
    let temp_dir = scratch_dir(label);
    let (executor, db) = build_test_executor_with_agents(
        temp_dir.clone(),
        Arc::new(FakeNotif),
        Arc::new(ScriptedExec::new(&[])),
        vec![
            Arc::new(crate::adapters::agent::opencode::runtime()),
            Arc::new(crate::adapters::agent::claude_code::runtime()),
        ],
    )
    .await;
    (executor, db, temp_dir)
}

/// Seed a project, a feature carrying `overrides`, and one `agent` step per
/// `(execution_id, step_id, status)` triple.
fn seed(
    db: &crate::adapters::database::SqliteAdapter,
    overrides: Vec<StepOverride>,
    steps: &[(&str, &str, &str)],
) {
    seed_kind(db, overrides, steps, "agent");
}

fn seed_kind(
    db: &crate::adapters::database::SqliteAdapter,
    overrides: Vec<StepOverride>,
    steps: &[(&str, &str, &str)],
    step_kind: &str,
) {
    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = db;
    let features: &dyn FeatureRepository = db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from(PROJECT),
            name: "assignment-test".to_string(),
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
            id: FeatureId::from(FEATURE),
            project_id: ProjectId::from(PROJECT),
            workflow_id: Some(WorkflowId::from("w-assign")),
            workflow_version_id: None,
            title: "assignment feature".to_string(),
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
            step_overrides: overrides,
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .unwrap();

    for (idx, (se_id, step_id, status)) in steps.iter().enumerate() {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(se_id.to_string()),
                feature_id: FeatureId::from(FEATURE),
                step_id: StepId::from(step_id.to_string()),
                step_index: idx as u32,
                step_kind: step_kind.to_string(),
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
}

fn reload(db: &crate::adapters::database::SqliteAdapter) -> Feature {
    let features: &dyn FeatureRepository = db;
    features
        .get(&FeatureId::from(FEATURE))
        .unwrap()
        .expect("the seeded feature row is gone")
}

#[tokio::test]
async fn pinning_a_queued_step_writes_tier_one_and_leaves_the_run_where_it_was() {
    let (executor, db, temp_dir) = build_test_executor("assign_pending").await;
    seed(
        &db,
        vec![StepOverride {
            step_id: "s-review".to_string(),
            agent_kind: Some("opencode".to_string()),
            model: None,
            effort: None,
        }],
        &[
            ("se-assign-0", "s-implement", "pending"),
            ("se-assign-1", "s-review", "pending"),
        ],
    );

    executor
        .step_set_assignment(
            "se-assign-0",
            StepAssignment {
                agent_kind: Some("claude-code".to_string()),
                model: Some("claude-opus-5".to_string()),
                effort: Some(EffortLevel::High),
            },
        )
        .await
        .expect("a pending step is assignable");

    let feature = reload(&db);
    assert_eq!(
        feature.step_overrides,
        vec![
            StepOverride {
                step_id: "s-review".to_string(),
                agent_kind: Some("opencode".to_string()),
                model: None,
                effort: None,
            },
            StepOverride {
                step_id: "s-implement".to_string(),
                agent_kind: Some("claude-code".to_string()),
                model: Some("claude-opus-5".to_string()),
                effort: Some(EffortLevel::High),
            },
        ],
        "the pin is appended to the stored list, not composed over it"
    );

    // Tier 2 is the bug this control replaces: a feature-wide write would
    // re-point every other node too, and read back from the UI as a success.
    assert_eq!(feature.agent_kind, None, "feature-wide harness was written");
    assert_eq!(feature.model, None, "feature-wide model was written");
    assert_eq!(feature.effort, None, "feature-wide effort was written");

    let features: &dyn FeatureRepository = &*db;
    let steps = features
        .steps_for_feature(&FeatureId::from(FEATURE))
        .unwrap();
    assert!(
        steps.iter().all(|s| s.status == "pending"),
        "assigning rewound or re-armed something: {:?}",
        steps.iter().map(|s| &s.status).collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn an_in_flight_step_is_refused_and_nothing_is_stored() {
    let (executor, db, temp_dir) = build_test_executor("assign_running").await;
    seed(
        &db,
        Vec::new(),
        &[("se-assign-0", "s-implement", "running")],
    );

    let err = executor
        .step_set_assignment(
            "se-assign-0",
            StepAssignment {
                agent_kind: Some("claude-code".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("a running step was reassigned");
    match err {
        AppError::Validation { message } => assert!(
            message.contains("running"),
            "the refusal must name the status that caused it: {message}"
        ),
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    assert!(
        reload(&db).step_overrides.is_empty(),
        "a refused assignment still wrote the row"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The row is seeded `running` so the two refusals it could draw are distinct:
/// a build that consulted the status first would word this as the in-flight
/// refusal, and the user would be told to wait for a step no scheduler will
/// ever dispatch.
#[tokio::test]
async fn an_out_of_band_sync_row_is_refused_before_its_status_is_consulted() {
    let (executor, db, temp_dir) = build_test_executor("assign_out_of_band").await;
    seed(
        &db,
        Vec::new(),
        &[("se-assign-0", MANUAL_SYNC_STEP_ID, "running")],
    );

    let err = executor
        .step_set_assignment("se-assign-0", StepAssignment::default())
        .await
        .expect_err("the out-of-band sync row was assignable");
    match err {
        AppError::Validation { message } => assert!(
            message.contains("nothing to assign"),
            "the out-of-band refusal must say what was refused: {message}"
        ),
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The third guard, and the only one whose failure mode is silent: a shadow
/// row accepts the write, `hydrate_shadow_feature` overwrites it from the
/// runner's copy on the next reconcile, and the pin the user watched land is
/// gone with no error anywhere. The step is `pending` so nothing but the
/// shadow check can produce the refusal, and the feature starts with a pin so
/// "nothing was written" is measured against a value rather than a default.
#[tokio::test]
async fn a_runner_owned_shadow_is_refused_and_its_pinned_list_is_untouched() {
    let (executor, db, temp_dir) = build_test_executor("assign_shadow").await;
    let already_pinned = vec![StepOverride {
        step_id: "s-review".to_string(),
        agent_kind: Some("opencode".to_string()),
        model: None,
        effort: None,
    }];
    seed(
        &db,
        already_pinned.clone(),
        &[("se-assign-0", "s-implement", "pending")],
    );

    let mirror: &dyn RemoteRunMirrorPort = &*db;
    mirror
        .upsert_submitted(
            "m1",
            "r1",
            Some(PROJECT),
            Some(FEATURE),
            FEATURE,
            paths::now_ms(),
        )
        .unwrap();

    let err = executor
        .step_set_assignment(
            "se-assign-0",
            StepAssignment {
                agent_kind: Some("claude-code".to_string()),
                model: Some("claude-opus-5".to_string()),
                effort: Some(EffortLevel::High),
            },
        )
        .await
        .expect_err("a shadow of a runner-owned run was assignable from the laptop");
    match err {
        AppError::Validation { message } => assert_eq!(
            message,
            shadow_refusal(RunAction::Assign, FEATURE),
            "the shadow refusal must be the one the domain words, routing the user to the runner"
        ),
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    assert_eq!(
        reload(&db).step_overrides,
        already_pinned,
        "the laptop wrote a pin the next hydrate_shadow_feature will discard"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// A gate reads no agent, so a pin on one is stored, drawn on the node as
/// planned, and never consulted — a success report for a change nothing
/// honours. Seeded `pending` so the kind is the only thing that can refuse.
#[tokio::test]
async fn a_gate_step_is_refused_and_nothing_is_stored() {
    let (executor, db, temp_dir) = build_test_executor("assign_gate").await;
    seed_kind(
        &db,
        Vec::new(),
        &[("se-assign-0", "s-approve", "pending")],
        "gate",
    );

    let err = executor
        .step_set_assignment(
            "se-assign-0",
            StepAssignment {
                agent_kind: Some("claude-code".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("a gate step was assignable");
    match err {
        AppError::Validation { message } => assert!(
            message.contains("'gate'"),
            "the refusal must name the kind: {message}"
        ),
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }
    assert!(reload(&db).step_overrides.is_empty());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn an_unregistered_harness_is_refused_and_nothing_is_stored() {
    let (executor, db, temp_dir) = build_test_executor("assign_unknown_harness").await;
    seed(
        &db,
        Vec::new(),
        &[("se-assign-0", "s-implement", "pending")],
    );

    let err = executor
        .step_set_assignment(
            "se-assign-0",
            StepAssignment {
                agent_kind: Some("claude_code".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect_err("an unregistered harness was pinned");
    match err {
        AppError::Validation { message } => assert!(
            message.contains("Unknown harness 'claude_code'"),
            "{message}"
        ),
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }
    assert!(reload(&db).step_overrides.is_empty());

    let _ = std::fs::remove_dir_all(temp_dir);
}
