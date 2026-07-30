use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::driver_registry::DriverRegistry;
use crate::adapters::step_executor::gate_waiter::GateWaiter;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::attachment_store::{AttachmentJsonPort, AttachmentStore};
use crate::ports::db::{
    AppSettingsRepository, FeatureRepository, GateRepository, MachineRepository, ProjectRepository,
    WorkflowRepository,
};
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::notification::NotificationPort;
use crate::ports::pricing::PricingTable;

use crate::adapters::worktree::git_ops::GitOpsHelper;

// ── Sub-modules (deep-module decomposition) ────────────────────────────────────

pub(crate) mod artifacts;
pub(crate) mod baseline;
pub(crate) mod context;
pub(crate) mod driver;
pub(crate) mod driver_registry;
pub(crate) mod failing_tests;
pub(crate) mod gate_waiter;
pub(crate) mod harness_shell;
pub(crate) mod impl_traits;
pub mod node_catalog;
pub mod node_lint;
pub mod preflight;
pub(crate) mod registry;
pub(crate) mod retry_policy;
pub mod scheduler;
pub mod setup;
pub(crate) mod spend;
pub(crate) mod step_status;
pub(crate) mod steps;
pub(crate) mod sync;
pub(crate) mod updates;

#[cfg(test)]
#[path = "../../../tests/e2e/step_executor.rs"]
mod tests;

// ── Core struct ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DagStepExecutor {
    #[allow(dead_code)]
    machines: Arc<dyn MachineRepository>,
    projects: Arc<dyn ProjectRepository>,
    features: Arc<dyn FeatureRepository>,
    workflows: Arc<dyn WorkflowRepository>,
    gates: Arc<dyn GateRepository>,
    app_settings: Arc<dyn AppSettingsRepository>,
    memory: Arc<dyn crate::ports::memory::ProjectMemoryPort>,
    signals: Arc<dyn crate::ports::memory_signals::MemorySignalsPort>,
    memory_llm: Arc<dyn crate::ports::memory_llm::MemoryLlmPort>,
    registry: Arc<AgentRegistry>,
    notif: Arc<dyn NotificationPort>,
    /// Notification persistence port. See the `new` arg
    /// documentation for why this lives on the executor.
    pub notifications: Arc<dyn crate::ports::db::NotificationRepository>,
    agent_exec: Arc<dyn AgentExecutionPort>,
    exec: Arc<dyn ExecutionPort>,
    /// Merge executor — wraps `git merge` for both subtask→feature
    /// and feature→upstream flows with structured conflict
    /// detection and an audit trail.
    pub merge_executor: Arc<dyn MergeExecutor>,
    /// Per-task run telemetry for `sequence` steps — one `subtask_runs`
    /// row per (task, attempt). See
    /// [`SubtaskRunRepository`](crate::ports::db::SubtaskRunRepository).
    subtask_runs: Arc<dyn crate::ports::db::SubtaskRunRepository>,
    /// Durable crash-resume state for `sequence` steps (checkpoint +
    /// plan cache). See
    /// [`SequenceResumeRepository`](crate::ports::db::SequenceResumeRepository).
    sequence_resume: Arc<dyn crate::ports::db::SequenceResumeRepository>,
    /// Git operations helper for the resolver / sync flows. The
    /// step handlers (agent.rs, parallel.rs) own their own
    /// `GitOpsHelper` instances; this one is dedicated to the
    /// `feature_sync` and `feature_resolve_sync_conflicts` paths
    /// so the two flows don't share transient state.
    pub git_ops: GitOpsHelper,
    /// Artifact persistence port. The step executor and the tool
    /// bridge both route artifact I/O through this so a future S3
    /// or SFTP-on-remote adapter can swap in without touching either
    /// caller. See `docs/ARCHITECTURE.md` §2 (locked port catalogue).
    artifacts: Arc<dyn ArtifactStore>,
    /// Per-feature user attachment store. Threaded through to every
    /// `ExecutionDriver` so `spawn_agent_session` can copy the
    /// feature's attachments into the per-step worktree before the
    /// agent's first turn (so the `external_directory: deny` fence
    /// accepts the `Read` tool call).
    pub attachments: Arc<dyn AttachmentStore>,
    /// JSON-manifest persistence for the same attachment list. Lives
    /// on the executor so [`feature_start`] can persist staged
    /// attachments to the freshly-created feature row BEFORE the
    /// driver is spawned — without this, the agent's first turn would
    /// race against the frontend's post-launch `feature_add_attachment`
    /// calls and sometimes miss the user's files.
    pub attachment_json: Arc<dyn AttachmentJsonPort>,
    workspace_dir: PathBuf,
    /// Live drivers keyed by step_execution_id. The driver inserts a
    /// `GateWaiter` while waiting for a decision; `gate_decide` looks up
    /// the waiter (fast path) and also writes to the DB (durable path).
    /// If the entry is absent — driver died, restart, or race — the
    /// caller falls back to `ensure_driver_running` so the DB row is
    /// reconciled on driver resume.
    gate_waiters: Arc<Mutex<HashMap<String, Arc<GateWaiter>>>>,
    /// Tracks which features currently have a live execution driver so
    /// `gate_decide` and `startup_watchdog` can re-spawn safely without
    /// doubling up on an in-flight run.
    driver_registry: Arc<DriverRegistry>,
    cancel_senders: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    /// Model → USD pricing. Plumbed through to every driver + agent turn
    /// so [`UsageAccumulator`](crate::domain::usage::UsageAccumulator) can
    /// backfill `cost_usd` when the agent's wire format omits it.
    pricing: Arc<dyn PricingTable>,
    /// Remote-run mirror (C4.2). A feature listed here is a read-only
    /// *shadow* of a run a `demeteo-runner` owns and drives on another
    /// machine — the engine must never interrupt its steps or arm a
    /// local driver against it (`ensure_driver_running`,
    /// `startup_watchdog`, `resume_interrupted_features` all consult it).
    /// Empty on the runner itself (its mirror table never gains rows).
    remote_run_mirror: Arc<dyn crate::ports::remote_run_mirror::RemoteRunMirrorPort>,
    /// Opens the PR when a run finishes, from the summary the `finalize` step
    /// authored. Wired only by the desktop composition — see
    /// [`with_mr_publisher`](Self::with_mr_publisher).
    mr_publisher: Option<Arc<dyn crate::ports::mr_publisher::MrPublisher>>,
}

