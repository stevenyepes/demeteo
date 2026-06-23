//! Shared application state passed to every Tauri command.
//!
//! `AppContext` is the single bag of dependency-injection ports that
//! commands reach into. Before this struct existed, every command had
//! to extract 2–4 separate `State<'_, *State>` extractors (one per
//! port), which forced each command body to re-assemble the ports it
//! needed and made the dependency graph of the app invisible from
//! `lib.rs`.
//!
//! In PR3 the single `db: Arc<dyn DatabasePort>` field was replaced
//! with seven narrow sub-ports aligned with the bounded contexts:
//! machines, threads, projects, features, workflows, gates, and app
//! settings. See `ports::db` for the trait definitions.
//!
//! `SessionState` (in `terminal.rs`) and `ForwardState` (in
//! `forward.rs`) are kept distinct because they hold *session-specific*
//! state (active SSH sessions, port forwards), not
//! dependency-injection ports.

use crate::adapters::agent::registry::AgentRegistry;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::db::{
    AppSettingsRepository, FeatureRepository, GateRepository, MachineRepository,
    MergeAuditRepository, NotificationRepository, ProjectRepository, ThreadRepository,
    WorkflowRepository,
};
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::mr_publisher::MrPublisher;
use crate::ports::notification::NotificationPort;
use crate::ports::pricing::PricingTable;
use crate::ports::provider_http::ProviderHttpPort;
use crate::ports::step_executor::{GatePresenter, StepExecutor};
use crate::ports::worktree_ops::WorktreeOpsPort;
use serde::Serialize;
use std::sync::Arc;

/// The single bag of ports every Tauri command can depend on.
///
/// Construction happens once in `lib.rs::run()` (the Tauri setup hook),
/// after every concrete adapter is built. Commands take a
/// `State<'_, AppContext>` and pull only the sub-ports they actually
/// use, keeping the dependency on each port visible at the call site
/// (`ctx.machines`, `ctx.projects`, …) instead of hidden behind five
/// separately named extractors.
pub struct AppContext {
    /// Machine + agent profile persistence.
    pub machines: Arc<dyn MachineRepository>,
    /// Thread + message + working memory + agent config persistence.
    pub threads: Arc<dyn ThreadRepository>,
    /// Project + repository + project settings persistence.
    pub projects: Arc<dyn ProjectRepository>,
    /// Feature + step execution persistence.
    pub features: Arc<dyn FeatureRepository>,
    /// Workflow + workflow version persistence.
    pub workflows: Arc<dyn WorkflowRepository>,
    /// Gate decision persistence.
    pub gates: Arc<dyn GateRepository>,
    /// App-wide settings: provider instances, app-session KV, first-launch flags.
    pub app_settings: Arc<dyn AppSettingsRepository>,
    /// Project memory persistence.
    pub memory: Arc<dyn crate::ports::memory::ProjectMemoryPort>,
    /// Merge audit persistence.
    pub merge_audit: Arc<dyn MergeAuditRepository>,
    /// In-app notification bell persistence. Written by the
    /// background MR-state monitor and read by `commands::notifications`.
    pub notifications: Arc<dyn NotificationRepository>,

    /// Process + filesystem execution port (local subprocess or remote SSH).
    pub exec: Arc<dyn ExecutionPort>,

    /// Policy-enforced execution port for agent-originated actions.
    pub agent_exec: Arc<dyn AgentExecutionPort>,

    /// UI notification port (Tauri event emitter).
    pub notif: Arc<dyn NotificationPort>,

    /// Agent runtime registry (opencode, hermes, claude-code, …).
    pub registry: Arc<AgentRegistry>,

    /// Step executor (DAG engine that drives a `Feature` through its workflow).
    pub executor: Arc<dyn StepExecutor>,

    /// Gate presenter (read-side of gate decisions).
    pub presenter: Arc<dyn GatePresenter>,

    /// Model → USD pricing (used to backfill per-step `cost_usd` when the
    /// agent's `Usage` event doesn't carry it).
    pub pricing: Arc<dyn PricingTable>,

    /// MR/PR publisher (GitHub + GitLab). Wired through `AppContext`
    /// so the orchestrator can publish from any code path without
    /// threading the port through every layer.
    pub mr_publisher: Arc<dyn MrPublisher>,

    /// Merge executor — wraps `git merge` for both subtask→feature
    /// and feature→upstream flows with structured conflict detection
    /// and an audit trail.
    pub merge_executor: Arc<dyn MergeExecutor>,

    /// Worktree operations (cloning, provisioning, status, branch delete, etc.).
    pub worktree_ops: Arc<dyn WorktreeOpsPort>,

    /// Provider HTTP operations (validation, list repos).
    pub provider_http: Arc<dyn ProviderHttpPort>,

    /// Path to application local data directory.
    pub app_data_dir: std::path::PathBuf,
}

pub const EVENT_THREAD_STATUS_CHANGED: &str = "thread_status_changed";
pub const EVENT_AGENT_EVENT: &str = "agent_event";

#[derive(Serialize, Clone)]
pub struct ThreadStatusChanged {
    pub thread_id: String,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct AgentConfigView {
    pub kind: String,
    pub enabled: bool,
    pub available: bool,
    pub install_command: String,
}
