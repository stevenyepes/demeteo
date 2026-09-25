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
    /// The user's **ordered selection of which harnesses gate validation** —
    /// tier 2 of [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses).
    ///
    /// It exists because `harnesses` was otherwise dead config: the only thing
    /// that selected an entry was a per-step `verifier.harness_names`, and all
    /// seven shipped starters declare none — so a user could add
    /// `lint → npm run lint`, see it accepted, and nothing would ever run it
    /// short of forking a starter. Ticking it here gates every workflow that
    /// declares nothing of its own.
    ///
    /// Ordered because `harnesses` is a `HashMap` and therefore has no order to
    /// inherit, while the order is the user's (cheap gates first — lint before
    /// integration). Names that no longer exist in `harnesses` are ignored at
    /// resolution rather than erroring.
    ///
    /// `None`/empty = no selection, which resolves exactly as it does today.
    /// Persisted inside the `harnesses` column — see
    /// `adapters/database/repos/project.rs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_gates: Option<Vec<String>>,
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
    /// The Workflow a new Feature starts with unless the launcher picks
    /// another. `None` = **unset**, not "no workflow": the launch path then
    /// falls back to the first entry of the workflow list, which is what
    /// every project did implicitly before this field existed.
    ///
    /// Read at launch time only. Unlike `default_effort` and `default_model`
    /// it feeds no resolution chain during a run, and a workflow deleted
    /// after being chosen leaves this id dangling by design — the caller
    /// resolves it against the workflow list and treats an unresolvable id
    /// as unset. See migration V40 for why there is no foreign key.
    #[serde(default)]
    pub default_workflow_id: Option<String>,
    /// Project-level default loop iteration budget for `on_failure` retry
    /// loops. `None` = use the engine default (3). Overridable per run via
    /// `Feature::loop_iterations`. See migration V13.
    #[serde(default)]
    pub default_loop_iterations: Option<u32>,
    /// Project-level default per-turn dollar budget (`--max-budget-usd`).
    /// `None` = use the engine default
    /// ([`crate::domain::agent_session::budget::DEFAULT_MAX_BUDGET_USD`]).
    /// Overridable per run via
    /// `Feature::max_budget_usd`. See migration V30.
    #[serde(default)]
    pub default_max_budget_usd: Option<f64>,
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
    /// The command a reviewing step should start from, as the user wrote it.
    /// Bound to `{{review_entrypoint}}` and carried into the prompt untouched —
    /// [`review_entrypoint`](crate::domain::review_entrypoint) holds why it is
    /// never wrapped, and why `None` and `""` are the same answer. See
    /// migration V42.
    #[serde(default)]
    pub review_entrypoint: Option<String>,
    /// The harness a conflict-resolution turn should run under, outranking the
    /// run's own launch pin for that turn alone.
    /// [`sync_resolver`](crate::domain::sync_resolver) holds why a role gets a
    /// tier above the run, and what `None` falls through to. See migration V44.
    #[serde(default)]
    pub sync_resolver_agent_kind: Option<String>,
    /// The model for that turn, inherited independently of the harness.
    #[serde(default)]
    pub sync_resolver_model: Option<String>,
    /// The reasoning effort for that turn. Clamped per harness at spawn, so a
    /// level the chosen harness does not offer is never emitted.
    #[serde(default)]
    pub sync_resolver_effort: Option<EffortLevel>,
    /// Whether a resolved sync waits for a human before it is published.
    /// `None` is "no opinion", which is not `false` —
    /// [`publish_policy`](crate::domain::sync_session::publish_policy) holds
    /// what it resolves to and why the setting can only turn review off. See
    /// migration V45.
    #[serde(default)]
    pub sync_review_before_push: Option<bool>,
}

fn default_artifact_subdir() -> String {
    "artifacts/".to_string()
}