impl DagStepExecutor {
    #[allow(clippy::too_many_arguments)]
    /// Per-feature user attachment store. Threaded through to every
    /// `ExecutionDriver` so `spawn_agent_session` can copy the
    /// feature's attachments into the per-step worktree before the
    /// agent's first turn (so the `external_directory: deny` fence
    /// accepts the `Read` tool call).
    pub fn new(
        machines: Arc<dyn MachineRepository>,
        projects: Arc<dyn ProjectRepository>,
        features: Arc<dyn FeatureRepository>,
        workflows: Arc<dyn WorkflowRepository>,
        gates: Arc<dyn GateRepository>,
        app_settings: Arc<dyn AppSettingsRepository>,
        memory: Arc<dyn crate::ports::memory::ProjectMemoryPort>,
        signals: Arc<dyn crate::ports::memory_signals::MemorySignalsPort>,
        memory_llm: Arc<dyn crate::ports::memory_llm::MemoryLlmPort>,
        registry: Arc<AgentRegistry>,
        notif: Arc<dyn NotificationPort>,
        // Notification persistence port. Used to write a row to
        // the `notifications` table when the engine emits a
        // user-visible event the user needs to act on (e.g. a
        // `RetryBudgetExhausted`). The same `SqliteAdapter`
        // already passed for `features` / `gates` implements
        // this port; threading it through here keeps the
        // orchestrator in charge of writing notification rows
        // instead of delegating to a background monitor (which
        // is the pattern `MrMerged` uses, but doesn't fit the
        // synchronous step-failure hot path).
        notifications: Arc<dyn crate::ports::db::NotificationRepository>,
        agent_exec: Arc<dyn AgentExecutionPort>,
        exec: Arc<dyn ExecutionPort>,
        merge_executor: Arc<dyn MergeExecutor>,
        // Per-task run telemetry for `sequence` steps. Same
        // `SqliteAdapter` as `features` / `gates`; threaded as its own
        // sub-port so the driver's task loop can open/close
        // `subtask_runs` rows without seeing the rest of the schema.
        subtask_runs: Arc<dyn crate::ports::db::SubtaskRunRepository>,
        // Durable crash-resume state for `sequence` steps: which tasks
        // landed at which commit, and the cached plan. Same
        // `SqliteAdapter` again; its own sub-port so the sequence step
        // depends on six methods rather than all of
        // `FeatureRepository` (docs/ARCHITECTURE.md §2).
        sequence_resume: Arc<dyn crate::ports::db::SequenceResumeRepository>,
        artifacts: Arc<dyn ArtifactStore>,
        attachments: Arc<dyn AttachmentStore>,
        attachment_json: Arc<dyn AttachmentJsonPort>,
        workspace_dir: PathBuf,
        pricing: Arc<dyn PricingTable>,
        remote_run_mirror: Arc<dyn crate::ports::remote_run_mirror::RemoteRunMirrorPort>,
    ) -> Self {
        let git_ops = GitOpsHelper::new(app_settings.clone(), exec.clone());
        Self {
            machines,
            projects,
            features,
            workflows,
            gates,
            app_settings,
            memory,
            signals,
            memory_llm,
            registry,
            notif,
            notifications,
            agent_exec,
            exec,
            merge_executor,
            subtask_runs,
            sequence_resume,
            git_ops,
            artifacts,
            attachments,
            attachment_json,
            workspace_dir,
            gate_waiters: Arc::new(Mutex::new(HashMap::new())),
            driver_registry: DriverRegistry::new(),
            cancel_senders: Arc::new(Mutex::new(HashMap::new())),
            pricing,
            remote_run_mirror,
            mr_publisher: None,
        }
    }

