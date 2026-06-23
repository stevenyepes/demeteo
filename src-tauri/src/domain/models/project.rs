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
