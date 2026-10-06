// Tests extracted from `src/application/lifecycle.rs` (mirrored-tests convention).

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{RepositoryId, StepExecutionId, WorkflowId, WorkflowVersionId};
use crate::domain::models::{
    Feature, Project, ProjectSettings, Repository, StepAttempt, StepExecution, SubtaskRunRow,
};
use crate::domain::runner_cache_release::{CacheReleaseReason, PendingRunnerRelease};
use crate::ports::db::{FeatureRepository, StepExecutionPatch};
use crate::ports::remote_run_mirror::{RemoteRunMirror, RemoteRunMirrorPort, RunnerCachePort};
use crate::ports::worktree_ops::{
    BranchDeleted, CommitMessageRejected, SquashOutcome, SyncFailure, SyncOutcome, WorktreeOpsPort,
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
    fn subtask_runs_mirror_for_step(
        &self,
        _: &StepExecutionId,
    ) -> Result<Vec<crate::domain::models::SubtaskRunMirrorRow>, String> {
        reject_feature_call!()
    }
    fn subtask_runs_replace_for_step(
        &self,
        _: &FeatureId,
        _: &StepExecutionId,
        _: &[crate::domain::models::SubtaskRunMirrorRow],
    ) -> Result<(), String> {
        reject_feature_call!()
    }
}

struct RecordingMirrors {
    calls: Arc<Mutex<Vec<String>>>,
    delete_error: Option<String>,
    present: Arc<Mutex<bool>>,
    /// What `list` answers while the mirror is present.
    rows: Vec<RemoteRunMirror>,
}

