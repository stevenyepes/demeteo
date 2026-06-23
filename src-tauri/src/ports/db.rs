//! Database port — split by bounded context.
//!
//! The original `DatabasePort` trait had 66 methods spanning 13 distinct
//! domains (machines, threads, projects, features, workflows, gates,
//! provider instances, app settings, …). Every consumer of that trait
//! was coupled to the entire schema, the test surface was huge, and the
//! adapter was 1300+ lines. This file splits the trait into 7 narrow
//! sub-ports aligned with the bounded contexts defined in
//! `docs/DDD_MODEL.md`:
//!
//! | Sub-port                  | Bounded context | Owns                                         |
//! |---------------------------|-----------------|----------------------------------------------|
//! | [`MachineRepository`]     | machines        | `Machine`, `AgentProfile`                    |
//! | [`ThreadRepository`]      | threads         | `ThreadSession`, `Message`, `AgentConfig`, `WorkingMemoryEntry` |
//! | [`ProjectRepository`]     | projects        | `Project`, `Repository`, `ProjectSettings`   |
//! | [`FeatureRepository`]     | features        | `Feature`, `StepExecution`                   |
//! | [`WorkflowRepository`]    | workflows       | `Workflow`, `WorkflowVersion`                |
//! | [`GateRepository`]        | gates           | `GateDecision`                               |
//! | [`AppSettingsRepository`] | app settings    | provider instances, app-session KV, first-launch flags |
//!
//! Each sub-port is small (≤ 12 methods), cohesive, and takes
//! strongly-typed ID newtypes. [`AppContext`](crate::state::AppContext)
//! holds one `Arc<dyn ...Repository>` per sub-port, and Tauri commands
//! extract only the sub-port they need.
//!
//! Mutation goes through a [`Patch`] value object
//! ([`ThreadPatch`], [`FeaturePatch`], [`StepExecutionPatch`]). Each
//! field of a Patch is a nested `Option<Option<T>>` so callers can
//! distinguish "leave alone" from "set to NULL" — the previous
//! 6-argument `step_execution_update_status` is gone.

use crate::domain::ids::{
    AgentProfileId, FeatureId, MachineId, MessageId, ProjectId, ProviderId, RepositoryId,
    StepExecutionId, StepId, ThreadId, WorkflowId, WorkflowVersionId,
};
use crate::domain::models::{
    AgentConfig, AgentProfile, Feature, GateDecision, Machine, Message, Project, ProjectSettings,
    ProviderInstance, RepoContext, Repository, StepExecution, ThreadSession, Workflow,
    WorkflowSchedule, WorkflowVersion, WorkingMemoryEntry, WorktreeContext,
};

// ─────────────────────────────────────────────────────────────────────────────
// Patch value objects
// ─────────────────────────────────────────────────────────────────────────────

/// Patch for [`ThreadRepository::update_thread`].
///
/// Each field is `Option<Option<T>>`:
/// * `None` → leave the column alone.
/// * `Some(None)` → set the column to NULL (only for nullable columns).
/// * `Some(Some(v))` → set the column to `v`.
///
/// `touch_timestamp: true` bumps `updated_at` to `now()` (used for
/// sidebar ordering) without changing any other field.
#[derive(Debug, Default, Clone)]
pub struct ThreadPatch {
    pub status: Option<String>,
    pub model: Option<Option<String>>,
    pub touch_timestamp: bool,
}

/// Patch for [`FeatureRepository::update`].
///
/// `None` → leave alone. `Some(None)` → NULL. `Some(Some(v))` → set.
#[derive(Debug, Default, Clone)]
pub struct FeaturePatch {
    pub status: Option<String>,
    pub total_cost: Option<Option<f64>>,
    pub duration: Option<Option<String>>,
    pub tokens: Option<Option<i64>>,
    pub agent_kind: Option<Option<String>>,
    pub model: Option<Option<String>>,
    /// Set/clear the MR/PR URL after [`MrPublisher::publish_mr`].
    pub mr_url: Option<Option<String>>,
    /// Set the MR/PR state on the feature (draft/open/merged/closed).
    pub mr_state: Option<Option<String>>,
}

