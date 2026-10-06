use super::{
    backfill_local_path, declared_remote_paths, hydrate_shadow_feature, mime_for_path,
    sequence_state_needs_fetch, shadow_feature_patch, shadow_feature_status, shadow_mr_state,
    shadow_step_artifacts_stale,
};
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaseline, HarnessBaselineRun};
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId};
use crate::domain::models::feature::{Feature, StepExecution};
use crate::domain::models::{
    EffortLevel, Platform, Project, SequenceCheckpoint, SequenceStateMirror, StepOverride,
    SubtaskRunMirrorRow,
};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};
use crate::state::AppContext;
use async_trait::async_trait;
use std::sync::Arc;

fn make_step(status: &str, tokens: i64, wall_clock_secs: u64, cost_usd: f64) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-1".to_string()),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: StepId::from("s-critic".to_string()),
        step_index: 5,
        step_kind: "agent".to_string(),
        status: status.to_string(),
        cost_usd: Some(cost_usd),
        tokens: Some(tokens),
        wall_clock_secs: Some(wall_clock_secs),
        artifact_path: Some("artifacts/critic-review.md".to_string()),
        artifact_paths: vec!["artifacts/critic-review.md".to_string()],
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn shadow_stale_false_when_no_existing_shadow_yet() {
    let fresh = make_step("completed", 86143, 147, 0.42);
    assert!(!shadow_step_artifacts_stale(None, &fresh));
}

#[test]
fn shadow_stale_false_when_nothing_changed() {
    let existing = make_step("completed", 86143, 147, 0.42);
    let fresh = make_step("completed", 86143, 147, 0.42);
    assert!(!shadow_step_artifacts_stale(Some(&existing), &fresh));
}

#[test]
fn shadow_stale_true_when_tokens_changed_but_status_and_count_would_look_identical() {
    let existing = make_step("completed", 86143, 147, 0.42);
    let fresh = make_step("completed", 137678, 95, 0.61);
    assert!(shadow_step_artifacts_stale(Some(&existing), &fresh));
}

#[test]
fn shadow_stale_true_when_status_differs() {
    let existing = make_step("completed", 100, 10, 0.1);
    let fresh = make_step("running", 100, 10, 0.1);
    assert!(shadow_step_artifacts_stale(Some(&existing), &fresh));
}

#[test]
fn declared_paths_single_first_and_deduped() {
    let output = declared_remote_paths(
        Some("/w/report.md"),
        &["/w/report.md".to_string(), "/w/diff.patch".to_string()],
    );
    assert_eq!(output, vec!["/w/report.md", "/w/diff.patch"]);
}

#[test]
fn declared_paths_none_single_uses_list_only() {
    let output = declared_remote_paths(None, &["/w/a.txt".to_string(), "/w/b.txt".to_string()]);
    assert_eq!(output, vec!["/w/a.txt", "/w/b.txt"]);
}

#[test]
fn declared_paths_empty_when_nothing_declared() {
    assert!(declared_remote_paths(None, &[]).is_empty());
}

#[test]
fn backfill_finds_matching_local_file_by_stem() {
    let existing = vec!["/data/artifacts/f-1/se-1/critic-review.md".to_string()];
    let output = backfill_local_path(&existing, "/workspace/critic-review.md");
    assert_eq!(output, Some(existing[0].clone()));
}

#[test]
fn backfill_none_when_no_matching_local_file() {
    let existing = vec!["/data/artifacts/f-1/se-1/other-report.md".to_string()];
    assert_eq!(
        backfill_local_path(&existing, "/workspace/critic-review.md"),
        None
    );
}

#[test]
fn mime_inferred_from_extension() {
    assert_eq!(mime_for_path("/w/report.md"), "text/markdown");
    assert_eq!(mime_for_path("/w/change.diff"), "text/x-diff");
    assert_eq!(mime_for_path("/w/manifest.json"), "application/json");
    assert_eq!(mime_for_path("/w/LICENSE"), "text/plain");
}

