use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId, WorkflowId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub compute_type: String, // 'local' | 'remote'
    pub remote_host: Option<MachineId>,
    pub status: String,
    pub nodes: i32,
    pub spend: f64,
    #[serde(default)]
    pub tokens: i64,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Repository {
    pub id: RepositoryId,
    pub project_id: ProjectId,
    pub provider_id: ProviderId,
    pub repo_path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorktreeStrategy {
    pub default_branch: String,
    pub branch_prefix: String,
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default)]
    pub build_command: Option<String>,
    #[serde(default)]
    pub coverage_command: Option<String>,
    #[serde(default)]
    pub conventions_file: Option<String>,
    pub pr_template: Option<String>,
    #[serde(default)]
    pub harnesses: Option<HashMap<String, String>>,
    /// Project-wide writability exceptions, applied on top of the
    /// capability-driven chmod fence. Repo-relative paths the agent may
    /// write to even when the step's capability (`ReadOnly`,
    /// `Artifacts`, `Verify`) would otherwise fence them. Designed for
    /// tool side-effects that aren't source or artifacts — e.g.
    /// `target/` for `cargo test`, `node_modules/` for `npm test`,
    /// `.venv/` for `pytest`. Each entry must be a relative path
    /// inside the worktree; `..` is rejected to prevent escape.
    /// Stays empty for `Implement` capability (which is already
    /// fully writable). See scope adapter `derive_writable_paths_for_scope`.
    #[serde(default)]
    pub extra_writable_paths: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectSettings {
    pub project_id: ProjectId,
    pub worktree_strategy: WorktreeStrategy,
    pub conflict_policy: String,
    pub feature_lifecycle: String,
    #[serde(default)]
    pub default_agent_kind: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    /// Project-level default loop iteration budget for `on_failure` retry
    /// loops. `None` = use the engine default (3). Overridable per run via
    /// `Feature::loop_iterations`. See migration V13.
    #[serde(default)]
    pub default_loop_iterations: Option<u32>,
    /// Repo-relative folder where agents write their reports
    /// (`research-report.md`, `critic-review.md`, …). The orchestrator
    /// injects `{{artifact_dir}}` into every step's prompt and excludes
    /// this folder from `commit_worktree_changes` unless
    /// `commit_artifacts` is true. Default: `"artifacts/"`.
    /// See migration V12 and AGENTS.md §6.
    #[serde(default = "default_artifact_subdir")]
    pub artifact_subdir: String,
    /// When false (default), the orchestrator's
    /// `commit_worktree_changes` runs `git add -A -- ':!<artifact_subdir>'`
    /// so the reports stay in the worktree as untracked files instead of
    /// being committed into the feature branch. The reports' content is
    /// still captured into the `FsArtifactStore` for the UI.
    /// Per-feature override lives on `Feature::commit_artifacts`.
    #[serde(default)]
    pub commit_artifacts: bool,
}

fn default_artifact_subdir() -> String {
    "artifacts/".to_string()
}

/// A project-scoped override of the coding agent ("harness") and/or model
/// for a (global) workflow — either the whole workflow or a single step.
/// Persisted in `project_workflow_overrides` (migrations V14 / V15).
///
/// Scope is set by `step_id`:
///   - `None` → workflow-level. At feature start it overlays the project
///     defaults (`ProjectSettings::default_agent_kind` / `default_model`) for
///     this workflow only.
///   - `Some(step_id)` → step-level. It is baked onto the matching
///     `StepConfig`, so it beats the workflow author's value for that step.
///
/// In all cases it still loses to a run-time override (feature-wide or
/// per-step, chosen at launch). `None` on a field = inherit for that field.
/// See `resolve_execution_context`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectWorkflowOverride {
    pub project_id: ProjectId,
    pub workflow_id: WorkflowId,
    /// `None` = workflow-level (stored as `''`); `Some` targets one step.
    #[serde(default)]
    pub step_id: Option<String>,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishOptions {
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub target_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrInfo {
    pub url: String,
    pub state: String,
    pub number: u64,
    pub provider_kind: String,
    pub provider_host: String,
}