/// Patch for [`FeatureRepository::step_update`].
///
/// `None` → leave alone. `Some(None)` → NULL. `Some(Some(v))` → set.
#[derive(Debug, Default, Clone)]
pub struct StepExecutionPatch {
    pub status: Option<String>,
    pub cost_usd: Option<Option<f64>>,
    pub tokens: Option<Option<i64>>,
    pub wall_clock_secs: Option<Option<u64>>,
    /// Legacy single-path field. The repo adapter also writes the first
    /// entry of `artifact_paths` here when the latter is set, so older
    /// readers (gate UI, startup watchdog) keep seeing a primary path.
    /// New code should set `artifact_paths` and let the adapter keep
    /// `artifact_path` in sync.
    pub artifact_path: Option<Option<String>>,
    /// Replace the full artifact list. `None` → leave alone. `Some(vec)`
    /// → set to that vec (may be empty to clear). There is no
    /// `Some(None)`-means-NULL path: the column is NOT NULL DEFAULT '[]'
    /// so an empty list is the "no artifacts" representation.
    pub artifact_paths: Option<Vec<String>>,
    pub error_message: Option<Option<String>>,
    /// Bump the per-step retry counter. The driver uses this when
    /// following an `on_failure -> goto` edge. `None` = leave alone.
    pub iteration_count: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. MachineRepository
// ─────────────────────────────────────────────────────────────────────────────

/// Persistence for connection profiles (`Machine`) and the
/// per-machine `AgentProfile` records.
pub trait MachineRepository: Send + Sync {
    fn get_machines(&self) -> Result<Vec<Machine>, String>;
    /// Look up a single machine by id. Replaces the 8+ call sites that
    /// fetched the full list and `.find()`ed it themselves.
    fn get_machine(&self, id: &MachineId) -> Result<Option<Machine>, String>;
    fn add(&self, m: Machine) -> Result<(), String>;
    fn update(&self, m: Machine) -> Result<(), String>;
    fn delete(&self, id: &MachineId) -> Result<(), String>;

    fn get_agent_profiles(&self, machine_id: &MachineId) -> Result<Vec<AgentProfile>, String>;
    fn add_agent_profile(&self, profile: AgentProfile) -> Result<(), String>;
    fn delete_agent_profile(&self, id: &AgentProfileId) -> Result<(), String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. ThreadRepository
// ─────────────────────────────────────────────────────────────────────────────

/// Persistence for agent threads, the canonical message history, the
/// per-thread working memory, and the per-machine agent-config records.
pub trait ThreadRepository: Send + Sync {
    fn get_thread_sessions(&self, machine_id: &MachineId) -> Result<Vec<ThreadSession>, String>;
    fn get_thread_sessions_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> Result<Vec<ThreadSession>, String>;
    fn add_thread_session(&self, thread: ThreadSession) -> Result<(), String>;
    fn delete_thread_session(&self, id: &ThreadId) -> Result<(), String>;
    /// Apply a [`ThreadPatch`] to a single thread. The previous 3 separate
    /// methods (`update_thread_status`, `update_thread_model`,
    /// `update_thread_timestamp`) collapsed into this.
    fn update_thread(&self, id: &ThreadId, patch: &ThreadPatch) -> Result<(), String>;

    fn get_messages(&self, thread_id: &ThreadId) -> Result<Vec<Message>, String>;
    fn append_message(&self, msg: &Message) -> Result<(), String>;
    fn delete_messages(&self, thread_id: &ThreadId) -> Result<(), String>;

    /// Per-machine structured agent configuration. Reads return the
    /// migrated typed records; writes accept a JSON-encoded string for
    /// forward-compat.
    fn get_agent_configs(&self, machine_id: &MachineId) -> Result<Vec<AgentConfig>, String>;
    fn set_agent_configs(&self, machine_id: &MachineId, agents_json: &str) -> Result<(), String>;

