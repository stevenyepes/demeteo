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
    pub workflow_id: Option<String>,
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
    /// Shell command to run the project's test suite (e.g. `cargo test`).
    #[serde(default)]
    pub test_command: Option<String>,
    /// Shell command to build the project (e.g. `cargo build --release`).
    /// Injected as `{{build_command}}` in prompt templates.
    #[serde(default)]
    pub build_command: Option<String>,
    /// Shell command to measure test coverage (e.g. `cargo tarpaulin`).
    /// Injected as `{{coverage_command}}` in prompt templates.
    #[serde(default)]
    pub coverage_command: Option<String>,
    /// Path to the project conventions file on the project host
    /// (e.g. `AGENTS.md`, `.cursor/rules/rules.md`, `CLAUDE.md`).
    /// Its content is injected as `{{project_conventions}}` in prompt templates.
    /// Auto-detected at bootstrap; user-editable in Project Settings.
    #[serde(default)]
    pub conventions_file: Option<String>,
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

// ─────────────────────────────────────────────────────────────────────────────
// Workflow catalog domain models (Phase R3)
// ─────────────────────────────────────────────────────────────────────────────

/// A versioned, reusable template that defines the steps to build a feature.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Starter-pack workflows shipped in the binary; cannot be deleted by the user.
    pub is_starter: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A single immutable snapshot of a workflow's step list.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowVersion {
    pub id: String,
    pub workflow_id: String,
    pub version: u32,
    /// JSON-serialised `Vec<StepConfig>`.
    pub steps_json: String,
    pub note: Option<String>,
    pub created_at: i64,
}

/// Configuration for one step in a workflow.
/// Stored as JSON inside `WorkflowVersion.steps_json`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StepConfig {
    /// Stable UUID — preserved across version bumps.
    pub id: String,
    /// "agent" | "parallel" | "gate"
    pub kind: String,
    pub title: String,
    /// Which agent to use. None = inherit from project's planner setting.
    /// One of: "opencode" | "hermes" | "claude-code" | "antigravity"
    pub agent_kind: Option<String>,
    /// Prompt template sent to the agent. May reference {{feature_description}}.
    pub prompt_template: Option<String>,
    /// "full" | "summary_only" | "none"
    pub artifact_mode: String,
    /// Step id to jump to on failure (loopback). None = abort feature.
    pub on_failure: Option<String>,
    /// Maximum loop iterations before the executor surfaces a gate instead.
    pub max_iterations: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Step execution domain models (Phase R4)
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime execution record for one step within a feature run.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StepExecution {
    pub id: String,
    pub feature_id: String,
    /// The stable step id from `StepConfig`.
    pub step_id: String,
    pub step_index: u32,
    /// "agent" | "parallel" | "gate"
    pub step_kind: String,
    /// pending | running | awaiting_gate | completed | failed | skipped | interrupted
    pub status: String,
    pub cost_usd: Option<f64>,
    pub wall_clock_secs: Option<u64>,
    /// Filesystem path of the artifact produced by this step.
    pub artifact_path: Option<String>,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A gate decision record — one row per gate step execution.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GateDecision {
    pub id: String,
    pub step_execution_id: String,
    /// None = pending. "approve" | "redirect" | "cancel"
    pub decision: Option<String>,
    /// Feedback / redirect instructions provided by the user.
    pub feedback: Option<String>,
    pub created_at: i64,
}
