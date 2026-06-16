use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Captured session info from the ACP `session/new` response.
/// Used by the frontend to display available modes, models, etc.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<SessionModeState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<ConfigOption>>,
    /// Raw JSON of the full session/new result so the frontend
    /// can access any future fields the agent sends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionModeState {
    #[serde(rename = "currentModeId")]
    pub current_mode_id: String,
    #[serde(rename = "availableModes")]
    pub available_modes: Vec<SessionModeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionModeInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(rename = "type", default)]
    pub option_type: String,
    #[serde(rename = "currentValue")]
    pub current_value: String,
    pub options: Vec<ConfigOptionValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOptionValue {
    pub value: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub auth_type: String, // 'key', 'password', 'agent', 'local'
    pub key_path: Option<String>,
    pub agents: Option<String>,               // JSON-encoded array of {kind, enabled} records
    pub auto_approved_rules: Option<String>,   // JSON-encoded array of auto-approved commands (regexes, legacy)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentProfile {
    pub id: String,
    pub machine_id: String,
    pub name: String,
    pub agent_type: String, // 'ollama', 'openai', 'cli', 'custom_http'
    pub command: Option<String>,
    pub work_dir: Option<String>,
    pub port: Option<i32>,
    pub ready_check: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatSession {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub sender: String, // 'user', 'agent'
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionHistory {
    pub id: String,
    pub machine_id: String,
    pub session_type: String, // 'terminal', 'agent'
    pub title: String,
    pub content: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ThreadSession {
    pub id: String,
    pub machine_id: String,
    pub title: String,
    pub mode: String, // 'worktree', 'adhoc'
    pub branch: Option<String>,
    pub repo_path: Option<String>,
    pub sandbox_path: Option<String>,
    pub status: String, // 'idle' | 'running' | 'pending_approval' | 'spawning' | 'installing' | 'error'
    pub agent_kind: Option<String>, // "opencode" | "hermes" | None
    #[serde(default)]
    pub model: Option<String>, // selected LLM model, persisted across session restarts
    pub updated_at: Option<i64>, // unix ms timestamp for sidebar ordering
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentConfig {
    pub kind: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkingMemoryEntry {
    pub file_path: String,
    pub line_count: Option<u32>,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<i64>,
    pub first_read_at: i64,
    pub last_read_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub role: String, // "user" | "assistant" | "system"
    pub content: String,
    pub metadata: Option<String>, // JSON
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderInstance {
    pub id: String,
    pub kind: String, // 'github' | 'gitlab'
    pub host: String,
    pub username: String,
    pub avatar_url: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub compute_type: String, // 'local' | 'remote'
    pub remote_host: Option<String>,
    pub status: String,
    pub nodes: i32,
    pub spend: f64,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Repository {
    pub id: String,
    pub project_id: String,
    pub provider_id: String,
    pub repo_path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Feature {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub status: String,
    pub total_cost: f64,
    pub duration: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorktreeStrategy {
    pub default_branch: String,
    pub branch_prefix: String,
    pub test_command: Option<String>,
    pub pr_template: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectSettings {
    pub project_id: String,
    pub worktree_strategy: WorktreeStrategy,
    pub conflict_policy: String,
    pub feature_lifecycle: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: Option<String>,
    pub is_locked: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoHealthStatus {
    pub repo_path: String,     // logical path e.g. "org/repo"
    pub is_cloned: bool,
    pub head_branch: Option<String>,
    pub worktrees: Vec<WorktreeInfo>,
    pub has_uncommitted: bool,
    pub has_unpushed: bool,
}