    fn upsert_working_memory_entry(
        &self,
        thread_id: &ThreadId,
        entry: WorkingMemoryEntry,
    ) -> Result<(), String>;
    fn get_working_memory(&self, thread_id: &ThreadId) -> Result<Vec<WorkingMemoryEntry>, String>;
    fn clear_working_memory(&self, thread_id: &ThreadId) -> Result<(), String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. ProjectRepository
// ─────────────────────────────────────────────────────────────────────────────

/// Persistence for projects, the per-project repository list, and the
/// per-project settings (worktree strategy, conflict policy, …).
pub trait ProjectRepository: Send + Sync {
    fn get_projects(&self) -> Result<Vec<Project>, String>;
    /// Single-project lookup. Replaces the 8+ `get_projects()?.into_iter().find(...)` patterns.
    fn get_project(&self, id: &ProjectId) -> Result<Option<Project>, String>;
    fn add(&self, p: Project) -> Result<(), String>;
    fn update(&self, p: Project) -> Result<(), String>;
    fn update_status(&self, id: &ProjectId, status: &str) -> Result<(), String>;
    fn delete(&self, id: &ProjectId) -> Result<(), String>;
    /// Delete all `Repository` rows whose `project_id` matches. Used as
    /// a pre-step when re-saving project settings.
    fn delete_repositories_for(&self, project_id: &ProjectId) -> Result<(), String>;

    fn add_repository(&self, repo: Repository) -> Result<(), String>;
    fn get_repositories_for(&self, project_id: &ProjectId) -> Result<Vec<Repository>, String>;

    fn get_settings(&self, project_id: &ProjectId) -> Result<Option<ProjectSettings>, String>;
    fn save_settings(&self, settings: ProjectSettings) -> Result<(), String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. FeatureRepository
// ─────────────────────────────────────────────────────────────────────────────

/// Persistence for `Feature` (one feature per orchestrator run) and
/// the per-step `StepExecution` rows.
pub trait FeatureRepository: Send + Sync {
    fn get_active(&self, project_id: &ProjectId) -> Result<Vec<Feature>, String>;
    fn get(&self, id: &FeatureId) -> Result<Option<Feature>, String>;
    fn add(&self, f: Feature) -> Result<(), String>;
    /// Apply a [`FeaturePatch`] (replaces the 4-arg `update_feature_status`).
    fn update(&self, id: &FeatureId, patch: &FeaturePatch) -> Result<(), String>;
    /// Backfill a legacy feature that wasn't created with a workflow id.
    fn update_workflow_id(&self, id: &FeatureId, workflow_id: &WorkflowId) -> Result<(), String>;

    fn step_create(&self, step: StepExecution) -> Result<(), String>;
    fn step_get(&self, id: &StepExecutionId) -> Result<Option<StepExecution>, String>;
    /// Apply a [`StepExecutionPatch`] (replaces the 6-arg `step_execution_update_status`).
    fn step_update(&self, id: &StepExecutionId, patch: &StepExecutionPatch) -> Result<(), String>;
    fn steps_for_feature(&self, feature_id: &FeatureId) -> Result<Vec<StepExecution>, String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. WorkflowRepository
// ─────────────────────────────────────────────────────────────────────────────

/// Persistence for the workflow catalog (reusable templates) and its
/// immutable versioned snapshots.
pub trait WorkflowRepository: Send + Sync {
    fn get(&self, id: &WorkflowId) -> Result<Option<Workflow>, String>;
    fn list(&self) -> Result<Vec<Workflow>, String>;
    fn create(&self, w: Workflow) -> Result<(), String>;
    fn update_meta(&self, id: &WorkflowId, name: &str, description: &str) -> Result<(), String>;
    fn delete(&self, id: &WorkflowId) -> Result<(), String>;

