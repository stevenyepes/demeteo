// Tests extracted from `src/application/run_view.rs` (mirrored-tests convention).
// `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId};
use crate::domain::models::feature::{Feature, StepExecution};
use crate::domain::models::{Platform, Project, SubtaskRunMirrorRow};
use crate::ports::db::{FeatureRepository, ProjectRepository, SequenceResumeRepository};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};
use async_trait::async_trait;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Arc;

/// `RunView::sequence_state` never reads `exec` — every method panics so an
/// accidental new dependency on it would fail this test loudly rather than
/// silently no-op.
struct UnusedExec;

#[async_trait]
impl ExecutionPort for UnusedExec {
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
        _method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        unimplemented!()
    }
    fn spawn_interactive(
        &self,
        _machine_id: &str,
        _binary: &str,
        _args: &[String],
        _cwd: &str,
        _env: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        unimplemented!()
    }
}

fn make_view() -> (RunView, Arc<SqliteAdapter>) {
    let adapter = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    let view = RunView::new(
        adapter.clone(),
        adapter.clone(),
        adapter.clone(),
        Arc::new(UnusedExec),
    );
    (view, adapter)
}

fn shadow_feature(id: &FeatureId) -> Feature {
    Feature {
        id: id.clone(),
        project_id: ProjectId::from("p-1".to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: "detached feature".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.8,
        duration: "2m".to_string(),
        tokens: 500,
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

fn shadow_step(feature_id: &FeatureId) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-shadow".to_string()),
        feature_id: feature_id.clone(),
        step_id: StepId::from("s-implement".to_string()),
        step_index: 0,
        step_kind: "sequence".to_string(),
        status: "running".to_string(),
        cost_usd: Some(0.8),
        tokens: Some(500),
        wall_clock_secs: Some(120),
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

/// Reproduces "task list not shown in the implement step for detached runs".
///
/// `hydrate_shadow_feature` (`application/remote_runs/reconcile.rs`) mirrors a
/// runner-owned feature's `Feature` and `StepExecution` rows into the
/// laptop's local tables on every poll, and — since the fix for this bug —
/// also mirrors the node's plan cache, checkpoint, and subtask runs via the
/// runner's `get_sequence_state` RPC. This test writes exactly what that
/// hydrate now persists locally (the same three tables, via the same
/// `SequenceResumeRepository`/`FeatureRepository` write methods it calls)
/// and asserts `RunView::sequence_state` — the function
/// `sequence_tasks_list`/`SequenceTasks.tsx`'s "Task list" panel reads
/// through — renders it, exactly as it already does for a local/SSH run.
#[test]
fn sequence_state_stays_unplanned_after_a_shadow_hydrate_even_though_the_runner_has_a_real_plan() {
    let (view, adapter) = make_view();
    let feature_id = FeatureId::from("f-detached".to_string());
    let step = shadow_step(&feature_id);
    let node_id = step.step_id.as_str();

    ProjectRepository::add(
        &*adapter,
        Project {
            id: ProjectId::from("p-1".to_string()),
            name: "detached project".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        },
    )
    .unwrap();

    // What a poll of `hydrate_shadow_feature` persists for a detached run:
    // the feature row and its step-execution shadow...
    FeatureRepository::add(&*adapter, shadow_feature(&feature_id)).unwrap();
    adapter.step_create(step.clone()).unwrap();

    // ...and, once the runner has genuinely decomposed the step, the plan
    // cache, checkpoint, and per-task run rows the `get_sequence_state` RPC
    // returned on this poll.
    adapter
        .plan_cache_put(
            &feature_id,
            node_id,
            r#"{"tasks":[{"id":"t1","title":"Reproduce the bug"}],"cycle":0}"#,
            None,
            0,
        )
        .unwrap();
    adapter
        .sequence_checkpoint_set(&feature_id, node_id, &["t1".to_string()], None, None, 0)
        .unwrap();
    adapter
        .subtask_runs_replace_for_step(
            &feature_id,
            &step.id,
            &[SubtaskRunMirrorRow {
                id: "sr-1".to_string(),
                subtask_id: "t1".to_string(),
                agent_id: Some("claude-code".to_string()),
                worktree_path: "/work/f-detached".to_string(),
                branch: "feature/f-detached".to_string(),
                status: "completed".to_string(),
                cost_usd: 0.42,
                tokens: 1200,
                error_message: None,
                started_at: 0,
                ended_at: Some(30_000),
            }],
        )
        .unwrap();

    let state = view.sequence_state(&feature_id, node_id, &step.id).unwrap();

    assert!(
        state.planned,
        "a detached run's shadowed sequence step must show its task list, \
         not read as unplanned just because nothing mirrored the runner's \
         plan cache onto the laptop"
    );
    assert_eq!(state.tasks.len(), 1);
    assert_eq!(state.tasks[0].id, "t1");
    assert!(
        state.tasks[0].landed,
        "the mirrored checkpoint must mark the task landed"
    );
}