// ── Shadow-feature replication (V37, decision 44) ────────────────────────────

fn runner_feature(harness_baseline: Option<HarnessBaseline>) -> Feature {
    Feature {
        id: FeatureId::from("f_shadow".to_string()),
        project_id: ProjectId::from("p_shadow".to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: "remote run".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 1.5,
        duration: "3m".to_string(),
        tokens: 42,
        created_at: 1_000,
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
        harness_baseline,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: Some("demeteo/features/f_shadow".to_string()),
    }
}

#[test]
fn the_shadow_patch_mirrors_the_measured_baseline() {
    // The update branch is the one that matters: the first poll inserts the
    // whole `Feature`, so a baseline missing from this patch is stale only
    // *after* the run measures it — silently, and only on a detached run.
    let baseline = HarnessBaseline {
        base_sha: "abc123".to_string(),
        harnesses: vec![HarnessBaselineRun {
            name: "unit".to_string(),
            command: "cargo test".to_string(),
            exit_ok: false,
            fingerprint: "fp".to_string(),
            output_ref: Some("/artifacts/unit.log".to_string()),
            environment: None,
            failing_tests: None,
            measured_at: 1_700,
            producer: BaselineProducer::Node,
        }],
    };
    let patch = shadow_feature_patch(&runner_feature(Some(baseline.clone())), None);
    assert_eq!(patch.harness_baseline, Some(Some(baseline)));
}

#[test]
fn the_shadow_patch_mirrors_the_branch_the_runner_cut_but_not_the_launch_inputs() {
    let patch = shadow_feature_patch(&runner_feature(None), None);
    assert_eq!(
        patch.resolved_branch,
        Some(Some("demeteo/features/f_shadow".to_string())),
        "only the runner knows what it named the branch"
    );
    assert!(patch.origin.is_none());
    assert!(patch.diff_base_branch.is_none());
}

#[test]
fn the_shadow_patch_mirrors_an_unmeasured_baseline_as_absent() {
    let patch = shadow_feature_patch(&runner_feature(None), None);
    assert_eq!(
        patch.harness_baseline,
        Some(None),
        "an unmeasured run must clear the shadow, not leave a stale record"
    );
}

#[test]
fn the_shadow_patch_mirrors_the_assignment_pins_the_runner_holds() {
    // Pins are editable mid-run on the runner, so the launch-input reasoning
    // above is inverted here: following is what keeps the desktop honest.
    let mut feature = runner_feature(None);
    feature.step_overrides = vec![StepOverride {
        step_id: "s-implement".to_string(),
        agent_kind: Some("claude-code".to_string()),
        model: Some("opus".to_string()),
        effort: Some(EffortLevel::High),
    }];
    let patch = shadow_feature_patch(&feature, None);
    assert_eq!(patch.step_overrides, Some(feature.step_overrides));
}

// ── A landed PR never goes back in flight ────────────────────────────────────

#[test]
fn a_local_merge_survives_a_runner_that_has_not_seen_it_yet() {
    // The desktop `MrMonitor` writes `merged` onto the shadow; the runner's
    // copy may still read `open` for a poll or two.
    assert_eq!(
        shadow_mr_state(Some("merged"), Some("open")),
        Some("merged".to_string())
    );
    assert_eq!(
        shadow_mr_state(Some("merged"), None),
        Some("merged".to_string())
    );
}

#[test]
fn a_runner_merge_lands_on_the_shadow() {
    assert_eq!(
        shadow_mr_state(Some("open"), Some("merged")),
        Some("merged".to_string())
    );
}

#[test]
fn a_reopened_pr_moves_a_closed_shadow_back_to_open() {
    assert_eq!(
        shadow_mr_state(Some("closed"), Some("open")),
        Some("open".to_string())
    );
}

#[test]
fn the_shadow_patch_keeps_a_local_merge() {
    let mut feature = runner_feature(None);
    feature.mr_state = Some("open".to_string());
    assert_eq!(
        shadow_feature_patch(&feature, Some("merged")).mr_state,
        Some(Some("merged".to_string()))
    );
    assert_eq!(
        shadow_feature_patch(&feature, Some("closed")).mr_state,
        Some(Some("open".to_string()))
    );
}

// ── Sequence-state mirror (task list not shown for detached runs) ────────────

/// Runner-RPC stub for the `get_sequence_state` mirror tests: answers
/// `get_feature`/`list_steps` with fixed rows and `get_sequence_state` with
/// whatever the test configures — a real payload, or an `unknown method` `Err`
/// modelling an older runner that predates this RPC.
struct SequenceRpcStub {
    feature: serde_json::Value,
    steps: serde_json::Value,
    sequence_state: std::sync::Mutex<Result<serde_json::Value, String>>,
    /// Every `if_revision` the laptop sent, in call order.
    if_revisions: std::sync::Mutex<Vec<Option<String>>>,
}

impl SequenceRpcStub {
    fn new(
        feature: serde_json::Value,
        steps: serde_json::Value,
        sequence_state: Result<serde_json::Value, String>,
    ) -> Self {
        Self {
            feature,
            steps,
            sequence_state: std::sync::Mutex::new(sequence_state),
            if_revisions: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ExecutionPort for SequenceRpcStub {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        unimplemented!()
    }
    async fn read_file(&self, _machine_id: &str, _path: &str) -> Result<String, String> {
        unimplemented!()
    }
    async fn write_file(
        &self,
        _machine_id: &str,
        _path: &str,
        _content: &str,
    ) -> Result<(), String> {
        unimplemented!()
    }
    async fn write_file_bytes(
        &self,
        _machine_id: &str,
        _path: &str,
        _content: &[u8],
    ) -> Result<(), String> {
        unimplemented!()
    }
    async fn get_metadata(&self, _machine_id: &str, _path: &str) -> Result<SftpEntry, String> {
        unimplemented!()
    }
    async fn list_dir(&self, _machine_id: &str, _path: &str) -> Result<Vec<SftpEntry>, String> {
        unimplemented!()
    }
    async fn setup_worktree(
        &self,
        _machine_id: &str,
        _repo_path: &str,
        _branch: &str,
        _sandbox_path: &str,
    ) -> Result<(), String> {
        unimplemented!()
    }
    async fn resolve_home(&self, _machine_id: &str) -> Result<String, String> {
        unimplemented!()
    }
    async fn resolve_platform(&self, _machine_id: &str) -> Result<Platform, String> {
        unimplemented!()
    }
    async fn resolve_user(&self, _machine_id: &str) -> Result<String, String> {
        unimplemented!()
    }
    async fn control_rpc(
        &self,
        _machine_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        match method {
            "get_feature" => Ok(self.feature.clone()),
            "list_steps" => Ok(self.steps.clone()),
            "get_sequence_state" => {
                self.if_revisions.lock().unwrap().push(
                    params
                        .get("if_revision")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                );
                self.sequence_state.lock().unwrap().clone()
            }
            other => Err(format!("unexpected runner RPC {other}")),
        }
    }
    fn spawn_interactive(
        &self,
        _machine_id: &str,
        _binary: &str,
        _args: &[String],
        _cwd: &str,
        _env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        unimplemented!()
    }
}

fn sequence_test_feature() -> Feature {
    Feature {
        id: FeatureId::from("f-1".to_string()),
        project_id: ProjectId::from("p-1".to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: "detached feature".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: 0,
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
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    }
}

fn sequence_test_step() -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-1".to_string()),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: StepId::from("s-implement".to_string()),
        step_index: 0,
        step_kind: "sequence".to_string(),
        status: "running".to_string(),
        cost_usd: Some(0.0),
        tokens: Some(0),
        wall_clock_secs: Some(0),
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// Builds an `AppContext` backed by a real (tempdir) `SqliteAdapter), with
/// `ctx.exec` swapped for `SequenceRpcStub`, and seeds the one project the
/// hydrate call needs. Returns the tempdir guard too, which removes it on drop.
fn make_sequence_test_ctx(
    label: &str,
    sequence_state: Result<serde_json::Value, String>,
) -> (AppContext, crate::support::test_dir::TestDir) {
    let temp_dir = crate::support::test_dir::TestDir::new(&format!(
        "demeteo_test_hydrate_sequence_state_{label}"
    ));
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: temp_dir.path().to_path_buf(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    ctx.projects
        .add(Project {
            id: ProjectId::from("p-1".to_string()),
            name: "detached project".to_string(),
            compute_type: "remote".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .unwrap();
    ctx.exec = Arc::new(SequenceRpcStub::new(
        serde_json::to_value(sequence_test_feature()).unwrap(),
        serde_json::to_value(vec![sequence_test_step()]).unwrap(),
        sequence_state,
    ));
    (ctx, temp_dir)
}

/// The reproduction case, exercised end to end: a detached run's poll must
/// mirror `sequence_plan_cache`, `sequence_checkpoints`, and `subtask_runs`
/// locally, not just `features`/`step_executions` — closing the gap the root
/// cause names (no RPC, no write path existed for these three tables).
#[tokio::test]
async fn hydrate_shadow_feature_mirrors_sequence_state_for_a_sequence_step() {
    let feature_id = FeatureId::from("f-1".to_string());
    let step_execution_id = StepExecutionId::from("se-1".to_string());
    let node_id = "s-implement";

    let subtask_row = SubtaskRunMirrorRow {
        id: "sr-1".to_string(),
        subtask_id: "t1".to_string(),
        agent_id: Some("claude-code".to_string()),
        worktree_path: "/work/f-1".to_string(),
        branch: "feature/f-1".to_string(),
        status: "completed".to_string(),
        cost_usd: 0.42,
        tokens: 1200,
        error_message: None,
        started_at: 0,
        ended_at: Some(30_000),
        plan_epoch: None,
        plan_cycle: None,
    };
    let sequence_state = SequenceStateMirror {
        plan_json: Some(
            r#"{"tasks":[{"id":"t1","title":"Reproduce the bug"}],"cycle":0}"#.to_string(),
        ),
        checkpoint: SequenceCheckpoint {
            landed_task_ids: vec!["t1".to_string()],
            anchor_sha: Some("abc123".to_string()),
            produced: None,
        },
        subtask_runs: vec![subtask_row],
    };
    let (ctx, _temp_dir) = make_sequence_test_ctx(
        "mirrors",
        Ok(serde_json::to_value(&sequence_state).unwrap()),
    );

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "running")
        .await
        .unwrap();

    let plan = ctx
        .sequence_resume
        .plan_cache_get(&feature_id, node_id)
        .unwrap();
    assert_eq!(
        plan, sequence_state.plan_json,
        "the runner's plan cache must land in the laptop's sequence_plan_cache"
    );

    let checkpoint = ctx
        .sequence_resume
        .sequence_checkpoint_get(&feature_id, node_id)
        .unwrap();
    assert_eq!(checkpoint.landed_task_ids, vec!["t1".to_string()]);
    assert_eq!(checkpoint.anchor_sha, Some("abc123".to_string()));

    let runs = ctx
        .features
        .subtask_runs_for_step(&step_execution_id)
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].subtask_id, "t1");
    assert_eq!(runs[0].status, "completed");
    assert_eq!(runs[0].cost_usd, 0.42);
}

/// Version skew: an older deployed runner has no `get_sequence_state` handler
/// at all, so the RPC comes back `Err("unknown method: get_sequence_state")`
/// (`demeteo-runner`'s dispatch table default arm). The whole reconcile poll
/// must not fail on that, and the local sequence tables must stay untouched
/// — `sequence_state` degrades to unplanned, exactly as before this RPC
/// existed, rather than the poll erroring or writing a half state.
#[tokio::test]
async fn hydrate_shadow_feature_leaves_sequence_tables_untouched_without_get_sequence_state() {
    let feature_id = FeatureId::from("f-1".to_string());
    let step_execution_id = StepExecutionId::from("se-1".to_string());
    let node_id = "s-implement";

    let (ctx, _temp_dir) = make_sequence_test_ctx(
        "method_not_found",
        Err("unknown method: get_sequence_state".to_string()),
    );

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "running")
        .await
        .expect("an older runner without get_sequence_state must not fail the whole poll");

    assert!(
        ctx.sequence_resume
            .plan_cache_get(&feature_id, node_id)
            .unwrap()
            .is_none(),
        "the plan cache must stay untouched when the RPC is unavailable"
    );
    assert!(
        ctx.sequence_resume
            .sequence_checkpoint_get(&feature_id, node_id)
            .unwrap()
            .is_empty(),
        "the checkpoint must stay untouched when the RPC is unavailable"
    );
    assert!(
        ctx.features
            .subtask_runs_for_step(&step_execution_id)
            .unwrap()
            .is_empty(),
        "subtask_runs must stay untouched when the RPC is unavailable"
    );
    assert!(
        ctx.features.step_get(&step_execution_id).unwrap().is_some(),
        "the step shadow itself must still hydrate normally"
    );
}

fn subtask_row(id: &str, subtask_id: &str, status: &str, started_at: i64) -> SubtaskRunMirrorRow {
    SubtaskRunMirrorRow {
        id: id.to_string(),
        subtask_id: subtask_id.to_string(),
        agent_id: Some("claude-code".to_string()),
        worktree_path: "/work/f-1".to_string(),
        branch: "feature/f-1".to_string(),
        status: status.to_string(),
        cost_usd: 0.1,
        tokens: 100,
        error_message: None,
        started_at,
        ended_at: (status != "running").then_some(started_at + 10),
        plan_epoch: Some("e-1".to_string()),
        plan_cycle: Some(2),
    }
}

fn two_ticket_state(rows: Vec<SubtaskRunMirrorRow>) -> SequenceStateMirror {
    SequenceStateMirror {
        plan_json: Some(
            r#"{"tasks":[{"id":"t1","title":"one"},{"id":"t2","title":"two"}]}"#.to_string(),
        ),
        checkpoint: SequenceCheckpoint {
            landed_task_ids: rows
                .iter()
                .filter(|r| r.status == "completed")
                .map(|r| r.subtask_id.clone())
                .collect(),
            anchor_sha: Some("abc123".to_string()),
            produced: None,
        },
        subtask_runs: rows,
    }
}

/// The stale-list reproduction: the runner advances through tickets while
/// the step row's status and counters stay exactly as they were (it rolls
/// them up only when the attempt ends), so nothing at step level says the
/// task list moved. The second poll must still pick up the new rows.
#[tokio::test]
async fn a_running_sequence_mirrors_tickets_that_advance_under_unchanged_step_counters() {
    let step_execution_id = StepExecutionId::from("se-1".to_string());
    let interrupted = two_ticket_state(vec![subtask_row("sr-a", "t1", "interrupted", 1_000)]);
    let (ctx, _temp_dir) =
        make_sequence_test_ctx("advance", Ok(serde_json::to_value(&interrupted).unwrap()));
    let stub = Arc::new(SequenceRpcStub::new(
        serde_json::to_value(sequence_test_feature()).unwrap(),
        serde_json::to_value(vec![sequence_test_step()]).unwrap(),
        Ok(serde_json::to_value(&interrupted).unwrap()),
    ));
    let mut ctx = ctx;
    ctx.exec = stub.clone();

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "running")
        .await
        .unwrap();

    let advanced = two_ticket_state(vec![
        subtask_row("sr-a", "t1", "interrupted", 1_000),
        subtask_row("sr-b", "t1", "completed", 2_000),
        subtask_row("sr-c", "t2", "running", 3_000),
    ]);
    *stub.sequence_state.lock().unwrap() = Ok(serde_json::to_value(&advanced).unwrap());

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "running")
        .await
        .unwrap();

    let statuses: Vec<(String, String)> = ctx
        .features
        .subtask_runs_mirror_for_step(&step_execution_id)
        .unwrap()
        .into_iter()
        .map(|r| (r.subtask_id, r.status))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("t1".to_string(), "interrupted".to_string()),
            ("t1".to_string(), "completed".to_string()),
            ("t2".to_string(), "running".to_string()),
        ]
    );
}

