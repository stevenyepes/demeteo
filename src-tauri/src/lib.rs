pub mod adapters;
pub mod application;
pub mod commands;
pub mod composition;
pub mod credential_cache;
pub mod db;
pub mod domain;
pub mod error;
pub mod forward;
pub mod infrastructure;
pub mod paths;
pub mod ports;
pub mod sftp;
pub mod shared;
pub mod ssh_util;
pub mod state;
pub mod terminal;

use forward::ForwardState;
use ports::agent_execution::AgentExecutionPort;
use ports::agent_runtime::AgentRuntime;
use ports::execution::ExecutionPort;
use ports::notification::NotificationPort;
use state::AppContext;
use std::sync::Arc;
use tauri::Manager;
use terminal::SessionState;

fn enrich_env_path() {
    // Enrich local PATH so coding agents installed in homebrew, cargo, npm-global, etc.
    // are discoverable by Tauri GUI process on macOS/Linux.
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(current_path) = std::env::var("PATH") {
            let mut paths: Vec<std::path::PathBuf> = std::env::split_paths(&current_path).collect();
            let home = std::env::var("HOME").unwrap_or_default();

            let mut additional_paths = vec![
                std::path::PathBuf::from("/opt/homebrew/bin"),
                std::path::PathBuf::from("/usr/local/bin"),
                std::path::PathBuf::from("/usr/bin"),
                std::path::PathBuf::from("/bin"),
                std::path::PathBuf::from("/usr/sbin"),
                std::path::PathBuf::from("/sbin"),
            ];

            if !home.is_empty() {
                additional_paths.push(std::path::PathBuf::from(format!("{}/.cargo/bin", home)));
                additional_paths.push(std::path::PathBuf::from(format!("{}/.local/bin", home)));
                additional_paths.push(std::path::PathBuf::from(format!(
                    "{}/.npm-global/bin",
                    home
                )));
                additional_paths.push(std::path::PathBuf::from(format!("{}/.opencode/bin", home)));
                // Also common nvm node versions paths
                additional_paths.push(std::path::PathBuf::from(format!(
                    "{}/.nvm/versions/node",
                    home
                )));
            }

            let mut changed = false;
            for p in additional_paths {
                if p.exists() && !paths.contains(&p) {
                    paths.push(p);
                    changed = true;
                }
            }

            if changed {
                if let Ok(new_path) = std::env::join_paths(paths) {
                    std::env::set_var("PATH", new_path);
                }
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    enrich_env_path();

    // Startup banner so a stale binary is obvious in the Tauri dev
    // console. Bump the suffix whenever the bootstrap/step-executor
    // path resolution changes.
    eprintln!(
        "[demeteo] startup v{} ({}) — paths/agent-target-dir fix active",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_NAME"),
    );

    // WebKitGTK on Wayland frequently dispatches a Gdk protocol error
    // (Error 71) on the host process. Disabling the DMA-BUF renderer
    // and accelerated compositing avoids the crash while allowing the
    // app to run natively under Wayland with correct UI scaling.
    #[cfg(target_os = "linux")]
    {
        if std::env::var("GDK_BACKEND").is_err() {
            std::env::set_var("GDK_BACKEND", "wayland,x11");
        }
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        if std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").is_err() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_local_data_dir()
                .expect("Failed to get local data dir");
            let conn = db::init_db(app_data_dir.clone()).expect("Failed to initialize database");

            let db_adapter = Arc::new(
                adapters::database::SqliteAdapter::new(conn)
                    .expect("Failed to initialize database adapter"),
            );
            let machines_repo: Arc<dyn crate::ports::db::MachineRepository> = db_adapter.clone();
            let projects_repo: Arc<dyn crate::ports::db::ProjectRepository> = db_adapter.clone();
            let features_repo: Arc<dyn crate::ports::db::FeatureRepository> = db_adapter.clone();
            let workflows_repo: Arc<dyn crate::ports::db::WorkflowRepository> = db_adapter.clone();
            let gates_repo: Arc<dyn crate::ports::db::GateRepository> = db_adapter.clone();
            let app_settings_repo: Arc<dyn crate::ports::db::AppSettingsRepository> =
                db_adapter.clone();
            let memory_repo: Arc<dyn crate::ports::memory::ProjectMemoryPort> = db_adapter.clone();
            let threads_repo: Arc<dyn crate::ports::db::ThreadRepository> = db_adapter.clone();
            let merge_audit_repo: Arc<dyn crate::ports::db::MergeAuditRepository> =
                db_adapter.clone();

            commands::workflows::seed_starter_workflows(&workflows_repo);
            let ssh_adapter: Arc<dyn ExecutionPort> = Arc::new(
                adapters::ssh::client::SshClientAdapter::new(machines_repo.clone()),
            );
            let local_adapter: Arc<dyn ExecutionPort> =
                Arc::new(adapters::local::execution::LocalSubprocessAdapter::new());
            let exec_inner: Arc<dyn ExecutionPort> =
                Arc::new(adapters::router::RouterExecutionPort::new(
                    machines_repo.clone(),
                    ssh_adapter,
                    local_adapter,
                ));
            let notif_adapter: Arc<dyn NotificationPort> = Arc::new(
                adapters::tauri_ui::notification::TauriNotificationAdapter::new(
                    app.handle().clone(),
                ),
            );
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
                ));

            let worktree_ops = Arc::new(adapters::worktree::git_ops::GitOpsHelper::new(
                app_settings_repo.clone(),
                exec_inner.clone(),
            ));

            let provider_http =
                Arc::new(adapters::provider_http::ReqwestProviderHttpAdapter::new());

            // Merge executor — owns the SQL audit table + the
            // structured conflict-report shape. Wired here so the
            // feature_sync command and the existing subtask→feature
            // merge share the same conflict-detection code path.
            let merge_executor: Arc<dyn ports::merge::MergeExecutor> = {
                let git_ops_for_merge = adapters::worktree::git_ops::GitOpsHelper::new(
                    app_settings_repo.clone(),
                    exec_inner.clone(),
                );
                Arc::new(adapters::merge::SqliteMergeExecutor::new(
                    merge_audit_repo.clone(),
                    git_ops_for_merge,
                    exec_inner.clone(),
                ))
            };

            // Build the DagStepExecutor before AppContext to avoid a
            // circular dependency (the executor contains sub-port Arcs;
            // AppContext contains the executor's Arc).
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
                    agent_registry.clone(),
                    notif_adapter.clone(),
                    agent_exec.clone(),
                    exec_inner.clone(),
                    merge_executor.clone(),
                    artifact_store,
                    app_data_dir.clone(),
                ));
                exec.startup_watchdog();
                exec
            };

            // Start workflow scheduler background task
            adapters::scheduler::start_scheduler(
                workflows_repo.clone(),
                step_executor_adapter.clone(),
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
                merge_audit: merge_audit_repo,
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
                app_data_dir: app_data_dir.clone(),
            });
            app.manage(SessionState::default());
            app.manage(ForwardState::default());

            // Set 1.25x zoom on Linux to offset the container 1x scaling fallback
            #[cfg(target_os = "linux")]
            {
                if let Some(webview) = app.get_webview_window("main") {
                    let _ = webview.set_zoom(1.25);
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(state) = window.try_state::<terminal::SessionState>() {
                    if let Ok(sessions) = state.sessions.lock() {
                        for (_, active) in sessions.iter() {
                            match &active.write_sink {
                                terminal::WriteSink::Ssh(ch) => {
                                    if let Ok(mut chan) = ch.lock() {
                                        let _ = chan.close();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::machine::get_machines,
            commands::machine::add_machine,
            commands::machine::delete_machine,
            commands::machine::update_machine,
            commands::machine::test_machine_connection,
            commands::agent_profile::get_agent_profiles,
            commands::agent_profile::add_agent_profile,
            commands::agent_profile::delete_agent_profile,
            commands::thread::get_thread_sessions,
            commands::thread::add_thread_session,
            commands::thread::update_thread_status,
            commands::thread::delete_thread_session,
            commands::ssh::test_ssh_connection,
            commands::agent_exec::request_action,
            commands::agent_exec::approve_intercept,
            commands::agent_exec::reject_intercept,
            commands::agent_config::get_agent_configs,
            commands::agent_config::set_agent_configs,
            commands::agent_config::get_working_memory,
            commands::agent_config::clear_working_memory,
            commands::agent_lifecycle::agent_start,
            commands::agent_lifecycle::agent_install_and_start,
            commands::agent_lifecycle::agent_prompt,
            commands::agent_lifecycle::agent_cancel,
            commands::agent_lifecycle::agent_restart,
            commands::agent_lifecycle::agent_get_session_info,
            commands::agent_lifecycle::agent_set_mode,
            commands::agent_lifecycle::agent_set_config_option,
            commands::app_session::get_app_session,
            commands::app_session::set_app_session,
            commands::app_session::delete_app_session,
            commands::messages::get_messages,
            commands::messages::append_message,
            terminal::set_machine_secret,
            terminal::delete_machine_secret,
            terminal::start_terminal_session,
            terminal::write_terminal_session,
            terminal::resize_terminal_session,
            terminal::close_terminal_session,
            terminal::list_terminal_sessions,
            terminal::close_machine_sessions,
            terminal::attach_terminal_session,
            terminal::detach_terminal_session,
            forward::start_port_forward,
            forward::stop_port_forward,
            sftp::sftp_list_dir,
            sftp::sftp_read_file,
            sftp::sftp_write_file,
            sftp::sftp_get_metadata,
            commands::providers::validate_provider_pat,
            commands::providers::fetch_provider_repos,
            commands::providers::connect_provider_instance,
            commands::providers::list_provider_instances,
            commands::providers::delete_provider_instance,
            commands::project::create_project,
            commands::project::get_projects,
            commands::project::seed_sample_project,
            commands::project::update_project,
            commands::project::delete_project,
            commands::project::check_repos_dirty,
            commands::project::get_repositories_for_project,
            commands::project::get_workspace_health,
            commands::project::get_project_by_id,
            commands::project::resolve_repo_dir,
            commands::project::project_memory_list,
            commands::project::project_memory_upsert,
            commands::project::project_memory_delete,
            commands::features::fetch_active_features,
            commands::features::start_feature,
            commands::features::feature_pause,
            commands::features::feature_resume,
            commands::features::feature_cancel,
            commands::features::feature_get,
            commands::features::step_get,
            commands::features::step_list_for_run,
            commands::features::gate_pending_for_run,
            commands::features::gate_decide,
            commands::features::step_retry,
            commands::features::replay_from_step,
            commands::features::feature_sync,
            commands::features::feature_resolve_sync_conflicts,
            commands::workflows::workflow_list,
            commands::workflows::workflow_get,
            commands::workflows::workflow_create,
            commands::workflows::workflow_update,
            commands::workflows::workflow_delete,
            commands::workflows::workflow_versions,
            commands::workflows::workflow_export,
            commands::workflows::workflow_import,
            commands::workflows::workflow_revert_to_default,
            commands::workflows::workflow_save_schedule,
            commands::bootstrap::bootstrap_project,
            commands::bootstrap::get_proposed_strategy,
            commands::bootstrap::save_project_settings,
            commands::agent_config_probe::get_agent_models,
            commands::pricing::pricing_list,
            commands::pricing::pricing_for,
            commands::mr_publisher::publish_mr,
            commands::mr_publisher::fetch_mr_state,
            commands::feature_lifecycle::feature_cleanup
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
