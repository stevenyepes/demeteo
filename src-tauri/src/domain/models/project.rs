use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
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