/// The bandwidth half: what the laptop reads back out of its own mirror
/// must hash to exactly what the runner sent, or `if_revision` never
/// matches and every poll resends the whole plan. Rows sharing a
/// `started_at` are included because their order is the easy thing to
/// lose on a round trip.
#[tokio::test]
async fn a_mirrored_sequence_state_round_trips_to_the_runners_revision() {
    let state = two_ticket_state(vec![
        subtask_row("sr-b", "t2", "completed", 1_000),
        subtask_row("sr-a", "t1", "completed", 1_000),
    ]);
    let (mut ctx, _temp_dir) =
        make_sequence_test_ctx("round_trip", Ok(serde_json::to_value(&state).unwrap()));
    let stub = Arc::new(SequenceRpcStub::new(
        serde_json::to_value(sequence_test_feature()).unwrap(),
        serde_json::to_value(vec![sequence_test_step()]).unwrap(),
        Ok(serde_json::to_value(&state).unwrap()),
    ));
    ctx.exec = stub.clone();
    let feature_id = FeatureId::from("f-1".to_string());

    // The runner answers from its own table, which orders ties the same way.
    let runner_side = {
        let mut sorted = state.clone();
        sorted
            .subtask_runs
            .sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
        sorted
    };

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "running")
        .await
        .unwrap();
    let local = super::super::read_sequence_state_mirror(
        &*ctx.features,
        &*ctx.sequence_resume,
        &feature_id,
        "s-implement",
    )
    .unwrap();
    assert_eq!(local.revision(), runner_side.revision());

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "running")
        .await
        .unwrap();
    assert_eq!(
        stub.if_revisions.lock().unwrap().last().cloned().flatten(),
        Some(runner_side.revision()),
        "the second poll must offer the mirrored revision so the runner can skip the payload"
    );
}