impl RecordingMirrors {
    fn new(calls: Arc<Mutex<Vec<String>>>, delete_error: Option<&str>) -> Self {
        Self {
            calls,
            delete_error: delete_error.map(str::to_string),
            present: Arc::new(Mutex::new(true)),
            rows: Vec::new(),
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
        Ok(if self.is_present() {
            self.rows.clone()
        } else {
            Vec::new()
        })
    }
    fn list_for_features(&self, _: &[&str]) -> Result<Vec<RemoteRunMirror>, String> {
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

/// Records the machine, clone dir and branch each cleanup call names, and
/// answers every cache release with `cache_error` when one is set.
struct RecordingWorktrees {
    calls: Arc<Mutex<Vec<String>>>,
    cache_error: Option<String>,
}

impl RecordingWorktrees {
    fn record(&self, call: &str, machine: Option<&str>, repo_dir: &str, branch: &str) {
        self.calls.lock().unwrap().push(format!(
            "{call}:{}:{repo_dir}:{branch}",
            machine.unwrap_or("<none>")
        ));
    }

    fn cache_result(&self) -> Result<(), String> {
        self.cache_error.clone().map_or(Ok(()), Err)
    }
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
    async fn create_terminal_worktree(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &crate::ports::worktree_ops::TerminalWorktreeRequest,
    ) -> Result<crate::ports::worktree_ops::TerminalWorktreeCreated, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn remove_terminal_worktree(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: &str,
        _: bool,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn list_terminal_branches(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<Vec<crate::domain::branch_listing::BranchOption>, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn list_terminal_worktrees(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
    ) -> Result<Vec<crate::domain::models::WorktreeInfo>, String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn cleanup_legacy_terminal_worktrees(
        &self,
        _: Option<&str>,
        _: &str,
    ) -> Result<usize, String> {
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
    async fn fetch_origin_refspec(
        &self,
        _: Option<&str>,
        _: &str,
        _: &crate::domain::feature_origin::Refspec,
    ) -> Result<(), String> {
        panic!("unexpected WorktreeOpsPort call")
    }
    async fn cut_branch_at(
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
    async fn create_and_push_branch(
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
    async fn branch_delete(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
        branch: &str,
    ) -> Result<BranchDeleted, String> {
        self.record("branch_delete", machine, repo_dir, branch);
        Ok(BranchDeleted {
            cache_release: self.cache_result(),
        })
    }
    async fn release_feature_cache(
        &self,
        machine: Option<&str>,
        repo_dir: &str,
        branch: &str,
    ) -> Result<(), String> {
        self.record("release_cache", machine, repo_dir, branch);
        self.cache_result()
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
        _: crate::ports::worktree_ops::MergeGate<'_>,
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

fn cleanup_feature(status: &str, mr_state: &str) -> Feature {
    Feature {
        id: FeatureId::from("f-cleanup"),
        project_id: ProjectId::from("p-cleanup"),
        workflow_id: None,
        workflow_version_id: None,
        title: "Cleanup fixture".to_string(),
        description: String::new(),
        status: status.to_string(),
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
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    }
}

fn cleanup_context(policy: &str, mr_state: &str) -> CleanupContext {
    cleanup_context_with(policy, "completed", mr_state, None)
}

fn cleanup_context_with(
    policy: &str,
    status: &str,
    mr_state: &str,
    cache_error: Option<&str>,
) -> CleanupContext {
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
        cleanup_feature(status, mr_state),
    ));
    let mirrors = Arc::new(RecordingMirrors::new(calls.clone(), None));
    ctx.features = features.clone();
    ctx.remote_run_mirror = mirrors.clone();
    ctx.worktree_ops = Arc::new(RecordingWorktrees {
        calls: calls.clone(),
        cache_error: cache_error.map(str::to_string),
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
        [
            cleanup_call("release_cache", &ctx),
            "update:f-cleanup:archived".to_string(),
            "dismiss:f-cleanup".to_string(),
        ]
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
            cleanup_call("branch_delete", &ctx),
            "update:f-cleanup:deleted".to_string(),
            "dismiss:f-cleanup".to_string(),
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

/// The call a cleanup of the fixture feature must make: on the local machine,
/// against the project's clone under the workspace — never the repository's
/// `owner/name` slug, which names a relative path that does not exist.
fn cleanup_call(call: &str, ctx: &AppContext) -> String {
    let clone_dir =
        crate::paths::repo_target_dir_local(&ctx.workspace_dir, "p-cleanup", "fixture/repo");
    let prefix = crate::adapters::step_executor::setup::fetch_default_settings()
        .worktree_strategy
        .branch_prefix;
    format!(
        "{call}:{}:{}:{}",
        crate::domain::ids::LOCAL_MACHINE,
        clone_dir.display(),
        cleanup_feature("completed", "merged").run_branch(&prefix)
    )
}

#[tokio::test]
async fn auto_delete_targets_the_project_clone_not_the_repository_slug() {
    let (ctx, _features, _mirrors, calls) = cleanup_context("auto_delete", "merged");
    let result = feature_cleanup(&ctx, "f-cleanup".to_string(), None)
        .await
        .unwrap();

    assert!(result.branch_deleted);
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(
        calls.lock().unwrap()[0],
        cleanup_call("branch_delete", &ctx)
    );
}

#[tokio::test]
async fn keep_releases_the_cache_of_a_feature_whose_pr_is_settled() {
    for mr_state in ["merged", "closed"] {
        let (ctx, _features, mirrors, calls) = cleanup_context("keep", mr_state);
        let result = feature_cleanup(&ctx, "f-cleanup".to_string(), None)
            .await
            .unwrap();

        assert_eq!(result.action, "noop");
        assert_eq!(
            *calls.lock().unwrap(),
            [cleanup_call("release_cache", &ctx)],
            "keep on a {mr_state} PR"
        );
        assert!(mirrors.is_present());
    }
}

#[tokio::test]
async fn keep_leaves_the_cache_of_a_feature_that_can_still_run() {
    for (status, mr_state) in [("completed", "open"), ("awaiting_mr", "none")] {
        let (ctx, _features, _mirrors, calls) =
            cleanup_context_with("keep", status, mr_state, None);
        feature_cleanup(&ctx, "f-cleanup".to_string(), None)
            .await
            .unwrap();

        assert!(
            calls.lock().unwrap().is_empty(),
            "keep on a {status} feature with a {mr_state} PR released its cache"
        );
    }
}

#[tokio::test]
async fn a_cache_that_will_not_delete_is_a_warning_not_a_failed_cleanup() {
    let (ctx, _features, mirrors, _calls) =
        cleanup_context_with("archive", "completed", "open", Some("cache held open"));
    let result = feature_cleanup(&ctx, "f-cleanup".to_string(), None)
        .await
        .unwrap();
    assert_eq!(result.action, "archived");
    assert_eq!(result.warnings, ["Dependency cache: cache held open"]);
    assert!(!mirrors.is_present());

    let (ctx, _features, _mirrors, _calls) = cleanup_context_with(
        "auto_delete",
        "completed",
        "merged",
        Some("cache held open"),
    );
    let result = feature_cleanup(&ctx, "f-cleanup".to_string(), None)
        .await
        .unwrap();
    assert!(
        result.branch_deleted,
        "the branch went even if the cache did not"
    );
    assert_eq!(result.warnings, ["Dependency cache: cache held open"]);
}

// ── runner-owned features ───────────────────────────────────────────
//
// A shadow's cache is beside the runner's clone; the local release would look
// for it on this process's view of the project, find nothing, and report
// success. So a shadow is routed to the runner, and what could not reach it is
// kept for the next reconcile — the cleanup that dismisses the mirror leaves
// nothing else on this machine that remembers the run.

fn runner_row() -> RemoteRunMirror {
    RemoteRunMirror {
        machine_id: "runner-1".to_string(),
        run_id: "run-1".to_string(),
        project_id: Some("p-cleanup".to_string()),
        title: "Cleanup fixture".to_string(),
        status: "awaiting_mr".to_string(),
        error: None,
        feature_id: Some("f-cleanup".to_string()),
        pr_url: None,
        pushed_branch: None,
        last_offset: 0,
        created_at: 0,
        updated_at: 0,
        last_notified_status: None,
    }
}

fn runner_owned_cleanup_context(policy: &str, mr_state: &str) -> CleanupContext {
    let (mut ctx, features, _local_mirrors, calls) =
        cleanup_context_with(policy, "completed", mr_state, None);
    let mut mirrors = RecordingMirrors::new(calls.clone(), None);
    mirrors.rows = vec![runner_row()];
    let mirrors = Arc::new(mirrors);
    ctx.remote_run_mirror = mirrors.clone();
    (ctx, features, mirrors, calls)
}

fn pending(reason: CacheReleaseReason) -> PendingRunnerRelease {
    PendingRunnerRelease {
        machine_id: "runner-1".to_string(),
        run_id: "run-1".to_string(),
        reason,
    }
}

/// `ctx.exec` here has no runner behind any machine, which is the unreachable
/// runner: the cleanup must still land, and the release must be kept.
#[tokio::test]
async fn cleaning_up_a_shadow_asks_its_runner_and_keeps_what_did_not_arrive() {
    for (policy, mr_state, reason, expected_calls) in [
        ("keep", "merged", CacheReleaseReason::Merged, &[][..]),
        (
            "archive",
            "open",
            CacheReleaseReason::Dismissed,
            &["update:f-cleanup:archived", "dismiss:f-cleanup"][..],
        ),
        (
            "auto_delete",
            "merged",
            CacheReleaseReason::Merged,
            &[
                "branch_delete",
                "update:f-cleanup:deleted",
                "dismiss:f-cleanup",
            ][..],
        ),
    ] {
        let (ctx, _features, _mirrors, calls) = runner_owned_cleanup_context(policy, mr_state);
        let result = feature_cleanup(&ctx, "f-cleanup".to_string(), None)
            .await
            .unwrap_or_else(|e| panic!("{policy}: an unreachable runner failed the cleanup: {e}"));

        let calls: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| match c.starts_with("branch_delete:") {
                true => "branch_delete".to_string(),
                false => c.clone(),
            })
            .collect();
        assert_eq!(
            calls, expected_calls,
            "{policy}: a shadow's cache must never be released on this machine's view of the project"
        );
        assert_eq!(result.warnings.len(), 1, "{policy}: {:?}", result.warnings);
        assert!(
            result.warnings[0].starts_with("Dependency cache: "),
            "{policy}: {:?}",
            result.warnings
        );
        assert_eq!(
            crate::application::remote_runs::pending_runner_releases(&*ctx.app_settings).unwrap(),
            [pending(reason)],
            "{policy}"
        );
    }
}

#[derive(Default)]
struct RecordingLocalCache {
    released: Mutex<Vec<String>>,
}

#[async_trait]
impl FeatureCachePort for RecordingLocalCache {
    async fn release(&self, feature: &Feature) -> Result<(), String> {
        self.released
            .lock()
            .unwrap()
            .push(feature.id.as_str().to_string());
        Ok(())
    }
}

#[derive(Default)]
struct RecordingRunner {
    calls: Mutex<Vec<(String, String, CacheReleaseReason)>>,
}

#[async_trait]
impl RunnerCachePort for RecordingRunner {
    async fn release_feature_cache(
        &self,
        machine_id: &str,
        run_id: &str,
        reason: CacheReleaseReason,
    ) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push((machine_id.to_string(), run_id.to_string(), reason));
        Ok(())
    }
}

fn routed(
    rows: Vec<RemoteRunMirror>,
) -> (
    RoutedFeatureCacheRelease,
    Arc<RecordingLocalCache>,
    Arc<RecordingRunner>,
) {
    let local = Arc::new(RecordingLocalCache::default());
    let runner = Arc::new(RecordingRunner::default());
    let mut mirrors = RecordingMirrors::new(Arc::new(Mutex::new(vec![])), None);
    mirrors.rows = rows;
    let settings = Arc::new(
        crate::adapters::database::SqliteAdapter::new(
            rusqlite::Connection::open_in_memory().unwrap(),
        )
        .unwrap(),
    );
    (
        RoutedFeatureCacheRelease {
            local: local.clone(),
            remote_run_mirror: Arc::new(mirrors),
            runner: runner.clone(),
            app_settings: settings,
        },
        local,
        runner,
    )
}

#[tokio::test]
async fn a_shadow_is_released_by_its_runner_with_the_reason() {
    let (cache, local, runner) = routed(vec![runner_row()]);

    cache
        .release(&cleanup_feature("awaiting_mr", "merged"))
        .await
        .unwrap();

    assert_eq!(
        *runner.calls.lock().unwrap(),
        [(
            "runner-1".to_string(),
            "run-1".to_string(),
            CacheReleaseReason::Merged
        )]
    );
    assert!(local.released.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_feature_this_machine_owns_is_released_here() {
    let mut elsewhere = runner_row();
    elsewhere.feature_id = Some("f-other".to_string());
    let (cache, local, runner) = routed(vec![elsewhere]);

    cache
        .release(&cleanup_feature("completed", "merged"))
        .await
        .unwrap();

    assert_eq!(*local.released.lock().unwrap(), ["f-cleanup"]);
    assert!(runner.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_shadow_that_has_not_finished_asks_no_one() {
    let (cache, local, runner) = routed(vec![runner_row()]);

    cache
        .release(&cleanup_feature("awaiting_mr", "open"))
        .await
        .expect_err("there is no reason to give the runner");

    assert!(runner.calls.lock().unwrap().is_empty());
    assert!(local.released.lock().unwrap().is_empty());
}

/// Answers `fetch_mr_state` with one fixed state; any other call is a test bug.
struct FixedMrState(&'static str);

#[async_trait]
impl crate::ports::mr_publisher::MrPublisher for FixedMrState {
    async fn publish_mr(
        &self,
        _: &str,
        _: &FeatureId,
        _: crate::domain::models::PublishOptions,
    ) -> Result<crate::domain::models::MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_state(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(self.0.to_string())
    }
    async fn list_open_mrs(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::mr_summary::MrSummary>, crate::domain::mr_list_error::MrListError>
    {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_detail(
        &self,
        _: &str,
        _: &str,
    ) -> Result<crate::domain::mr_summary::MrSummary, crate::domain::mr_list_error::MrListError>
    {
        panic!("unexpected MrPublisher call")
    }
    async fn post_mr_comment(&self, _: &str, _: &str, _: &str) -> Result<String, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn publish_branch_mr(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: crate::domain::models::PublishOptions,
    ) -> Result<crate::domain::models::MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
}

/// The shadow's row still reads `open` — hydration copies the runner's view,
/// and the runner could not see the merge — so the runner must be told the
/// state this cleanup just fetched, not the row's.
#[tokio::test]
async fn keep_tells_the_runner_the_freshly_fetched_pr_state() {
    let (mut ctx, _features, _mirrors, calls) = runner_owned_cleanup_context("keep", "open");
    let mut shadow = cleanup_feature("completed", "open");
    shadow.mr_url = Some("https://example.invalid/pr/1".to_string());
    ctx.features = Arc::new(RecordingFeatures::with_feature(calls, shadow));
    ctx.mr_publisher = Arc::new(FixedMrState("merged"));

    feature_cleanup(&ctx, "f-cleanup".to_string(), None)
        .await
        .unwrap();

    assert_eq!(
        crate::application::remote_runs::pending_runner_releases(&*ctx.app_settings).unwrap(),
        [pending(CacheReleaseReason::Merged)]
    );
}
