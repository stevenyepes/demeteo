use crate::domain::models::{Feature, GateDecision, StepExecution};
use async_trait::async_trait;
use serde::Serialize;

/// Step executor — the DAG engine that drives a `Feature` through its
/// workflow.
///
/// **All methods are async.** Tauri supports async commands natively
/// (v2). Making the port async removes the previous `block_in_place`
/// anti-pattern used to call async impls from sync trait methods.
#[async_trait]
pub trait StepExecutor: Send + Sync {
    /// Start a new feature run.
    ///
    /// - `title`: short human label for the feature (used as the
    ///   `features.title` row, the worktree branch slug, and the
    ///   ProjectHome header).
    /// - `description`: the rich prompt body. This is what gets
    ///   rendered into `{{feature_description}}` for every step.
    ///   Required — the executor refuses to start with an empty
    ///   description.
    /// - `agent_kind`, `model`: per-feature overrides for the project's
    ///   defaults. `None` means "use whatever the project says".
    /// - `commit_artifacts`: per-feature override for the project's
    ///   `commit_artifacts` setting. `None` means inherit the project
    ///   default. See migration V12 and `commit_worktree_changes`.
    /// - `loop_iterations`: per-run override of the `on_failure` retry-loop
    ///   budget. `None` means inherit the project default
    ///   (`ProjectSettings::default_loop_iterations`) or the engine default.
    /// - `step_overrides`: per-step agent/model overrides chosen at launch.
    ///   See migration V13.
    #[allow(clippy::too_many_arguments)]
    async fn feature_start(
        &self,
        project_id: &str,
        workflow_id: &str,
        title: &str,
        description: &str,
        agent_kind: Option<&str>,
        model: Option<&str>,
        commit_artifacts: Option<bool>,
        loop_iterations: Option<u32>,
        step_overrides: Vec<crate::domain::models::StepOverride>,
    ) -> Result<Feature, String>;

    async fn feature_pause(&self, feature_id: &str) -> Result<(), String>;
    async fn feature_resume(&self, feature_id: &str) -> Result<(), String>;
    async fn feature_cancel(&self, feature_id: &str) -> Result<(), String>;

    async fn step_get(&self, execution_id: &str) -> Result<StepExecution, String>;
    /// Retry a failed/interrupted step. `new_model` / `new_agent` re-pin the
    /// feature-wide model/harness overrides before the rerun (`None` keeps the
    /// existing override).
    async fn step_retry(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        new_agent: Option<&str>,
    ) -> Result<(), String>;
    /// Replay from the given step execution — reset the target step and
    /// all subsequent steps to `pending`, clear their artifacts and gate
    /// decisions, then restart the execution loop. Works for any step
    /// status (completed, failed, interrupted, awaiting_gate, running).
    /// `new_model` / `new_agent` re-pin the feature-wide overrides before the
    /// rerun (`None` keeps the existing override).
    async fn replay_from_step(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        new_agent: Option<&str>,
    ) -> Result<(), String>;
    async fn step_list_for_run(&self, feature_id: &str) -> Result<Vec<StepExecution>, String>;

    /// Sync the feature branch with `origin/<default_branch>`. Returns
    /// the audit-shaped result so the UI can show a clean merge, no
    /// changes, or a conflict list. The optional
    /// `revalidate_step_execution_id` is used after conflict
    /// resolution: the executor replays that step so the validation
    /// runs again on the freshly-synced tree.
    async fn feature_sync(
        &self,
        feature_id: &str,
        revalidate_step_execution_id: Option<&str>,
    ) -> Result<SyncOutcomeView, String>;

    /// Spawn a fresh agent session to resolve the merge conflicts left
    /// over from `feature_sync`. The agent runs in a temporary
    /// worktree on the conflicted feature branch, edits the conflict
    /// files to remove markers, and commits the resolution. After
    /// committing, the resolution is merged back into the feature
    /// branch on the main repo. If `revalidate_step_execution_id` is
    /// provided, the named step is replayed so the workflow's
    /// validation re-runs on the freshly-merged tree.
    async fn feature_resolve_sync_conflicts(
        &self,
        feature_id: &str,
        conflict_files: &[String],
        revalidate_step_execution_id: Option<&str>,
    ) -> Result<SyncOutcomeView, String>;
}

/// What `feature_sync` and `feature_resolve_sync_conflicts` return to
/// the UI. Serialized verbatim so the React side can render the
/// outcome without re-parsing the database.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncOutcomeView {
    /// The merge produced a clean commit (or there was nothing to
    /// merge from upstream).
    Ok {
        merge_commit_sha: String,
        changed: bool,
    },
    /// The merge left the working tree in a conflicted state and no
    /// resolution was attempted. `conflict_files` is the parsed list
    /// of unmerged paths.
    Conflict {
        conflict_files: Vec<crate::domain::models::ConflictFile>,
        raw_error: String,
    },
    /// A previous conflict was successfully resolved by an agent and
    /// the feature branch is now clean.
    Resolved {
        merge_commit_sha: String,
        revalidated_step_id: Option<String>,
    },
    /// The resolution agent was spawned but failed to clean up the
    /// conflicts. The user is expected to take over (the working
    /// tree is still conflicted).
    ResolutionFailed {
        reason: String,
        conflict_files: Vec<crate::domain::models::ConflictFile>,
    },
}

#[async_trait]
pub trait GatePresenter: Send + Sync {
    async fn gate_pending_for_run(&self, feature_id: &str) -> Result<Option<GateDecision>, String>;
    async fn gate_decide(
        &self,
        step_execution_id: &str,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), String>;
}
