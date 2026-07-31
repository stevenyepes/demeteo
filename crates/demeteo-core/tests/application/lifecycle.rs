// Tests extracted from `src/application/lifecycle.rs` (mirrored-tests convention).

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{RepositoryId, StepExecutionId, WorkflowId, WorkflowVersionId};
use crate::domain::models::{
    Feature, Project, ProjectSettings, Repository, StepAttempt, StepExecution, SubtaskRunRow,
};
use crate::ports::db::{FeatureRepository, StepExecutionPatch};
use crate::ports::remote_run_mirror::{RemoteRunMirror, RemoteRunMirrorPort};
use crate::ports::worktree_ops::{
    CommitMessageRejected, SquashOutcome, SyncFailure, SyncOutcome, WorktreeOpsPort,
};
use crate::state::AppContext;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

/// A deliberately narrow lifecycle double: every call other than `update`
/// is rejected so an accidental expansion of cleanup's persistence surface is
/// visible to these tests.
struct RecordingFeatures {
    calls: Arc<Mutex<Vec<String>>>,
    update_error: Option<String>,
    feature: Option<Feature>,
}

impl RecordingFeatures {
    fn new(calls: Arc<Mutex<Vec<String>>>, update_error: Option<&str>) -> Self {
        Self {
            calls,
            update_error: update_error.map(str::to_string),
            feature: None,
        }
    }

