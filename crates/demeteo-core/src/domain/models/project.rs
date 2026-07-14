use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId, WorkflowId};
use crate::domain::models::EffortLevel;
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
    /// Optional shell command run inside each subtask worktree before the
    /// verifier's harness command (`npm ci`, `cargo fetch`, `prisma
    /// generate`, a DB migration, …). Runs after write permissions are
    /// restored and after `provision_subtask_worktree`'s dependency-cache
    /// symlinking, so it only needs to handle what symlinking a prior
    /// install can't — codegen, migrations, freshly-added dependencies.
    /// `None` (default) skips this step entirely.
    #[serde(default)]
    pub prepare_command: Option<String>,
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
    /// Project-wide default effort for every step of every run. `None` = fall
    /// through to [`EffortLevel::DEFAULT`] (high). The lowest tier of the
    /// effort resolution chain; a workflow, a launch override, or a project
    /// workflow override all beat it. See migration V29.
    #[serde(default)]
    pub default_effort: Option<EffortLevel>,
    /// Project-level default loop iteration budget for `on_failure` retry
    /// loops. `None` = use the engine default (3). Overridable per run via
    /// `Feature::loop_iterations`. See migration V13.
    #[serde(default)]
    pub default_loop_iterations: Option<u32>,
    /// Repo-relative folder where agents write their reports
    /// (`research-report.md`, `critic-review.md`, …). The orchestrator
    /// injects `{{report_dir}}` (alias `{{artifact_dir}}`, kept for
    /// back-compat with older workflows) into every step's prompt and
    /// excludes this folder from `commit_worktree_changes` unless
    /// `commit_artifacts` is true. Default: `"artifacts/"`.
    /// See migration V12 and AGENTS.md §6.
    ///
    /// Note for `wf-starter-docs-update`: the docs-update workflow's
    /// `s-draft` and `s-polish` steps are explicitly told (in their
    /// `prompt_template`) to write the real doc body at the path the
    /// survey/gate approved (typically under `docs/`) and to use
    /// `{{report_dir}}` ONLY for the short change-summary report.
    /// That separation is what keeps a "create a new doc explaining
    /// feature X" feature from silently landing its body under
    /// `artifacts/s-draft.md` (which `commit_artifacts=false` would
    /// keep off the branch). The StartFeatureModal's advanced
    /// section surfaces this toggle so users can opt a single
    /// docs-update feature into committing its reports alongside the
    /// new doc.
    #[serde(default = "default_artifact_subdir")]
    pub artifact_subdir: String,
    /// When false (default), the orchestrator's
    /// `commit_worktree_changes` runs `git add -A -- ':!<artifact_subdir>'`
    /// so the reports stay in the worktree as untracked files instead of
    /// being committed into the feature branch. The reports' content is
    /// still captured into the `FsArtifactStore` for the UI.
    /// Per-feature override lives on `Feature::commit_artifacts`.
    ///
    /// Note for `wf-starter-docs-update`: leave this `false` (the
    /// default) for the "create a new doc" case so the new doc body
    /// at its real `docs/...` path lands on the branch while the
    /// `artifacts/s-draft.md` summary stays out. Flip it to `true`
    /// (per-feature via the StartFeatureModal advanced section) if
    /// the user wants both the doc and the change-summary report in
    /// the same commit.
    ///
    /// Note for remote runner runs (C4.4): the default `false` used to be
    /// a silent data-loss trap for remote features — the PR was the only
    /// channel back to the laptop, so uncommitted reports were invisible.
    /// That no longer holds: C4.2's shadow mirror pulls each remote step's
    /// declared artifacts into the laptop `FsArtifactStore` and `RunView`
    /// renders them via the return inbox's "View feature". So `false`
    /// stays the clean default for remote too (reports reach the laptop
    /// through the mirror, not the branch); `true` is now a deliberate
    /// "also commit them to the PR" opt-in, not the only way to see them.
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
    #[serde(default)]
    pub effort: Option<EffortLevel>,
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
