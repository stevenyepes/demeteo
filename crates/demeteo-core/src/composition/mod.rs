//! Composition root. `build_core_context` is the only place in the engine
//! that constructs concrete adapters and wires them into the `AppContext`
//! dependency bag (which doubles as the design doc's `CoreContext` —
//! see docs/REMOTE_EXECUTION.md M0.2).
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
use crate::application;
use crate::ports;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentRuntime;
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::NotificationPort;
use crate::state::{AppContext, AppVersion};
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
    build_core_context_with_policy_writer(cfg, notif, runtime).0
}

/// [`build_core_context`], plus the one [`ports::hub::HubPolicyWriter`] over
/// the same database. It is returned beside the context rather than inside
/// it because the Hub request handler is built over the context, and nothing
/// the handler can reach may write the policy it is judged by (docs/HUB.md
/// §7). Only the desktop's Settings command should hold it.
pub fn build_core_context_with_policy_writer(
    cfg: CoreConfig,
    notif: Arc<dyn NotificationPort>,
    runtime: tokio::runtime::Handle,
) -> (AppContext, Arc<dyn ports::hub::HubPolicyWriter>) {
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
    // Wrapped here, before the first clone: a clone of the bare repo hands
    // its holder writes the snapshot never hears about (docs/HUB.md §5).
    let snapshot_wake = Arc::new(tokio::sync::Notify::new());
    let projects_repo: Arc<dyn ports::db::ProjectRepository> = Arc::new(
        adapters::hub::socket::snapshot_wake::WakingProjects::with_wake(
            db_adapter.clone(),
            snapshot_wake.clone(),
        ),
    );
    let workflows_repo: Arc<dyn ports::db::WorkflowRepository> = Arc::new(
        adapters::hub::socket::snapshot_wake::WakingWorkflows::with_wake(
            db_adapter.clone(),
            snapshot_wake.clone(),
        ),
    );
    let features_repo: Arc<dyn ports::db::FeatureRepository> = db_adapter.clone();
    let sequence_resume_repo: Arc<dyn ports::db::SequenceResumeRepository> = db_adapter.clone();
    let gates_repo: Arc<dyn ports::db::GateRepository> = db_adapter.clone();
    let app_settings_repo: Arc<dyn ports::db::AppSettingsRepository> = db_adapter.clone();
    let memory_repo: Arc<dyn ports::memory::ProjectMemoryPort> = db_adapter.clone();
    let signals_repo: Arc<dyn ports::memory_signals::MemorySignalsPort> = db_adapter.clone();
    let threads_repo: Arc<dyn ports::db::ThreadRepository> = db_adapter.clone();
    let merge_audit_repo: Arc<dyn ports::db::MergeAuditRepository> = db_adapter.clone();
    let notifications_repo: Arc<dyn ports::db::NotificationRepository> = db_adapter.clone();
    let runner_runs_repo: Arc<dyn ports::runner_run::RunnerRunPort> = db_adapter.clone();
    // `RunEventRecorder` appends through `ctx.run_events`; handed the bare
    // repo instead, every row would wait for the socket's next connection.
    let waking_run_events = adapters::hub::socket::wake::WakingRunEvents::new(db_adapter.clone());
    let run_events_wake = waking_run_events.wake();
    let run_events_repo: Arc<dyn ports::run_events::RunEventsPort> = Arc::new(waking_run_events);
    let remote_run_mirror_repo: Arc<dyn ports::remote_run_mirror::RemoteRunMirrorPort> =
        db_adapter.clone();
    let sync_sessions_repo: Arc<dyn ports::sync_session::SyncSessionPort> = db_adapter.clone();
    let discoveries_repo: Arc<dyn ports::discovery::DiscoveryPort> = db_adapter.clone();
    let tickets_repo: Arc<dyn ports::discovery::TicketPort> = db_adapter.clone();
    let ask_repo: Arc<dyn ports::ask::AskPort> = db_adapter.clone();
    let oauth_clients: Arc<dyn ports::oauth::OAuthClientRepository> = db_adapter.clone();
    let oauth_grants: Arc<dyn ports::oauth::OAuthGrantRepository> = db_adapter.clone();
    let mcp_consent = Arc::new(adapters::mcp::consent_waiter::McpConsentWaiterRegistry::new());

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

    let mut runtimes: Vec<Arc<dyn AgentRuntime>> = vec![
        Arc::new(adapters::agent::opencode::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::hermes::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::claude_code::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::codex::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::pi::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(adapters::agent::noop::NoopRuntime) as Arc<dyn AgentRuntime>,
    ];
    // C5 topology gate only: the deterministic no-LLM `"stub"` agent is
    // registered exclusively when `DEMETEO_STUB_AGENT` is set (see
    // `adapters::agent::stub_runtime`). Every production path leaves the env
    // var unset, so no workflow can ever select it.
    if adapters::agent::stub_runtime::stub_agent_enabled() {
        runtimes
            .push(Arc::new(adapters::agent::stub_runtime::StubRuntime) as Arc<dyn AgentRuntime>);
    }
    let agent_registry = Arc::new(adapters::agent::registry::AgentRegistry::new(runtimes));
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
    // Shared by the merge executor and the step executor: it is the process's
    // only record of an out-of-band sync being in flight, and both the thing
    // that starts one and every reader that has to tell a live session from an
    // abandoned one need the same instance
    // (`domain::sync_session::sync_liveness`).
    let sync_turns = Arc::new(application::sync_turns::SyncTurns::default());

    let merge_executor: Arc<dyn ports::merge::MergeExecutor> = {
        let git_ops_for_merge = adapters::worktree::git_ops::GitOpsHelper::new(
            app_settings_repo.clone(),
            exec_inner.clone(),
        );
        Arc::new(adapters::merge::SqliteMergeExecutor::new(
            merge_audit_repo.clone(),
            sync_sessions_repo.clone(),
            features_repo.clone(),
            sync_turns.clone(),
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
    let artifact_store: Arc<dyn ports::artifact_store::ArtifactStore> = Arc::new(
        adapters::artifact_store::fs::FsArtifactStore::new(app_data_dir.clone()),
    );
    let cache_reclaim = Arc::new(application::cache_reclaim::DeferredReclaim::default());
    let step_executor_adapter = {
        let exec = adapters::step_executor::DagStepExecutor::new(
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
            db_adapter.clone(),
            sequence_resume_repo.clone(),
            artifact_store.clone(),
            attachment_store.clone(),
            attachment_json.clone(),
            workspace_dir.clone(),
            pricing.clone(),
            remote_run_mirror_repo.clone(),
            sync_turns.clone(),
        )
        .with_cache_reclaim(cache_reclaim.clone());
        // Only the desktop lets a finished run open its own PR. The headless
        // runner (`LocalOnly`) publishes at the end of `run.rs` instead, with
        // a memory-only PAT — it has no keyring for this publisher to resolve
        // one from, and holding a credential for the whole run is precisely
        // what its design avoids (docs/REMOTE_EXECUTION.md §6.2).
        let exec = Arc::new(match execution_mode {
            ExecutionMode::Router => exec.with_mr_publisher(mr_publisher.clone()),
            ExecutionMode::LocalOnly => exec,
        });
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
    let feature_cache: Arc<dyn ports::worktree_ops::FeatureCachePort> = Arc::new(
        crate::application::lifecycle::RoutedFeatureCacheRelease::new(
            crate::application::lifecycle::FeatureCacheRelease {
                projects: projects_repo.clone(),
                exec: exec_inner.clone(),
                workspace_dir: workspace_dir.clone(),
                worktree_ops: worktree_ops.clone(),
            },
            remote_run_mirror_repo.clone(),
            app_settings_repo.clone(),
        ),
    );
    adapters::mr_monitor::start_mr_monitor(
        adapters::mr_monitor::MrMonitorPorts {
            features: features_repo.clone(),
            mr_publisher: mr_publisher.clone(),
            notifications: notifications_repo.clone(),
            notif: notif.clone(),
            tickets: tickets_repo.clone(),
            discoveries: discoveries_repo.clone(),
            cache: feature_cache.clone(),
        },
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
        sequence_resume_repo.clone(),
        threads_repo.clone(),
        run_events_repo.clone(),
        exec_inner.clone(),
    ));

    let hub_requests_repo: Arc<dyn ports::hub::HubRequestRepository> = db_adapter.clone();
    let hub_policy_reader: Arc<dyn ports::hub::HubPolicyReader> = db_adapter.clone();
    let hub_policy_writer: Arc<dyn ports::hub::HubPolicyWriter> = db_adapter.clone();
    let hub_pins: Arc<dyn ports::hub::HubPinRepository> = db_adapter.clone();
    let hub_passkeys: Arc<dyn ports::hub::HubPasskeyTrust> = db_adapter.clone();
    let hub_passkey_reader: Arc<dyn ports::hub::HubPasskeyReader> = db_adapter.clone();

    let app_version = AppVersion::default();
    let hub = match execution_mode {
        ExecutionMode::Router => hub_pairing(
            hub_pins.clone(),
            hub_policy_reader.clone(),
            app_version.clone(),
            hub_socket(
                run_events_repo.clone(),
                run_events_wake,
                Arc::new(adapters::hub::snapshot_source::RepoSnapshotSource::new(
                    db_adapter.clone(),
                    db_adapter.clone(),
                    db_adapter.clone(),
                )),
                snapshot_wake,
                runtime.clone(),
            ),
        ),
        ExecutionMode::LocalOnly => None,
    };

    let ctx = AppContext {
        machines: machines_repo,
        threads: threads_repo,
        projects: projects_repo,
        features: features_repo,
        sequence_resume: sequence_resume_repo,
        workflows: workflows_repo,
        gates: gates_repo,
        app_settings: app_settings_repo,
        memory: memory_repo,
        signals: signals_repo,
        notifications: notifications_repo,
        exec: exec_inner,
        agent_exec,
        notif,
        registry: agent_registry,
        executor: step_executor_adapter.clone(),
        presenter: step_executor_adapter,
        pricing,
        mr_publisher,
        feature_cache,
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
        sync_sessions: sync_sessions_repo,
        discoveries: discoveries_repo,
        discovery_turns: Arc::new(application::discovery::running::RunningTurns::default()),
        ask_turns: Arc::new(application::ask::running::RunningTurns::default()),
        tickets: tickets_repo,
        ticket_starts: Arc::new(application::tickets::starting::StartingTickets::default()),
        sync_turns,
        remote_run_mirror_guard: Arc::new(tokio::sync::Mutex::new(())),
        run_view,
        ask: ask_repo,
        artifact_store,
        oauth_clients,
        oauth_grants,
        mcp_consent,
        app_version,
        hub,
        hub_requests: hub_requests_repo,
        hub_policy: hub_policy_reader,
        hub_passkeys: hub_passkey_reader,
    };

    // The MCP listener is the first background task that needs the *whole*
    // `AppContext` (to dispatch `agent_surface` calls from `/mcp`), so
    // it starts here rather than alongside the repo-scoped tasks above.
    // Desktop only — the headless runner has no consent UI and no reason to
    // host this (`docs/MCP_INTEGRATION.md` §3).
    if matches!(execution_mode, ExecutionMode::Router) {
        adapters::mcp::start_if_enabled(ctx.clone(), &runtime);
    }

    // Both modes: the runner sweeps its own workspace against its own
    // database, which no laptop-side project path ever reaches into. Racing
    // `resume_interrupted_features` is safe because a resumable feature keeps
    // its cache and every deletion is re-judged just before it happens.
    cache_reclaim.bind(ctx.clone());
    application::cache_sweep::start_cache_sweep(ctx.clone(), &runtime);

    // Before `start_socket`, which `src-tauri/src/lib.rs` calls once this
    // returns, so the first connection already answers what it is sent.
    if let Some(hub) = &ctx.hub {
        hub.bind_requests(Arc::new(application::hub_requests::HubRequests::new(
            Arc::new(ctx.clone()),
            application::hub_requests::HubRequestPorts {
                requests: ctx.hub_requests.clone(),
                policy: ctx.hub_policy.clone(),
                pins: hub_pins,
                passkeys: hub_passkeys,
                attachments: hub.attachments(),
                now_ms: Arc::new(crate::shared::time::now_ms),
            },
        )));
    }

    (ctx, hub_policy_writer)
}

/// Shares `app_version` with the context, so the version the desktop sets
/// after this returns is the one the Hub's consent page shows.
///
/// The socket is attached here but not started: its `Hello` announces that
/// same version, which is still unset until this returns, so the desktop
/// starts it (`src-tauri/src/lib.rs`) once the version is set.
#[cfg(feature = "keyring")]
fn hub_pairing(
    pins: Arc<dyn ports::hub::HubPinRepository>,
    policy: Arc<dyn ports::hub::HubPolicyReader>,
    app_version: AppVersion,
    socket: Option<adapters::hub::socket::SocketConfig>,
) -> Option<Arc<adapters::hub::HubPairing>> {
    let token = match adapters::hub::token_client::ReqwestTokenEndpoint::new() {
        Ok(token) => Arc::new(token),
        Err(e) => {
            tracing::warn!(error = %e, "Hub pairing is off: no HTTP client for /token");
            return None;
        }
    };
    let pairing = adapters::hub::HubPairing::new(
        pins,
        Arc::new(adapters::hub::keyring_store::KeyringSecretStore),
        token.clone(),
        policy,
    )
    .with_refresh_endpoint(token)
    .with_app_version(app_version);
    Some(Arc::new(match socket {
        Some(socket) => pairing.with_socket(socket),
        None => pairing,
    }))
}

/// No keyring, no pairing: a file in its place is what AGENTS.md §2 forbids.
#[cfg(not(feature = "keyring"))]
fn hub_pairing(
    _pins: Arc<dyn ports::hub::HubPinRepository>,
    _policy: Arc<dyn ports::hub::HubPolicyReader>,
    _app_version: AppVersion,
    _socket: Option<adapters::hub::socket::SocketConfig>,
) -> Option<Arc<adapters::hub::HubPairing>> {
    None
}

/// `run_events` and `wake` have to come from one
/// [`WakingRunEvents`](adapters::hub::socket::wake::WakingRunEvents), and
/// `snapshot_wake` has to be the one `ctx.projects` and `ctx.workflows` wake
/// through. With no dialer the desktop still pairs; it just never dials.
#[cfg(feature = "hub-socket")]
fn hub_socket(
    run_events: Arc<dyn ports::run_events::RunEventsPort>,
    wake: Arc<tokio::sync::Notify>,
    snapshots: Arc<dyn ports::hub::HubSnapshotSource>,
    snapshot_wake: Arc<tokio::sync::Notify>,
    runtime: tokio::runtime::Handle,
) -> Option<adapters::hub::socket::SocketConfig> {
    match adapters::hub::socket::ws_transport::WsSocketDialer::new() {
        Ok(dialer) => Some(adapters::hub::socket::SocketConfig {
            dialer: Arc::new(dialer),
            run_events,
            wake,
            snapshots,
            snapshot_wake,
            snapshot_debounce: std::time::Duration::from_secs(
                crate::domain::hub_socket::SNAPSHOT_DEBOUNCE_SECS,
            ),
            runtime,
            sleeper: Arc::new(adapters::hub::socket::TokioSleeper),
            jitter: adapters::hub::socket::os_jitter,
        }),
        Err(e) => {
            tracing::warn!(error = %e, "the Hub socket is off: no WebSocket client");
            None
        }
    }
}

#[cfg(not(feature = "hub-socket"))]
fn hub_socket(
    _run_events: Arc<dyn ports::run_events::RunEventsPort>,
    _wake: Arc<tokio::sync::Notify>,
    _snapshots: Arc<dyn ports::hub::HubSnapshotSource>,
    _snapshot_wake: Arc<tokio::sync::Notify>,
    _runtime: tokio::runtime::Handle,
) -> Option<adapters::hub::socket::SocketConfig> {
    None
}

#[cfg(test)]
#[path = "../../tests/infrastructure/composition.rs"]
mod tests;