#[tokio::test]
async fn an_unchanged_reply_leaves_the_mirror_as_it_was() {
    let step_execution_id = StepExecutionId::from("se-1".to_string());
    let state = two_ticket_state(vec![subtask_row("sr-a", "t1", "running", 1_000)]);
    let (mut ctx, _temp_dir) =
        make_sequence_test_ctx("unchanged", Ok(serde_json::to_value(&state).unwrap()));
    let stub = Arc::new(SequenceRpcStub::new(
        serde_json::to_value(sequence_test_feature()).unwrap(),
        serde_json::to_value(vec![sequence_test_step()]).unwrap(),
        Ok(serde_json::to_value(&state).unwrap()),
    ));
    ctx.exec = stub.clone();
    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "running")
        .await
        .unwrap();

    *stub.sequence_state.lock().unwrap() = Ok(serde_json::json!({ "unchanged": true }));
    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "running")
        .await
        .unwrap();

    let runs = ctx
        .features
        .subtask_runs_for_step(&step_execution_id)
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "running");
}

#[test]
fn only_a_running_sequence_is_asked_on_every_poll() {
    assert!(sequence_state_needs_fetch(false, false, "running"));
    for settled in [
        "completed",
        "failed",
        "interrupted",
        "pending",
        "awaiting_gate",
    ] {
        assert!(
            !sequence_state_needs_fetch(false, false, settled),
            "{settled}"
        );
        assert!(
            sequence_state_needs_fetch(false, true, settled),
            "{settled}"
        );
        assert!(
            sequence_state_needs_fetch(true, false, settled),
            "{settled}"
        );
    }
}

