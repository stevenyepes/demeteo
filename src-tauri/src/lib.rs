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

use tauri::Manager;

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

#[cfg(target_os = "linux")]
fn configure_linux_gpu_env() {
    if std::env::var("DEMETEO_DISABLE_GPU").ok().as_deref() == Some("1") {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        tracing::info!("GPU rendering disabled via DEMETEO_DISABLE_GPU");
        return;
    }

    let is_nvidia = std::path::Path::new("/proc/driver/nvidia/version").exists();
    if is_nvidia {
        for (k, v) in [
            ("GBM_BACKEND", "nvidia-drm"),
            ("__GLX_VENDOR_LIBRARY_NAME", "nvidia"),
            ("__NV_DISABLE_EXPLICIT_SYNC", "1"),
        ] {
            if std::env::var(k).is_err() {
                std::env::set_var(k, v);
            }
        }
        tracing::info!("NVIDIA detected: GPU rendering enabled (explicit sync off)");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    enrich_env_path();

    #[cfg(target_os = "linux")]
    configure_linux_gpu_env();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        name = env!("CARGO_PKG_NAME"),
        "startup — paths/agent-target-dir fix active"
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            composition::setup_app_state(app)?;

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
                                terminal::WriteSink::LocalPty(_) => {
                                    // Local PTY child is killed when keepalive drops
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
            commands::app_session::get_workspace_dir,
            commands::app_session::get_workspace_dir_setting,
            commands::app_session::set_workspace_dir_setting,
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
            commands::memory::memory_agent_config_get,
            commands::memory::memory_agent_config_set,
            commands::memory::memory_agent_test_connection,
            commands::memory::memory_agent_list_models,
            commands::project::get_workflow_overrides,
            commands::project::set_workflow_override,
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
            commands::features::feature_get_worktree,
            commands::git::git_changed_files,
            commands::git::git_file_at_ref,
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
            commands::feature_lifecycle::feature_cleanup,
            commands::notifications::notifications_list,
            commands::notifications::notification_mark_read,
            commands::notifications::notification_unread_count
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
