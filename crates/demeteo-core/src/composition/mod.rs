//! Composition root. `build_core_context` is the only place in the engine
//! that constructs concrete adapters and wires them into the `AppContext`
//! dependency bag (which doubles as the design doc's `CoreContext` —
//! see docs/REMOTE_EXECUTION_PLAN.md M0.2).
//!
//! Both binaries call this: the Tauri app adapts the result into its own
//! managed state (`app.manage(ctx)`), and the future headless runner
//! (`demeteo-runner`, M1) calls it directly with `ExecutionMode::LocalOnly`
//! and a non-UI `NotificationPort`. `NotificationPort` is the one adapter
//! that must be injected rather than constructed here, because it is the
//! part of the DI graph that genuinely differs per composition root
//! (Tauri event emitter vs. webhook/email); everything else is identical
//! between the two binaries.

use crate::adapters;
use crate::ports;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentRuntime;
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::NotificationPort;
use crate::state::AppContext;
use std::path::PathBuf;
use std::sync::Arc;

/// How to route `ExecutionPort` calls.
pub enum ExecutionMode {
    /// Route per-`Machine`: local subprocess for `auth_type == "local"`,
    /// SSH otherwise. Used by the desktop app, which manages a mix of
    /// local and remote machines.
    Router,
    /// Always execute locally, never over SSH. Used by the headless
    /// runner (M1): the runner *is* the machine it's running on, so
    /// nested SSH collapses away (docs/REMOTE_EXECUTION.md §3).
    LocalOnly,
}

/// Everything `build_core_context` needs that isn't itself a port
/// implementation detail: where on disk to root the database/artifacts,
/// and how to route execution.
pub struct CoreConfig {
    /// Directory holding `demeteo.db`, artifacts, and attachments. Also the
    /// default workspace (repo clone) directory when the `workspace_base_dir`
    /// app-setting is unset.
    pub app_data_dir: PathBuf,
    pub execution_mode: ExecutionMode,
}