// ── Run-level blocks on the shadow's status ──────────────────────────────────

#[test]
fn a_credential_park_overrides_a_runner_feature_that_already_finished() {
    // The terminal push park: the pipeline is done on the runner, the PR is
    // not — so the shadow must band as "needs you", not "done".
    assert_eq!(
        shadow_feature_status("completed", "needs-credentials"),
        "needs-credentials"
    );
    assert_eq!(
        shadow_feature_status("pending", "needs-credentials"),
        "needs-credentials"
    );
}

#[test]
fn a_parked_gate_reads_as_awaiting_gate_whatever_the_feature_column_says() {
    assert_eq!(shadow_feature_status("running", "parked"), "awaiting_gate");
    assert_eq!(
        shadow_feature_status("awaiting_gate", "parked"),
        "awaiting_gate"
    );
    assert_eq!(shadow_feature_status("gated", "parked"), "gated");
}

#[test]
fn an_unblocked_run_mirrors_the_runner_feature_verbatim() {
    for run_status in [
        "running",
        "awaiting_mr",
        "completed",
        "failed",
        "unreachable",
    ] {
        assert_eq!(shadow_feature_status("completed", run_status), "completed");
    }
}

#[tokio::test]
async fn hydrate_shows_a_credential_park_until_the_run_moves_on() {
    let feature_id = FeatureId::from("f-1".to_string());
    let (mut ctx, _temp_dir) =
        make_sequence_test_ctx("credential_park", Ok(serde_json::Value::Null));
    let mut finished = sequence_test_feature();
    finished.status = "completed".to_string();
    ctx.exec = Arc::new(SequenceRpcStub::new(
        serde_json::to_value(finished).unwrap(),
        serde_json::json!([]),
        Ok(serde_json::Value::Null),
    ));

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "needs-credentials")
        .await
        .unwrap();
    assert_eq!(
        ctx.features.get(&feature_id).unwrap().unwrap().status,
        "needs-credentials",
        "the first poll inserts the shadow — it must already carry the block"
    );

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "awaiting_mr")
        .await
        .unwrap();
    assert_eq!(
        ctx.features.get(&feature_id).unwrap().unwrap().status,
        "completed",
        "once credentials land, the update branch must follow the runner again"
    );
}

