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
use crate::ports::attachment_store::{AttachmentJsonPort, AttachmentStore};
use crate::ports::db::{
    AppSettingsRepository, FeatureRepository, GateRepository, MachineRepository,
    NotificationRepository, ProjectRepository, SequenceResumeRepository, ThreadRepository,
    WorkflowRepository,
};
use crate::ports::execution::ExecutionPort;
use crate::ports::mr_publisher::MrPublisher;
use crate::ports::notification::NotificationPort;
use crate::ports::pricing::PricingTable;
use crate::ports::provider_http::ProviderHttpPort;
use crate::ports::remote_run_mirror::RemoteRunMirrorPort;
use crate::ports::run_events::RunEventsPort;
use crate::ports::runner_run::RunnerRunPort;
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
    /// Durable `sequence`-step resume state: the crash checkpoint and the
    /// plan cache, keyed per (feature, node).
    pub sequence_resume: Arc<dyn SequenceResumeRepository>,
    /// Workflow + workflow version persistence.
    pub workflows: Arc<dyn WorkflowRepository>,
    /// Gate decision persistence.
    pub gates: Arc<dyn GateRepository>,
    /// App-wide settings: provider instances, app-session KV, first-launch flags.
    pub app_settings: Arc<dyn AppSettingsRepository>,
    /// Project memory persistence.
    pub memory: Arc<dyn crate::ports::memory::ProjectMemoryPort>,
    /// Memory signal queue (run observations awaiting distillation).
    pub signals: Arc<dyn crate::ports::memory_signals::MemorySignalsPort>,
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

    /// Worktree operations (cloning, provisioning, status, branch delete, etc.).
    pub worktree_ops: Arc<dyn WorktreeOpsPort>,

    /// Provider HTTP operations (validation, list repos).
    pub provider_http: Arc<dyn ProviderHttpPort>,

    /// Memory agent LLM port (chat + embeddings against a user-configured
    /// OpenAI-compatible endpoint). The one deliberate direct-to-provider call
    /// path, scoped to the memory feature.
    pub memory_llm: Arc<dyn crate::ports::memory_llm::MemoryLlmPort>,

    /// Per-feature user attachment store (images, files). Files live
    /// under `<app_local_data_dir>/attachments/<feature_id>/<sha256>.<ext>`;
    /// the on-disk store is the source of truth for the bytes, while
    /// the JSON manifest on the `Feature` row (see migration V19 and
    /// `Feature::attachments`) is the source of truth for which
    /// attachments belong to which feature.
    pub attachments: Arc<dyn AttachmentStore>,

    /// JSON-manifest persistence for the per-feature attachment
    /// list (the column `features.attachments_json`, migration V19).
    /// Split from [`AttachmentStore`] so the FS adapter doesn't have
    /// to know about SQLite and vice versa.
    pub attachment_json: Arc<dyn AttachmentJsonPort>,

    /// Path to application local data directory (DB, artifacts, etc.).
    pub app_data_dir: std::path::PathBuf,

    /// Base directory for project workspace storage (where repos are cloned).
    ///
    /// Defaults to `app_data_dir`. Users can override via the
    /// `workspace_base_dir` app-session key; takes effect after restart.
    pub workspace_dir: std::path::PathBuf,

    /// Headless-runner run submissions (docs/REMOTE_EXECUTION.md
    /// M3.2), keyed by client-generated `run_id`. Unused by the Tauri app
    /// (the table exists in its database via the shared migration set,
    /// but nothing ever writes to it there).
    pub runner_runs: Arc<dyn RunnerRunPort>,

    /// Append-only per-run event log (M3.3), read by the `stream_events`
    /// RPC. Also unused by the Tauri app.
    pub run_events: Arc<dyn RunEventsPort>,

    /// Laptop-side mirror of remote runs (M6.1/M6.2), keyed by
    /// `(machine_id, run_id)`. Unused by `demeteo-runner` — only the
    /// Tauri app populates it.
    pub remote_run_mirror: Arc<dyn RemoteRunMirrorPort>,

    /// Serializes local mirror dismissal with reconciliation's guarded
    /// status/hydration work. A reconciler may list a row before cleanup,
    /// but it must reclaim that row under this guard before it can apply any
    /// runner state; cleanup removes the row while holding the same guard.
    pub remote_run_mirror_guard: Arc<tokio::sync::Mutex<()>>,

    /// Single read model for a run's rendered surface — feature, steps,
    /// per-step artifacts, agent stream, cost (C3,
    /// `docs/EXECUTION_PARITY.md`). UI display commands read through
    /// this instead of reaching for `features`/`threads`/`exec` directly, so a
    /// runner-owned feature can later be sourced from a shadow mirror (C4)
    /// transparently to the UI.
    pub run_view: Arc<crate::application::run_view::RunView>,
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
    /// Human-facing name from the runtime's declared capabilities, so the UI
    /// never has to derive a label from the kind slug.
    pub display_label: String,
}

/// A registered coding agent and the capabilities Demeteo asks of it, exposed
/// to the frontend so the UI has a single source of truth for "which agents
/// exist" instead of a hardcoded list per component.
#[derive(Serialize)]
pub struct AgentCatalogEntry {
    pub kind: String,
    pub display_label: String,
    pub lists_models: bool,
    pub default_model: Option<String>,
    pub install_command: String,
    /// The effort levels this agent actually accepts, straight from its
    /// declared `AgentCapabilities`. Empty (hermes) means the agent has no
    /// per-invocation effort control at all, and the UI must not offer one.
    pub effort_levels: Vec<crate::domain::models::EffortLevel>,
    /// What Demeteo's own spawn flags do to this harness's machine-local
    /// personalization, straight from its declared `AgentCapabilities`. The UI
    /// states the consequence before a run starts; the type's docs
    /// ([`PersonalizationSupport`](crate::ports::agent_runtime::PersonalizationSupport))
    /// carry why the answer is about Demeteo and not about the harness.
    pub personalization: crate::ports::agent_runtime::PersonalizationSupport,
}
