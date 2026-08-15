use crate::application::attachments::StagedAttachmentInput;
use crate::domain::models::{EffortLevel, Feature, GateDecision, StepExecution};
use crate::error::AppError;
use async_trait::async_trait;
use serde::Serialize;

/// Everything a launch decides, in one value.
///
/// Bundled because it travels as a unit from each of the four launch paths —
/// the Tauri command, the runner, the scheduler, and the create-project
/// wizard — straight onto the `Feature` row, and none of them make these
/// choices separately. It was thirteen positional parameters behind
/// `#[allow(clippy::too_many_arguments)]`, six of them `Option`s, which is a
/// signature where a caller can transpose two arguments and get a compiling
/// run that starts with the wrong model.
///
/// `Default` is what keeps the non-launching callers honest: a scheduler that
/// only chooses a project, workflow and title spells exactly that and
/// inherits the rest, rather than counting `None`s.
#[derive(Debug, Clone, Default)]
pub struct FeatureLaunch {
    /// Caller-supplied id for the new Feature row. `None` (every local
    /// launch) generates one. The runner passes `RunSpec::feature_id` so a
    /// detached run's Feature carries the same id as the eager shadow the
    /// laptop inserted at submit time — one run, one id, on both databases.
    pub feature_id: Option<String>,
    pub project_id: String,
    pub workflow_id: String,
    /// Short human label — the `features.title` row, the worktree branch
    /// slug, and the ProjectHome header.
    pub title: String,
    /// The rich prompt body rendered into `{{feature_description}}` for every
    /// step. Required — the executor refuses to start with an empty one.
    pub description: String,
    /// Per-feature overrides for the project's defaults. `None` means "use
    /// whatever the project says" (for effort, that bottoms out at
    /// [`EffortLevel::DEFAULT`]).
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<EffortLevel>,
    /// Per-feature override for the project's `commit_artifacts` setting.
    /// `None` means inherit. See migration V12 and `commit_worktree_changes`.
    pub commit_artifacts: Option<bool>,
    /// Per-run override of the `on_failure` retry-loop budget. `None` means
    /// inherit the project default (`ProjectSettings::default_loop_iterations`)
    /// or the engine default.
    pub loop_iterations: Option<u32>,
    pub max_budget_usd: Option<f64>,
    /// Per-step agent/model overrides chosen at launch. See migration V13.
    pub step_overrides: Vec<crate::domain::models::StepOverride>,
    /// Attachments the user dropped/picked before clicking "Launch feature".
    /// Persisted to the freshly-created feature row BEFORE the driver is
    /// spawned, so the agent's first turn sees them on its first
    /// `features.get(&self.f_id)` read. Empty when nothing was attached.
    pub staged_attachments: Vec<StagedAttachmentInput>,
    /// Where the run's branch is cut from. Defaults to the project's default
    /// branch, which is what every launch did before V41.
    pub origin: crate::domain::feature_origin::FeatureOrigin,
}

/// Step executor — the DAG engine that drives a `Feature` through its
/// workflow.
///
/// **All methods are async.** Tauri supports async commands natively
/// (v2). Making the port async removes the previous `block_in_place`
/// anti-pattern used to call async impls from sync trait methods.
#[async_trait]
pub trait StepExecutor: Send + Sync {
    /// Start a new feature run. Every choice the launch makes is a field of
    /// [`FeatureLaunch`].
    async fn feature_start(&self, launch: FeatureLaunch) -> Result<Feature, String>;

    async fn feature_pause(&self, feature_id: &str) -> Result<(), String>;
    async fn feature_resume(&self, feature_id: &str) -> Result<(), String>;
    async fn feature_cancel(&self, feature_id: &str) -> Result<(), String>;

    async fn step_get(&self, execution_id: &str) -> Result<StepExecution, String>;
    /// Retry a failed/interrupted step. `new_model` / `new_agent` /
    /// `new_effort` re-pin the feature-wide model/harness/effort overrides
    /// before the rerun (`None` keeps the existing override).
    ///
    /// **Precondition:** the executor refuses to retry when an earlier step
    /// (any step with `step_index < target.step_index`) is still non-terminal
    /// (`pending`, `running`, `verifying`, or `awaiting_gate`). The check
    /// surfaces as `AppError::validation` so the UI can both disable the
    /// Retry Step button and surface the blocking predecessor by name.
    async fn step_retry(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        new_agent: Option<&str>,
        new_effort: Option<EffortLevel>,
    ) -> Result<(), AppError>;
    /// Replay from the given step execution — reset the target step and
    /// all subsequent steps to `pending`, clear their artifacts and gate
    /// decisions, then restart the execution loop. Works for any step
    /// status (completed, failed, interrupted, awaiting_gate, running).
    /// `new_model` / `new_agent` / `new_effort` re-pin the feature-wide
    /// overrides before the rerun (`None` keeps the existing override).
    async fn replay_from_step(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        new_agent: Option<&str>,
        new_effort: Option<EffortLevel>,
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
    /// Apply a gate decision for the given step execution.
    ///
    /// **Precondition:** the executor refuses to apply the decision when an
    /// earlier step (`step_index < target.step_index`) is still non-terminal
    /// (`pending`, `running`, `verifying`, or `awaiting_gate`). The check
    /// surfaces as `AppError::validation` so the UI can render a blocking
    /// banner above the Approve / Redirect buttons and disable them.
    async fn gate_decide(
        &self,
        step_execution_id: &str,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), AppError>;
}