    /// Give the executor a publisher, so a finished run opens its own PR from
    /// the summary the `finalize` step authored.
    ///
    /// Desktop only. The headless runner deliberately does **not** call this:
    /// it holds no keyring credential a publisher could resolve a PAT from,
    /// and opens its PR at the end of `run.rs` with a memory-only, run-scoped
    /// PAT instead (docs/REMOTE_EXECUTION.md §6.2). A runner that wired one up
    /// here would fail the publish and force a credential to be resident for
    /// the whole run — the exact thing that design avoids.
    pub fn with_mr_publisher(
        mut self,
        publisher: Arc<dyn crate::ports::mr_publisher::MrPublisher>,
    ) -> Self {
        self.mr_publisher = Some(publisher);
        self
    }

    /// The feature ids currently listed in the remote-run mirror — the
    /// runner-owned shadow set (C4.2) restart reconciliation and driver
    /// recovery must leave alone. Read fresh on each use: a run can be
    /// submitted (or its mirror row hydrated) at any point after startup.
    pub(crate) fn runner_owned_features(&self) -> std::collections::HashSet<String> {
        self.remote_run_mirror
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| r.feature_id)
            .collect()
    }

    /// Read-only view of the live-driver registry, used by `gate_decide`
    /// to decide whether the driver is alive without taking the lock twice.
    pub fn driver_registry(&self) -> Arc<DriverRegistry> {
        self.driver_registry.clone()
    }

    /// Access the shared waiter map. Tests and the gate handler both
    /// need to insert / remove waiters; the executor owns the canonical
    /// instance and hands out clones via this accessor.
    pub fn gate_waiters(&self) -> Arc<Mutex<HashMap<String, Arc<GateWaiter>>>> {
        self.gate_waiters.clone()
    }
}
