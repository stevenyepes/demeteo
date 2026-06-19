use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::artifact::{ArtifactDecl, ArtifactMode};
use super::ids::{
    AgentProfileId, FeatureId, GateDecisionId, MachineId, MessageId, ProjectId, ProviderId,
    RepositoryId, StepExecutionId, StepId, ThreadId, WorkflowId, WorkflowVersionId,
};

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

fn deserialize_lax_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(deserializer)?;
    match val {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        _ => Ok(val.to_string()),
    }
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
    #[serde(rename = "currentValue", deserialize_with = "deserialize_lax_string")]
    pub current_value: String,
    #[serde(default)]
    pub options: Vec<ConfigOptionValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOptionValue {
    #[serde(deserialize_with = "deserialize_lax_string")]
    pub value: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Machine {
    pub id: MachineId,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub auth_type: String, // 'key', 'password', 'agent', 'local'
    pub key_path: Option<String>,
    pub agents: Option<String>,               // JSON-encoded array of {kind, enabled} records
    pub auto_approved_rules: Option<String>,   // JSON-encoded array of auto-approved commands (regexes, legacy)
    #[serde(default)]
    pub use_login_shell: Option<bool>,         // null/false = no login shell; true = bash -l -c
    #[serde(default)]
    pub setup_commands: Option<String>,        // JSON array of shell commands run after clone
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentProfile {
    pub id: AgentProfileId,
    pub machine_id: MachineId,
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
    pub machine_id: MachineId,
    pub session_type: String, // 'terminal', 'agent'
    pub title: String,
    pub content: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ThreadSession {
    pub id: ThreadId,
    pub machine_id: MachineId,
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
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub role: String, // "user" | "assistant" | "system"
    pub content: String,
    pub metadata: Option<String>, // JSON
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderInstance {
    pub id: ProviderId,
    pub kind: String, // 'github' | 'gitlab'
    pub host: String,
    pub username: String,
    pub avatar_url: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub compute_type: String, // 'local' | 'remote'
    pub remote_host: Option<MachineId>,
    pub status: String,
    pub nodes: i32,
    pub spend: f64,
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
pub struct Feature {
    pub id: FeatureId,
    pub project_id: ProjectId,
    pub workflow_id: Option<WorkflowId>,
    pub title: String,
    pub status: String,
    pub total_cost: f64,
    pub duration: String,
    pub created_at: i64,
    #[serde(default)]
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// URL of the published PR/MR, if any. Set by the `MrPublisher`.
    #[serde(default)]
    pub mr_url: Option<String>,
    /// State of the PR/MR on the provider: `none|draft|open|merged|closed`.
    #[serde(default)]
    pub mr_state: Option<String>,
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
    pub project_id: ProjectId,
    pub worktree_strategy: WorktreeStrategy,
    pub conflict_policy: String,
    pub feature_lifecycle: String,
    #[serde(default)]
    pub default_agent_kind: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
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
    pub id: WorkflowId,
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
    pub id: WorkflowVersionId,
    pub workflow_id: WorkflowId,
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
    pub id: StepId,
    /// "agent" | "parallel" | "gate"
    pub kind: String,
    pub title: String,
    /// Which agent to use. None = inherit from project's planner setting.
    /// One of: "opencode" | "hermes" | "claude-code" | "antigravity"
    pub agent_kind: Option<String>,
    /// Prompt template sent to the agent. May reference {{feature_description}}.
    pub prompt_template: Option<String>,
    /// Default persistence mode for this step's artifacts. The per-artifact
    /// `ArtifactDecl.mode` overrides this when set; this field is what the
    /// workflow editor's dropdown binds to.
    pub artifact_mode: String,
    /// Step id to jump to on failure (loopback). None = abort feature.
    pub on_failure: Option<StepId>,
    /// Maximum loop iterations before the executor surfaces a gate instead.
    pub max_iterations: Option<u32>,
    /// Per-step artifact contract. The executor resolves these at
    /// `TurnComplete` against the events the agent emitted, computes
    /// derived artifacts (diffs, worktree pointers), and writes the
    /// resulting references to `step_execution.artifact_paths`.
    ///
    /// `None` (JSON `null`) or `Some([])` is the legacy backstop: the
    /// executor falls back to the chat-stream-as-artifact path so old
    /// workflows keep running during migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<ArtifactDecl>>,
}

impl StepConfig {
    /// The effective per-step default mode as a typed enum.
    pub fn artifact_mode_typed(&self) -> ArtifactMode {
        ArtifactMode::from_str_loose(&self.artifact_mode)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Step execution domain models (Phase R4)
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime execution record for one step within a feature run.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StepExecution {
    pub id: StepExecutionId,
    pub feature_id: FeatureId,
    /// The stable step id from `StepConfig`.
    pub step_id: StepId,
    pub step_index: u32,
    /// "agent" | "parallel" | "gate"
    pub step_kind: String,
    /// pending | running | awaiting_gate | completed | failed | skipped | interrupted
    pub status: String,
    pub cost_usd: Option<f64>,
    pub wall_clock_secs: Option<u64>,
    /// Legacy single-path field. Kept so existing readers (gate display,
    /// pre-refactor tests, the startup watchdog) still see a sensible
    /// primary path. New code should prefer [`Self::artifact_paths`].
    /// Populated by the executor as the first entry of `artifact_paths`
    /// when the latter is non-empty; cleared when the latter is empty.
    #[serde(default)]
    pub artifact_path: Option<String>,
    /// Ordered list of artifact references produced by this step.
    /// Stored as a JSON-encoded TEXT column on `step_executions` (V5
    /// migration). Each entry is an `ArtifactStore` reference
    /// (filesystem path for the FS adapter) that the next step's
    /// prompt renderer and the UI's `ArtifactViewer` can resolve.
    #[serde(default)]
    pub artifact_paths: Vec<String>,
    pub error_message: Option<String>,
    /// How many times the executor has entered this step via a
    /// `on_failure -> goto` edge. Persists across executor restarts so
    /// the `max_iterations` budget can't be reset by relaunching.
    #[serde(default)]
    pub iteration_count: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A gate decision record — one row per gate step execution.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GateDecision {
    pub id: GateDecisionId,
    pub step_execution_id: StepExecutionId,
    /// None = pending. "approve" | "redirect" | "cancel"
    pub decision: Option<String>,
    /// Feedback / redirect instructions provided by the user.
    pub feedback: Option<String>,
    pub created_at: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase R6 — merge / conflict / publish domain models
// ─────────────────────────────────────────────────────────────────────────────

/// One row per `parallel`-step subtask execution. Linked back to the
/// `step_executions` row that spawned it.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubtaskRun {
    pub id: String,
    pub feature_id: FeatureId,
    pub step_execution_id: StepExecutionId,
    /// The planner-assigned subtask id ("sub-1", "sub-2", …).
    pub subtask_id: String,
    /// The agent session that ran the subtask, if any. Set after
    /// the agent spawns; cleared on teardown so the audit trail
    /// doesn't pin a stale handle.
    pub agent_id: Option<String>,
    pub worktree_path: String,
    pub branch: String,
    /// pending | running | completed | failed | skipped
    pub status: String,
    pub cost_usd: f64,
    pub error_message: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// One row per attempt to merge a `SubtaskRun` back into the feature
/// branch. Conflict reports and resolution attempts are recorded
/// here so the user can see what happened even after the driver
/// moves on.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubtaskMerge {
    pub id: String,
    pub subtask_run_id: String,
    pub feature_id: FeatureId,
    pub source_branch: String,
    pub target_branch: String,
    /// pending | ok | conflict | skipped | aborted
    pub status: String,
    pub merge_commit_sha: Option<String>,
    /// JSON-encoded [`ConflictReport`] when `status == "conflict"`.
    pub conflict_report: Option<String>,
    pub resolution_attempts: i32,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

/// Result of a successful merge. `Ok` from [`MergeExecutor::merge_subtask_into_feature`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MergeOutcome {
    pub merge_commit_sha: String,
    pub target_branch: String,
    pub source_branch: String,
}

/// One file in a conflict set. Path is repo-relative.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictFile {
    pub path: String,
    /// Short one-line summary ("both modified", "deleted by us",
    /// "deleted by them", "added by both", …).
    pub kind: String,
}

/// `git merge` / `git rebase` returned this — the merge executor
/// surfaces it so the conflict resolver cascade has structured
/// data to work with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictReport {
    pub source_branch: String,
    pub target_branch: String,
    pub files: Vec<ConflictFile>,
    /// Raw stderr from the failing git invocation. Useful for the
    /// manual-resolution UI ("look at the actual git error").
    pub raw_error: String,
    /// Detected at: ms-since-epoch. Helps the UI render "X minutes ago".
    pub detected_at: i64,
}

/// Per-project setting that controls how a merge conflict is
/// resolved. Mirrors the dropdown in `ProjectSettings`'s
/// "Conflict Resolution Policy" field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Always surface a gate; never auto-merge.
    AlwaysGate,
    /// Try the auto-agent first; cascade to manual on failure.
    AutoAgent,
    /// Skip the auto-agent; immediately open the manual UI.
    AutoHuman,
}

impl ConflictPolicy {
    pub fn from_db(s: &str) -> Self {
        match s {
            "auto_agent" => ConflictPolicy::AutoAgent,
            "auto_human" => ConflictPolicy::AutoHuman,
            _ => ConflictPolicy::AlwaysGate,
        }
    }
}

/// Options for [`MrPublisher::publish_mr`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishOptions {
    /// True → open as draft. False → open as ready-for-review.
    #[serde(default)]
    pub draft: bool,
    /// Optional PR/MR title override; defaults to the feature title.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional body override; defaults to the rendered PR template.
    #[serde(default)]
    pub body: Option<String>,
    /// Optional base branch override; defaults to the project's
    /// `default_branch` setting.
    #[serde(default)]
    pub target_branch: Option<String>,
}

/// Returned by [`MrPublisher::publish_mr`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrInfo {
    pub url: String,
    /// Provider's state machine: "draft" | "open" | "merged" | "closed".
    pub state: String,
    pub number: u64,
    pub provider_kind: String,
    pub provider_host: String,
}