#[tokio::test]
async fn a_pre_clone_credential_park_marks_the_desktop_row_it_cannot_hydrate() {
    let feature_id = FeatureId::from("f-1".to_string());
    let (mut ctx, _temp_dir) =
        make_sequence_test_ctx("pre_clone_park", Ok(serde_json::Value::Null));
    let mut submitted = sequence_test_feature();
    submitted.status = "pending".to_string();
    ctx.features.add(submitted).unwrap();
    ctx.exec = Arc::new(SequenceRpcStub::new(
        serde_json::Value::Null,
        serde_json::json!([]),
        Ok(serde_json::Value::Null),
    ));

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "needs-credentials")
        .await
        .unwrap();
    assert_eq!(
        ctx.features.get(&feature_id).unwrap().unwrap().status,
        "needs-credentials"
    );
}

#[tokio::test]
async fn hydrate_keeps_a_shadow_the_desktop_saw_merge() {
    let feature_id = FeatureId::from("f-1".to_string());
    let (mut ctx, _temp_dir) = make_sequence_test_ctx("local_merge", Ok(serde_json::Value::Null));
    let mut landed = sequence_test_feature();
    landed.status = "completed".to_string();
    landed.mr_state = Some("merged".to_string());
    ctx.features.add(landed).unwrap();
    let mut lagging = sequence_test_feature();
    lagging.status = "completed".to_string();
    lagging.mr_state = Some("open".to_string());
    ctx.exec = Arc::new(SequenceRpcStub::new(
        serde_json::to_value(lagging).unwrap(),
        serde_json::json!([]),
        Ok(serde_json::Value::Null),
    ));

    hydrate_shadow_feature(&ctx, "m-1", "r-1", "p-1", "f-1", "awaiting_mr")
        .await
        .unwrap();
    assert_eq!(
        ctx.features.get(&feature_id).unwrap().unwrap().mr_state,
        Some("merged".to_string()),
        "a runner that has not seen the merge must not put a landed ticket back in flight"
    );
}