/// A subset of `ProjectSettings` writable by a non-UI external caller (the
/// `agent_surface` seam) — run shape only, never a spend or safety boundary.
///
/// Excluded on purpose, and not by omission:
/// - `default_max_budget_usd`, `default_loop_iterations`: per-run cost
///   ceilings. A caller that could raise either is raising the dollar/retry
///   bound on every run it subsequently causes — the permission split is
///   meaningless if the same call can both start a run and widen its own
///   leash.
/// - `sync_review_before_push`: switches off the human review gate before a
///   push. Same shape of boundary as budget: a caller must not be able to
///   remove the check on its own output.
/// - `worktree_strategy`, `feature_lifecycle`: project topology, not run
///   shape — set once when the project is configured, not per launch.
///
/// Rejected: sharing `ProjectSettings` whole with the UI's save path
/// (`save_project_settings`). The UI is a human looking at a labelled
/// control; this caller is not. A shared path would grant budget and
/// review-bypass changes under a call that reads like "save my
/// preferences."
///
/// A field added to `ProjectSettings` later is unreachable here until
/// someone deliberately adds it below, having decided it is run-shape and
/// not a boundary — that decision is the point of this being a type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunShapePatch {
    #[serde(default)]
    pub default_agent_kind: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub default_effort: Option<EffortLevel>,
    #[serde(default)]
    pub default_workflow_id: Option<String>,
    #[serde(default = "default_artifact_subdir")]
    pub artifact_subdir: String,
    #[serde(default)]
    pub commit_artifacts: bool,
    #[serde(default)]
    pub sync_resolver_agent_kind: Option<String>,
    #[serde(default)]
    pub sync_resolver_model: Option<String>,
    #[serde(default)]
    pub sync_resolver_effort: Option<EffortLevel>,
}

/// Why a [`RunShapePatch`] was refused before it reached storage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunShapePatchError {
    #[error(
        "artifact_subdir must be a relative path of letters, digits, spaces and `_-./`, \
         with no `.` or `..` segments"
    )]
    ArtifactSubdir,
    #[error("{0} must not start with `-` or contain whitespace or control characters")]
    Identifier(&'static str),
}

impl RunShapePatch {
    /// A typed patch is only as safe as its field types, and these are all
    /// `String`. `artifact_subdir` ends up inside a `git add` pathspec and an
    /// artifact-scope path; the agent/model/workflow fields end up as argv
    /// values of a spawned harness and as lookup keys. None of them is a shell
    /// sink today (the sink escapes — see `resolve_add_exclusions`), so this is
    /// the second fence, not the first: an external caller gets a refusal
    /// instead of a value that later runs are left to survive.
    pub fn validate(&self) -> Result<(), RunShapePatchError> {
        let subdir =
            crate::domain::staged_deliverable::normalize_artifact_subdir(&self.artifact_subdir);
        let subdir_ok = subdir.is_empty()
            || (!subdir.starts_with('/')
                && subdir.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.' | '/')
                })
                && subdir
                    .split('/')
                    .all(|seg| !seg.is_empty() && seg != "." && seg != ".."));
        if !subdir_ok {
            return Err(RunShapePatchError::ArtifactSubdir);
        }

        let identifiers = [
            ("default_agent_kind", &self.default_agent_kind),
            ("default_model", &self.default_model),
            ("default_workflow_id", &self.default_workflow_id),
            ("sync_resolver_agent_kind", &self.sync_resolver_agent_kind),
            ("sync_resolver_model", &self.sync_resolver_model),
        ];
        for (field, value) in identifiers {
            if let Some(v) = value {
                if v.starts_with('-') || v.chars().any(|c| c.is_whitespace() || c.is_control()) {
                    return Err(RunShapePatchError::Identifier(field));
                }
            }
        }
        Ok(())
    }
}

/// Copies `patch`'s nine fields onto `settings`, returning every other
/// field unchanged. See [`RunShapePatch`] for which fields those are and why.
pub fn apply_run_shape_patch(
    mut settings: ProjectSettings,
    patch: RunShapePatch,
) -> ProjectSettings {
    settings.default_agent_kind = patch.default_agent_kind;
    settings.default_model = patch.default_model;
    settings.default_effort = patch.default_effort;
    settings.default_workflow_id = patch.default_workflow_id;
    settings.artifact_subdir = patch.artifact_subdir;
    settings.commit_artifacts = patch.commit_artifacts;
    settings.sync_resolver_agent_kind = patch.sync_resolver_agent_kind;
    settings.sync_resolver_model = patch.sync_resolver_model;
    settings.sync_resolver_effort = patch.sync_resolver_effort;
    settings
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
/// In all cases it still loses to a run-time override on the feature row —
/// the feature-wide pair chosen at launch, and above it the per-step
/// [`StepOverride`](crate::domain::models::StepOverride), which is writable
/// for as long as the run is alive. `None` on a field = inherit for that field.
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

#[cfg(test)]
#[path = "../../../tests/domain/models/project.rs"]
mod tests;