    fn with_feature(calls: Arc<Mutex<Vec<String>>>, feature: Feature) -> Self {
        Self {
            calls,
            update_error: None,
            feature: Some(feature),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

macro_rules! reject_feature_call {
    () => {
        panic!("unexpected FeatureRepository call")
    };
}

impl FeatureRepository for RecordingFeatures {
    fn get_active(&self, _: &ProjectId) -> Result<Vec<Feature>, String> {
        reject_feature_call!()
    }
    fn get(&self, id: &FeatureId) -> Result<Option<Feature>, String> {
        match self.feature.as_ref() {
            Some(feature) if feature.id == *id => Ok(Some(feature.clone())),
            Some(_) => Ok(None),
            None => reject_feature_call!(),
        }
    }
    fn add(&self, _: Feature) -> Result<(), String> {
        reject_feature_call!()
    }
    fn update(&self, id: &FeatureId, patch: &FeaturePatch) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!(
            "update:{}:{}",
            id.as_str(),
            patch.status.as_deref().unwrap_or("")
        ));
        self.update_error.clone().map_or(Ok(()), Err)
    }
    fn update_workflow_id(&self, _: &FeatureId, _: &WorkflowId) -> Result<(), String> {
        reject_feature_call!()
    }
    fn merge_harness_baseline(
        &self,
        _: &FeatureId,
        _: &crate::domain::harness_baseline::HarnessBaseline,
    ) -> Result<(), String> {
        reject_feature_call!()
    }
    fn pin_workflow_version(&self, _: &FeatureId, _: &WorkflowVersionId) -> Result<(), String> {
        reject_feature_call!()
    }
    fn list_with_open_mr(&self) -> Result<Vec<Feature>, String> {
        reject_feature_call!()
    }
    fn step_create(&self, _: StepExecution) -> Result<(), String> {
        reject_feature_call!()
    }
    fn step_get(&self, _: &StepExecutionId) -> Result<Option<StepExecution>, String> {
        reject_feature_call!()
    }
    fn step_update(&self, _: &StepExecutionId, _: &StepExecutionPatch) -> Result<(), String> {
        reject_feature_call!()
    }
    fn steps_for_feature(&self, _: &FeatureId) -> Result<Vec<StepExecution>, String> {
        reject_feature_call!()
    }
    fn attempt_open(&self, _: &StepExecutionId, _: i64, _: Option<&str>) -> Result<u32, String> {
        reject_feature_call!()
    }
    fn attempt_close(
        &self,
        _: &StepExecutionId,
        _: u32,
        _: &str,
        _: f64,
        _: i64,
        _: u64,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: i64,
    ) -> Result<(), String> {
        reject_feature_call!()
    }
    fn attempts_for_step(&self, _: &StepExecutionId) -> Result<Vec<StepAttempt>, String> {
        reject_feature_call!()
    }
    fn subtask_runs_for_step(&self, _: &StepExecutionId) -> Result<Vec<SubtaskRunRow>, String> {
        reject_feature_call!()
    }
}

struct RecordingMirrors {
    calls: Arc<Mutex<Vec<String>>>,
    delete_error: Option<String>,
    present: Arc<Mutex<bool>>,
}

impl RecordingMirrors {
    fn new(calls: Arc<Mutex<Vec<String>>>, delete_error: Option<&str>) -> Self {
        Self {
            calls,
            delete_error: delete_error.map(str::to_string),
            present: Arc::new(Mutex::new(true)),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn is_present(&self) -> bool {
        *self.present.lock().unwrap()
    }
}

impl RemoteRunMirrorPort for RecordingMirrors {
    fn upsert_submitted(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: &str,
        _: i64,
    ) -> Result<RemoteRunMirror, String> {
        panic!("unexpected RemoteRunMirrorPort call")
    }
    fn update_status(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<&str>,
        _: i64,
        _: i64,
    ) -> Result<(), String> {
        panic!("unexpected RemoteRunMirrorPort call")
    }
    fn mark_notified(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        panic!("unexpected RemoteRunMirrorPort call")
    }
    fn delete_for_feature(&self, feature_id: &str) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("dismiss:{feature_id}"));
        match self.delete_error.clone() {
            Some(error) => Err(error),
            None => {
                *self.present.lock().unwrap() = false;
                Ok(())
            }
        }
    }
    fn get(&self, _: &str, _: &str) -> Result<Option<RemoteRunMirror>, String> {
        panic!("unexpected RemoteRunMirrorPort call")
    }
    fn list(&self) -> Result<Vec<RemoteRunMirror>, String> {
        panic!("unexpected RemoteRunMirrorPort call")
    }
}

#[test]
fn archive_and_accepted_auto_delete_dismiss_after_local_status_update() {
    for (status, expected) in [("archived", "archived"), ("deleted", "deleted")] {
        let calls = Arc::new(Mutex::new(vec![]));
        let features = RecordingFeatures::new(calls.clone(), None);
        let mirrors = RecordingMirrors::new(calls, None);
        let feature_id = FeatureId::from("f-cleanup");

        persist_feature_cleanup(&features, &mirrors, &feature_id, status).unwrap();

        assert_eq!(
            features.calls(),
            [
                format!("update:f-cleanup:{expected}"),
                "dismiss:f-cleanup".to_string()
            ]
        );
        assert_eq!(
            mirrors.calls(),
            [
                format!("update:f-cleanup:{expected}"),
                "dismiss:f-cleanup".to_string()
            ]
        );
    }
}

#[test]
fn failed_status_update_does_not_dismiss_and_failed_dismissal_is_propagated() {
    let feature_id = FeatureId::from("f-cleanup");
    let calls = Arc::new(Mutex::new(vec![]));
    let features = RecordingFeatures::new(calls.clone(), Some("state write failed"));
    let mirrors = RecordingMirrors::new(calls, None);
    assert_eq!(
        persist_feature_cleanup(&features, &mirrors, &feature_id, "archived"),
        Err("state write failed".to_string())
    );
    assert_eq!(mirrors.calls(), ["update:f-cleanup:archived"]);

    let calls = Arc::new(Mutex::new(vec![]));
    let features = RecordingFeatures::new(calls.clone(), None);
    let mirrors = RecordingMirrors::new(calls, Some("mirror write failed"));
    assert_eq!(
        persist_feature_cleanup(&features, &mirrors, &feature_id, "deleted"),
        Err("mirror write failed".to_string())
    );
    assert_eq!(
        features.calls(),
        ["update:f-cleanup:deleted", "dismiss:f-cleanup"]
    );
    assert_eq!(
        mirrors.calls(),
        ["update:f-cleanup:deleted", "dismiss:f-cleanup"]
    );
}

struct RecordingWorktrees {
    calls: Arc<Mutex<Vec<String>>>,
}

type CleanupContext = (
    AppContext,
    Arc<RecordingFeatures>,
    Arc<RecordingMirrors>,
    Arc<Mutex<Vec<String>>>,
);

#[async_trait]
impl WorktreeOpsPort for RecordingWorktrees {
    async fn check_repo_dirty(&self, _: Option<&str>, _: &str) -> Result<(bool, bool), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn get_head_branch(&self, _: Option<&str>, _: &str) -> Option<String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn list_worktrees(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<Vec<crate::domain::models::WorktreeInfo>, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn detect_worktree_strategy(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<crate::domain::models::WorktreeStrategy, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn clone_repository(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn create_feature_branch(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn provision_subtask_worktree(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<String, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn cleanup_subtask_worktree(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn branch_delete(&self, _: Option<&str>, _: &str, _: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push("branch_delete".to_string());
        Ok(())
    }
    async fn merge_subtask(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn sync_feature_with_upstream(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<SyncOutcome, SyncFailure> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn validate_commit_message(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
    ) -> Result<(), CommitMessageRejected> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn squash_feature_branch(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<SquashOutcome, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn restore_pre_squash(&self, _: Option<&str>, _: &str, _: &str) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
}

fn cleanup_feature(mr_state: &str) -> Feature {
    Feature {
        id: FeatureId::from("f-cleanup"),
        project_id: ProjectId::from("p-cleanup"),
        workflow_id: None,
        workflow_version_id: None,
        title: "Cleanup fixture".to_string(),
        description: String::new(),
        status: "completed".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: 0,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: Some(mr_state.to_string()),
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: vec![],
        attachments: vec![],
        harness_baseline: None,
    }
}

fn cleanup_context(policy: &str, mr_state: &str) -> CleanupContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-lifecycle-cleanup-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let project_id = ProjectId::from("p-cleanup");
    ctx.projects
        .add(Project {
            id: project_id.clone(),
            name: "cleanup fixture".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .unwrap();
    let mut settings: ProjectSettings =
        crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = project_id.clone();
    settings.feature_lifecycle = policy.to_string();
    ctx.projects.save_settings(settings).unwrap();
    ctx.projects
        .add_repository(Repository {
            id: RepositoryId::from("r-cleanup"),
            project_id,
            provider_id: crate::domain::ids::ProviderId::from("provider-cleanup"),
            repo_path: "fixture/repo".to_string(),
        })
        .unwrap();

    let calls = Arc::new(Mutex::new(vec![]));
    let features = Arc::new(RecordingFeatures::with_feature(
        calls.clone(),
        cleanup_feature(mr_state),
    ));
    let mirrors = Arc::new(RecordingMirrors::new(calls.clone(), None));
    ctx.features = features.clone();
    ctx.remote_run_mirror = mirrors.clone();
    ctx.worktree_ops = Arc::new(RecordingWorktrees {
        calls: calls.clone(),
    });
    (ctx, features, mirrors, calls)
}

#[tokio::test]
async fn feature_cleanup_policy_branches_dismiss_only_successful_transitions() {
    let (ctx, features, mirrors, calls) = cleanup_context("keep", "open");
    let result = feature_cleanup(&ctx, "f-cleanup".to_string(), None)
        .await
        .unwrap();
    assert_eq!(result.action, "noop");
    assert!(calls.lock().unwrap().is_empty());
    assert!(mirrors.is_present(), "keep must retain its mirror");
    assert!(features.calls().is_empty());

    let (ctx, _features, mirrors, calls) = cleanup_context("archive", "open");
    let result = feature_cleanup(&ctx, "f-cleanup".to_string(), None)
        .await
        .unwrap();
    assert_eq!(result.action, "archived");
    assert_eq!(
        *calls.lock().unwrap(),
        ["update:f-cleanup:archived", "dismiss:f-cleanup"]
    );
    assert!(!mirrors.is_present(), "archive must dismiss its mirror");

    let (ctx, _features, mirrors, calls) = cleanup_context("auto_delete", "open");
    let result = feature_cleanup(&ctx, "f-cleanup".to_string(), Some(true))
        .await
        .unwrap();
    assert_eq!(result.action, "deleted");
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "branch_delete",
            "update:f-cleanup:deleted",
            "dismiss:f-cleanup"
        ]
    );
    assert!(
        !mirrors.is_present(),
        "accepted auto-delete must dismiss its mirror"
    );

    let (ctx, features, mirrors, calls) = cleanup_context("auto_delete", "open");
    let error = match feature_cleanup(&ctx, "f-cleanup".to_string(), None).await {
        Ok(_) => panic!("unmerged auto-delete without force must fail"),
        Err(error) => error,
    };
    assert!(error.contains("requires the MR to be merged"));
    assert!(calls.lock().unwrap().is_empty());
    assert!(
        mirrors.is_present(),
        "rejected auto-delete must retain its mirror"
    );
    assert!(features.calls().is_empty());
}