/// Construct every adapter and wire them into the single `AppContext`
/// dependency bag, starting the engine's background tasks (scheduler,
/// MR-state monitor, memory worker, and interrupted-feature resume) along
/// the way.
///
/// `runtime` is an explicit `tokio::runtime::Handle` rather than relying on
/// an ambient "current" runtime, because the two callers differ: the
/// headless runner calls this from inside its own `#[tokio::main]`, where
/// `Handle::current()` works fine — but the Tauri app calls this
/// synchronously from its (non-async) `.setup()` hook, which is *not*
/// polled as a task, so there is no ambient reactor to find. Tauri keeps
/// its own lazily-created global runtime (`tauri::async_runtime`) that is
/// separate from any thread-local "current" one; the app passes that
/// runtime's handle in explicitly instead.
///
/// Does **not** seed starter workflows or manage any Tauri-specific state —
/// those stay in each binary's own thin composition shell.
pub fn build_core_context(
    cfg: CoreConfig,
    notif: Arc<dyn NotificationPort>,
    runtime: tokio::runtime::Handle,
) -> AppContext {
    let CoreConfig {
        app_data_dir,
        execution_mode,
    } = cfg;

    let conn = crate::db::init_db(app_data_dir.clone()).expect("Failed to initialize database");
    let db_adapter = Arc::new(
        adapters::database::SqliteAdapter::new(conn)
            .expect("Failed to initialize database adapter"),
    );
    let machines_repo: Arc<dyn ports::db::MachineRepository> = db_adapter.clone();
    let projects_repo: Arc<dyn ports::db::ProjectRepository> = db_adapter.clone();
    let features_repo: Arc<dyn ports::db::FeatureRepository> = db_adapter.clone();
    let workflows_repo: Arc<dyn ports::db::WorkflowRepository> = db_adapter.clone();
    let gates_repo: Arc<dyn ports::db::GateRepository> = db_adapter.clone();
    let app_settings_repo: Arc<dyn ports::db::AppSettingsRepository> = db_adapter.clone();
    let memory_repo: Arc<dyn ports::memory::ProjectMemoryPort> = db_adapter.clone();
    let signals_repo: Arc<dyn ports::memory_signals::MemorySignalsPort> = db_adapter.clone();
    let threads_repo: Arc<dyn ports::db::ThreadRepository> = db_adapter.clone();
    let merge_audit_repo: Arc<dyn ports::db::MergeAuditRepository> = db_adapter.clone();
    let notifications_repo: Arc<dyn ports::db::NotificationRepository> = db_adapter.clone();
    let runner_runs_repo: Arc<dyn ports::runner_run::RunnerRunPort> = db_adapter.clone();
    let run_events_repo: Arc<dyn ports::run_events::RunEventsPort> = db_adapter.clone();
    let remote_run_mirror_repo: Arc<dyn ports::remote_run_mirror::RemoteRunMirrorPort> =
        db_adapter.clone();

    // Resolve the workspace directory: user-configurable base for repo
    // storage, defaults to `app_data_dir`. Takes effect on next launch
    // after the setting is changed.
    let workspace_dir: PathBuf = app_settings_repo
        .get_app_session("workspace_base_dir")
        .ok()
        .flatten()
        .and_then(|p| {
            if p.trim().is_empty() {
                return None;
            }
            let path = PathBuf::from(p.trim());
            if path.is_absolute() {
                Some(path)
            } else {
                None
            }
        })
        .unwrap_or_else(|| app_data_dir.clone());

    let exec_inner: Arc<dyn ExecutionPort> = match execution_mode {
        ExecutionMode::Router => {
            let ssh_adapter: Arc<dyn ExecutionPort> = Arc::new(
                adapters::ssh::client::SshClientAdapter::new(machines_repo.clone()),
            );
            let local_adapter: Arc<dyn ExecutionPort> =
                Arc::new(adapters::local::execution::LocalSubprocessAdapter::new());
            Arc::new(adapters::router::RouterExecutionPort::new(
                machines_repo.clone(),
                ssh_adapter,
                local_adapter,
            ))
        }
        ExecutionMode::LocalOnly => {
            Arc::new(adapters::local::execution::LocalSubprocessAdapter::new())
        }
    };

    let agent_exec: Arc<dyn AgentExecutionPort> = Arc::new(
        adapters::agent::direct_execution::DirectExecutionPort::new(exec_inner.clone()),
    );

    let agent_registry = Arc::new(adapters::agent::registry::AgentRegistry::new(vec![
        Arc::new(adapters::agent::opencode::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::hermes::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::claude_code::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::antigravity::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::noop::NoopRuntime) as Arc<dyn AgentRuntime>,
    ]));
    let pricing: Arc<dyn ports::pricing::PricingTable> =
        Arc::new(adapters::pricing::HardcodedPricingTable::new());
    let mr_publisher: Arc<dyn ports::mr_publisher::MrPublisher> =
        Arc::new(adapters::mr_publisher::HttpMrPublisher::new(
            app_settings_repo.clone(),
            projects_repo.clone(),
            features_repo.clone(),
            exec_inner.clone(),
            workspace_dir.clone(),
        ));

    let worktree_ops = Arc::new(adapters::worktree::git_ops::GitOpsHelper::new(
        app_settings_repo.clone(),
        exec_inner.clone(),
    ));

    let provider_http = Arc::new(adapters::provider_http::ReqwestProviderHttpAdapter::new());

    let memory_llm: Arc<dyn ports::memory_llm::MemoryLlmPort> =
        Arc::new(adapters::memory_llm::ReqwestMemoryLlmAdapter::new());

    // Merge executor — owns the SQL audit table + the structured
    // conflict-report shape. Wired here so the feature_sync command and
    // the existing subtask→feature merge share the same
    // conflict-detection code path.
    let merge_executor: Arc<dyn ports::merge::MergeExecutor> = {
        let git_ops_for_merge = adapters::worktree::git_ops::GitOpsHelper::new(
            app_settings_repo.clone(),
            exec_inner.clone(),
        );
        Arc::new(adapters::merge::SqliteMergeExecutor::new(
            merge_audit_repo.clone(),
            git_ops_for_merge,
            exec_inner.clone(),
            workspace_dir.clone(),
        ))
    };

    // Build the DagStepExecutor before AppContext to avoid a circular
    // dependency (the executor contains sub-port Arcs; AppContext
    // contains the executor's Arc).
    let attachment_store: Arc<dyn ports::attachment_store::AttachmentStore> = Arc::new(
        adapters::attachment_store::fs::FsAttachmentStore::new(app_data_dir.clone()),
    );
    let attachment_json: Arc<dyn ports::attachment_store::AttachmentJsonPort> = db_adapter.clone();
    let step_executor_adapter = {
        let artifact_store: Arc<dyn ports::artifact_store::ArtifactStore> = Arc::new(
            adapters::artifact_store::fs::FsArtifactStore::new(app_data_dir.clone()),
        );
        let exec = Arc::new(adapters::step_executor::DagStepExecutor::new(
            machines_repo.clone(),
            projects_repo.clone(),
            features_repo.clone(),
            workflows_repo.clone(),
            gates_repo.clone(),
            app_settings_repo.clone(),
            memory_repo.clone(),
            signals_repo.clone(),
            memory_llm.clone(),
            agent_registry.clone(),
            notif.clone(),
            notifications_repo.clone(),
            agent_exec.clone(),
            exec_inner.clone(),
            merge_executor.clone(),
            artifact_store,
            attachment_store.clone(),
            attachment_json.clone(),
            workspace_dir.clone(),
            pricing.clone(),
        ));
        // Reconcile DB + notifications first (synchronous, fast).
        exec.startup_watchdog();
        // Then spawn the actual driver resumes on the runtime. Without
        // this, the re-emitted GateRequired events have no live driver
        // behind them and the user's gate_decide is silently dropped —
        // see the watchdog/registry docs.
        let exec_for_resume = exec.clone();
        runtime.spawn(async move {
            exec_for_resume.resume_interrupted_features().await;
        });
        exec
    };

    // Start workflow scheduler background task.
    adapters::scheduler::start_scheduler(
        workflows_repo.clone(),
        step_executor_adapter.clone(),
        &runtime,
    );

    // Start the background MR-state monitor. Polls
    // `MrPublisher::fetch_mr_state` every 2 minutes, persists a
    // `Notification` row on transition to `merged`, and emits
    // `DomainEvent::MrMerged` for the bell + toast.
    adapters::mr_monitor::start_mr_monitor(
        features_repo.clone(),
        mr_publisher.clone(),
        notifications_repo.clone(),
        notif.clone(),
        &runtime,
    );

    // Start the background memory agent. Polls the memory_signals queue,
    // distills signals into project memories via the user-configured LLM.
    // No-ops while the memory agent is disabled.
    adapters::memory_worker::start_memory_worker(
        app_settings_repo.clone(),
        signals_repo.clone(),
        memory_repo.clone(),
        memory_llm.clone(),
        &runtime,
    );

    // Single read model for the rendered run surface (C3). Delegates to the
    // laptop repos for local/SSH runs; the C4 runner mirror plugs in here.
    let run_view = Arc::new(crate::application::run_view::RunView::new(
        features_repo.clone(),
        threads_repo.clone(),
        exec_inner.clone(),
    ));

    AppContext {
        machines: machines_repo,
        threads: threads_repo,
        projects: projects_repo,
        features: features_repo,
        workflows: workflows_repo,
        gates: gates_repo,
        app_settings: app_settings_repo,
        memory: memory_repo,
        signals: signals_repo,
        merge_audit: merge_audit_repo,
        notifications: notifications_repo,
        exec: exec_inner,
        agent_exec,
        notif,
        registry: agent_registry,
        executor: step_executor_adapter.clone(),
        presenter: step_executor_adapter,
        pricing,
        mr_publisher,
        merge_executor,
        worktree_ops,
        provider_http,
        memory_llm,
        attachments: attachment_store,
        attachment_json: db_adapter,
        app_data_dir,
        workspace_dir,
        runner_runs: runner_runs_repo,
        run_events: run_events_repo,
        remote_run_mirror: remote_run_mirror_repo,
        run_view,
    }
}