    fn save_version(&self, v: WorkflowVersion) -> Result<(), String>;
    fn latest_version(&self, workflow_id: &WorkflowId) -> Result<Option<WorkflowVersion>, String>;
    fn versions(&self, workflow_id: &WorkflowId) -> Result<Vec<WorkflowVersion>, String>;
    /// Used by the first-launch seed step.
    fn count(&self) -> Result<u32, String>;
    fn update_schedule(
        &self,
        id: &WorkflowId,
        schedule: Option<WorkflowSchedule>,
    ) -> Result<(), String>;
    fn update_schedule_next_run(
        &self,
        id: &WorkflowId,
        next_run_at: Option<i64>,
    ) -> Result<(), String>;
    fn list_scheduled(&self) -> Result<Vec<Workflow>, String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. GateRepository
// ─────────────────────────────────────────────────────────────────────────────

/// Persistence for human-in-the-loop gate decisions (one row per
/// `gate` step execution).
pub trait GateRepository: Send + Sync {
    fn create(&self, g: GateDecision) -> Result<(), String>;
    fn decide(
        &self,
        step_execution_id: &StepExecutionId,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), String>;
    fn pending_for_feature(&self, feature_id: &FeatureId) -> Result<Option<GateDecision>, String>;
    fn latest_decided_for_feature(
        &self,
        feature_id: &FeatureId,
    ) -> Result<Option<GateDecision>, String>;
    /// Remove the gate decision row for a given step execution.
    /// Used when replaying from a gate step to clear its pending/decided state.
    fn reset_for_step_execution(&self, step_execution_id: &StepExecutionId) -> Result<(), String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. AppSettingsRepository
// ─────────────────────────────────────────────────────────────────────────────

/// Persistence for app-wide configuration: provider instances
/// (GitHub/GitLab connections), per-key UI-state KV, and first-launch
/// flags.
pub trait AppSettingsRepository: Send + Sync {
    fn add_provider_instance(&self, provider: ProviderInstance) -> Result<(), String>;
    fn get_provider_instances(&self) -> Result<Vec<ProviderInstance>, String>;
    fn delete_provider_instance(&self, id: &ProviderId) -> Result<(), String>;

    fn get_app_session(&self, key: &str) -> Result<Option<String>, String>;
    fn set_app_session(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete_app_session(&self, key: &str) -> Result<(), String>;

    fn app_setting_get(&self, key: &str) -> Result<Option<String>, String>;
    fn app_setting_set(&self, key: &str, value: &str) -> Result<(), String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. MergeAuditRepository
// ─────────────────────────────────────────────────────────────────────────────

pub trait MergeAuditRepository: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn record_merge_outcome(
        &self,
        subtask_run_id: &str,
        feature_id: &FeatureId,
        source_branch: &str,
        target_branch: &str,
        status: &str,
        merge_sha: Option<&str>,
        conflict_json: Option<&str>,
        now: i64,
    ) -> Result<(), String>;

    #[allow(clippy::too_many_arguments)]
    fn record_sync_outcome(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        default_branch: &str,
        status: &str,
        merge_sha: Option<&str>,
        conflict_json: Option<&str>,
        now: i64,
    ) -> Result<(), String>;

    fn lookup_worktree_context(
        &self,
        feature_id: &FeatureId,
        subtask_run_id: &str,
    ) -> Result<WorktreeContext, String>;

    fn lookup_repo_context(&self, feature_id: &FeatureId) -> Result<RepoContext, String>;

    fn get_last_sync_worktree_path(&self, feature_id: &FeatureId)
        -> Result<Option<String>, String>;

    fn skip_merge(&self, subtask_run_id: &str, reason: &str) -> Result<(), String>;
}

// Convenience unused-aliases to silence "unused" warnings for ID newtypes
// that appear in Patch struct docstrings but not the field list yet.
#[allow(dead_code)]
type _DocIdAliases = (MessageId, StepId, WorkflowVersionId, RepositoryId);
