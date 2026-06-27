//! Composition root. `setup_app_state` is the single place that constructs
//! concrete adapters, wires them into `AppContext`, and registers Tauri
//! managed state. Adding a new port requires touching this function only.

pub use crate::state::AppContext;

use crate::forward::ForwardState;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentRuntime;
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::NotificationPort;
use crate::terminal::SessionState;
use std::sync::Arc;
use tauri::Manager;

pub fn setup_app_state(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_local_data_dir()?;
    tracing::info!(path = %app_data_dir.display(), "data dir");
    let conn = crate::db::init_db(app_data_dir.clone())?;

    let db_adapter = Arc::new(
        crate::adapters::database::SqliteAdapter::new(conn)?,
    );
    let machines_repo: Arc<dyn crate::ports::db::MachineRepository> = db_adapter.clone();
    let projects_repo: Arc<dyn crate::ports::db::ProjectRepository> = db_adapter.clone();
    let features_repo: Arc<dyn crate::ports::db::FeatureRepository> = db_adapter.clone();
    let workflows_repo: Arc<dyn crate::ports::db::WorkflowRepository> = db_adapter.clone();
    let gates_repo: Arc<dyn crate::ports::db::GateRepository> = db_adapter.clone();
    let app_settings_repo: Arc<dyn crate::ports::db::AppSettingsRepository> = db_adapter.clone();
    let memory_repo: Arc<dyn crate::ports::memory::ProjectMemoryPort> = db_adapter.clone();
    let signals_repo: Arc<dyn crate::ports::memory_signals::MemorySignalsPort> = db_adapter.clone();
    let threads_repo: Arc<dyn crate::ports::db::ThreadRepository> = db_adapter.clone();
    let merge_audit_repo: Arc<dyn crate::ports::db::MergeAuditRepository> = db_adapter.clone();
    let notifications_repo: Arc<dyn crate::ports::db::NotificationRepository> = db_adapter.clone();

    // Resolve the workspace directory: user-configurable base for repo
    // storage, defaults to the Tauri app data dir. Takes effect on
    // next launch after the setting is changed.
    let workspace_dir: std::path::PathBuf = app_settings_repo
        .get_app_session("workspace_base_dir")
        .ok()
        .flatten()
        .and_then(|p| {
            if p.trim().is_empty() {
                return None;
            }
            let path = std::path::PathBuf::from(p.trim());
            if path.is_absolute() {
                Some(path)
            } else {
                None
            }
        })
        .unwrap_or_else(|| app_data_dir.clone());
    tracing::info!(path = %workspace_dir.display(), "workspace dir");

    crate::commands::workflows::seed_starter_workflows(&workflows_repo);

    let ssh_adapter: Arc<dyn ExecutionPort> = Arc::new(
        crate::adapters::ssh::client::SshClientAdapter::new(machines_repo.clone()),
    );
    let local_adapter: Arc<dyn ExecutionPort> =
        Arc::new(crate::adapters::local::execution::LocalSubprocessAdapter::new());
    let exec_inner: Arc<dyn ExecutionPort> =
        Arc::new(crate::adapters::router::RouterExecutionPort::new(
            machines_repo.clone(),
            ssh_adapter,
            local_adapter,
        ));
    let notif_adapter: Arc<dyn NotificationPort> = Arc::new(
        crate::adapters::tauri_ui::notification::TauriNotificationAdapter::new(
            app.handle().clone(),
        ),
    );
    let agent_exec: Arc<dyn AgentExecutionPort> = Arc::new(
        crate::adapters::agent::direct_execution::DirectExecutionPort::new(exec_inner.clone()),
    );

    let agent_registry =
        Arc::new(crate::adapters::agent::registry::AgentRegistry::new(vec![
            Arc::new(crate::adapters::agent::opencode::runtime()) as Arc<dyn AgentRuntime>,
            Arc::new(crate::adapters::agent::hermes::runtime()) as Arc<dyn AgentRuntime>,
            Arc::new(crate::adapters::agent::claude_code::runtime()) as Arc<dyn AgentRuntime>,
            Arc::new(crate::adapters::agent::antigravity::runtime()) as Arc<dyn AgentRuntime>,
            Arc::new(crate::adapters::agent::noop::NoopRuntime) as Arc<dyn AgentRuntime>,
        ]));
    let pricing: Arc<dyn crate::ports::pricing::PricingTable> =
        Arc::new(crate::adapters::pricing::HardcodedPricingTable::new());
    let mr_publisher: Arc<dyn crate::ports::mr_publisher::MrPublisher> =
        Arc::new(crate::adapters::mr_publisher::HttpMrPublisher::new(
            app_settings_repo.clone(),
            projects_repo.clone(),
            features_repo.clone(),
            exec_inner.clone(),
            workspace_dir.clone(),
        ));

    let worktree_ops = Arc::new(crate::adapters::worktree::git_ops::GitOpsHelper::new(
        app_settings_repo.clone(),
        exec_inner.clone(),
    ));

    let provider_http =
        Arc::new(crate::adapters::provider_http::ReqwestProviderHttpAdapter::new());

    let memory_llm: Arc<dyn crate::ports::memory_llm::MemoryLlmPort> =
        Arc::new(crate::adapters::memory_llm::ReqwestMemoryLlmAdapter::new());

    // Merge executor — owns the SQL audit table + the structured
    // conflict-report shape. Wired here so the feature_sync command
    // and the existing subtask→feature merge share the same
    // conflict-detection code path.
    let merge_executor: Arc<dyn crate::ports::merge::MergeExecutor> = {
        let git_ops_for_merge = crate::adapters::worktree::git_ops::GitOpsHelper::new(
            app_settings_repo.clone(),
            exec_inner.clone(),
        );
        Arc::new(crate::adapters::merge::SqliteMergeExecutor::new(
            merge_audit_repo.clone(),
            git_ops_for_merge,
            exec_inner.clone(),
            workspace_dir.clone(),
        ))
    };

    // Build the DagStepExecutor before AppContext to avoid a circular
    // dependency (the executor contains sub-port Arcs; AppContext
    // contains the executor's Arc).
    let step_executor_adapter = {
        let artifact_store: Arc<dyn crate::ports::artifact_store::ArtifactStore> = Arc::new(
            crate::adapters::artifact_store::fs::FsArtifactStore::new(app_data_dir.clone()),
        );
        let exec = Arc::new(crate::adapters::step_executor::DagStepExecutor::new(
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
            notif_adapter.clone(),
            notifications_repo.clone(),
            agent_exec.clone(),
            exec_inner.clone(),
            merge_executor.clone(),
            artifact_store,
            app_data_dir.clone(),
            workspace_dir.clone(),
        ));
        // Reconcile DB + notifications first (synchronous, fast).
        exec.startup_watchdog();
        // Then spawn the actual driver resumes on the runtime.
        // Without this, the re-emitted GateRequired events have no live
        // driver behind them and the user's gate_decide is silently
        // dropped — see the watchdog/registry docs.
        let exec_for_resume = exec.clone();
        tauri::async_runtime::spawn(async move {
            exec_for_resume.resume_interrupted_features().await;
        });
        exec
    };

    // Start workflow scheduler background task.
    crate::adapters::scheduler::start_scheduler(
        workflows_repo.clone(),
        step_executor_adapter.clone(),
    );

    // Start the background MR-state monitor. Polls
    // `MrPublisher::fetch_mr_state` every 2 minutes, persists a
    // `Notification` row on transition to `merged`, and emits
    // `DomainEvent::MrMerged` for the bell + toast.
    crate::adapters::mr_monitor::start_mr_monitor(
        features_repo.clone(),
        mr_publisher.clone(),
        notifications_repo.clone(),
        notif_adapter.clone(),
    );

    // Start the background memory agent. Polls the memory_signals queue,
    // distills signals into project memories via the user-configured LLM.
    // No-ops while the memory agent is disabled.
    crate::adapters::memory_worker::start_memory_worker(
        app_settings_repo.clone(),
        signals_repo.clone(),
        memory_repo.clone(),
        memory_llm.clone(),
    );

    app.manage(AppContext {
        machines: machines_repo.clone(),
        threads: threads_repo.clone(),
        projects: projects_repo.clone(),
        features: features_repo.clone(),
        workflows: workflows_repo.clone(),
        gates: gates_repo.clone(),
        app_settings: app_settings_repo.clone(),
        memory: memory_repo,
        signals: signals_repo.clone(),
        merge_audit: merge_audit_repo,
        notifications: notifications_repo,
        exec: exec_inner,
        agent_exec,
        notif: notif_adapter,
        registry: agent_registry,
        executor: step_executor_adapter.clone(),
        presenter: step_executor_adapter,
        pricing,
        mr_publisher,
        merge_executor,
        worktree_ops,
        provider_http,
        memory_llm: memory_llm.clone(),
        app_data_dir: app_data_dir.clone(),
        workspace_dir: workspace_dir.clone(),
    });
    app.manage(SessionState::default());
    app.manage(ForwardState::default());

    Ok(())
}
