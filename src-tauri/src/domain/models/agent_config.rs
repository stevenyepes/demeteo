use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: Option<String>,
    pub is_locked: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoHealthStatus {
    pub repo_path: String, // logical path e.g. "org/repo"
    pub is_cloned: bool,
    pub head_branch: Option<String>,
    pub worktrees: Vec<WorktreeInfo>,
    pub has_uncommitted: bool,
    pub has_unpushed: bool,
}
