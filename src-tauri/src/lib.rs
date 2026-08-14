pub mod adapters;
pub mod commands;
pub mod composition;
pub mod env_path;
pub mod forward;
pub mod sftp;
pub mod terminal;
pub mod tray;

// `domain`, `ports`, `application`, `shared`, `error`, `infrastructure`,
// `paths`, `ssh_util`, `credential_cache`, and `state` live in `demeteo-core`
// (see docs/REMOTE_EXECUTION.md M0.1) — re-exported under their old
// names so every existing `crate::domain::X` / `demeteo_lib::domain::X` call
// site keeps working unchanged. `state` (`AppContext`) moved with them
// despite the design doc's original plan to keep it local, because
// `application/*` hard-depends on `AppContext` — leaving it behind would
// have created a cycle between the two crates. `adapters` is a partial
// re-export (most of it moved; `adapters::tauri_ui` stays local) — see
// `adapters/mod.rs`.
pub use demeteo_core::{
    application, credential_cache, db, domain, error, infrastructure, paths, ports, shared,
    ssh_util, state,
};

/// Compile-time release channel ("stable" or "nightly").
///
/// Defaults to `"stable"` when the `DEMETEO_RELEASE_CHANNEL` env var is not
/// set at build time. The AUR `PKGBUILD` `build()` honours this var so a
/// `nightly` build surfaces as `NIGHTLY` in the About screen without a
/// separate binary.
pub const RELEASE_CHANNEL: &str = match option_env!("DEMETEO_RELEASE_CHANNEL") {
    Some(c) => c,
    None => "stable",
};

use composition::{build_core_context, CoreConfig, ExecutionMode};
use forward::ForwardState;
use ports::notification::NotificationPort;
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

    // A GUI launch inherits the environment block Explorer held at logon, so
    // PATH here is not the PATH the user's own shell resolves against. See
    // `env_path` for why that has to be rebuilt from the registry rather than
    // trusted, and for what it still cannot recover.
    #[cfg(target_os = "windows")]
    {
        let inherited = std::env::var("PATH").unwrap_or_default();
        let appdata = std::env::var("APPDATA").ok();
        let local_appdata = std::env::var("LOCALAPPDATA").ok();
        let user_profile = std::env::var("USERPROFILE").ok();

        let appended: Vec<String> = env_path::windows_shim_dirs(
            appdata.as_deref(),
            local_appdata.as_deref(),
            user_profile.as_deref(),
        )
        .into_iter()
        .filter(|dir| dir.exists())
        .map(|dir| dir.to_string_lossy().into_owned())
        .collect();

        let enriched = env_path::compose_windows_path(
            &inherited,
            env_path::machine_environment_path().as_deref(),
            env_path::user_environment_path().as_deref(),
            &appended,
            &|name: &str| std::env::var(name).ok(),
        );

        if enriched != inherited {
            std::env::set_var("PATH", enriched);
        }
    }
}

#[cfg(target_os = "linux")]
fn configure_linux_gpu_env() {
    if std::env::var("DEMETEO_DISABLE_GPU").ok().as_deref() == Some("1") {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        eprintln!("[demeteo] GPU rendering disabled via DEMETEO_DISABLE_GPU");
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
        eprintln!("[demeteo] NVIDIA detected: GPU rendering enabled (explicit sync off)");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    enrich_env_path();

    #[cfg(target_os = "linux")]
    configure_linux_gpu_env();

    // Initialize structured logging. The log file lives next to demeteo.db
    // so `open ~/Library/…` finds both at once. RUST_LOG controls the filter;
    // if not set we default to INFO for this crate, WARN for everything else.
    //
    // The `_guard` ensures the non-blocking writer flushes before process exit.
    // Box::leak is the standard pattern for a guard that must live forever
    // in a Tauri app where there is no obvious drop point before process exit.
    let log_dir = {
        #[cfg(target_os = "macos")]
        {
            std::env::var("HOME")
                .ok()
                .map(|h| {
                    std::path::PathBuf::from(h)
                        .join("Library/Application Support/com.stvcloud.demeteo.dev")
                })
                .unwrap_or_else(|| std::env::temp_dir().join("demeteo"))
        }
        #[cfg(target_os = "linux")]
        {
            let base = std::env::var("XDG_DATA_HOME")
                .ok()
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::var("HOME")
                        .ok()
                        .map(|h| std::path::PathBuf::from(h).join(".local").join("share"))
                })
                .unwrap_or_else(std::env::temp_dir);
            base.join("com.stvcloud.demeteo")
        }
        #[cfg(target_os = "windows")]
        {
            std::env::var("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("com.stvcloud.demeteo")
        }
    };
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "[demeteo] warning: could not create log dir {}: {}",
            log_dir.display(),
            e
        );
    }
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "demeteo_lib=info,warn".parse().expect("static filter"));
    let file_appender =
        std::panic::catch_unwind(|| tracing_appender::rolling::daily(&log_dir, "demeteo.log"));
    match file_appender {
        Ok(appender) => {
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(non_blocking)
                .with_ansi(false)
                .try_init()
                .ok();
            Box::leak(Box::new(guard));
        }
        Err(_) => {
            eprintln!(
                "[demeteo] warning: file logging disabled (could not open {}); logs go to stderr only",
                log_dir.join("demeteo.log").display()
            );
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .try_init()
                .ok();
        }
    }

    // Startup banner so a stale binary is obvious in the Tauri dev
    // console. Bump the suffix whenever the bootstrap/step-executor
    // path resolution changes.
    eprintln!(
        "[demeteo] startup v{} ({}) — channel={} — paths/agent-target-dir fix active",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_NAME"),
        crate::RELEASE_CHANNEL,
    );

    tauri::Builder::default()
        // Must be the FIRST `.plugin(...)` registered: this is an upstream
        // requirement of tauri-plugin-single-instance to reliably intercept a
        // second launch on all platforms. Do not reorder below other plugins
        // or `.setup()`.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tracing::info!("second app instance launched; focusing existing window");
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_local_data_dir()
                .expect("Failed to get local data dir");
            eprintln!("[demeteo] data dir: {}", app_data_dir.display());

            // The unified event log (P1.13): decorate the Tauri emitter
            // with the local run-event recorder, so every narrative
            // DomainEvent is also appended to `run_events` (keyed by
            // feature id) and pushed as a `run_event` record. Late-bound:
            // the recorder's sink is the `run_events` repo built inside
            // `build_core_context`, wired right after it returns — events
            // fired during startup reconcile are forwarded live but not
            // recorded, same as the runner's `RunEventBridge` pattern.
            let run_event_recorder = Arc::new(
                demeteo_core::adapters::run_event_log::RunEventRecorder::new(Arc::new(
                    adapters::tauri_ui::notification::TauriNotificationAdapter::new(
                        app.handle().clone(),
                    ),
                )),
            );
            let notif_adapter: Arc<dyn NotificationPort> = run_event_recorder.clone();

            // Expose the notification port as Tauri state BEFORE it is moved into
            // `build_core_context`, so terminal.rs can read it via `try_state` and
            // route `awaiting_approval` transitions through the same gated
            // OS-notification pipeline (Terminal Agent Activity T2.6).
            app.manage(notif_adapter.clone());

            // `.setup()` runs synchronously (not polled as a task), so
            // there's no ambient "current" tokio runtime for the engine's
            // background tasks to spawn onto — pass Tauri's own runtime
            // handle explicitly instead (see build_core_context's doc).
            let runtime = tauri::async_runtime::handle().inner().clone();
            let ctx = build_core_context(
                CoreConfig {
                    app_data_dir: app_data_dir.clone(),
                    execution_mode: ExecutionMode::Router,
                },
                notif_adapter,
                runtime,
            );
            eprintln!("[demeteo] workspace dir: {}", ctx.workspace_dir.display());
            run_event_recorder.wire(ctx.run_events.clone());

            commands::workflows::seed_starter_workflows(&ctx.workflows);

            app.manage(ctx);
            app.manage(SessionState::default());
            app.manage(ForwardState::default());

            // Background poller that labels local terminals running a coding
            // agent (Claude/OpenCode/…), including ones launched by hand.
            terminal::spawn_agent_detector(app.handle().clone());

            // Background cadence sweep that resolves working ↔ awaiting_input
            // from each agent session's output timing and emits
            // `terminal-session-activity` on change (TERMINAL_ACTIVITY §4).
            terminal::spawn_activity_sweep(app.handle().clone());

            // Build the system tray (Show / Hide / Quit). A tray-backend
            // failure is logged and swallowed inside `build_tray`, so this
            // never aborts startup — hence the ignored result.
            let _ = tray::build_tray(app.handle());

            // Request OS-notification permission once here, on the main thread,
            // rather than lazily from the background emit path — otherwise a
            // never-decided permission would prompt (and re-prompt) off-thread on
            // every domain event. No-op on desktop Linux (already `Granted`).
            adapters::tauri_ui::notification::request_startup_permission(app.handle());

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
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide to tray only when background mode is ON *and* a tray icon
                // actually exists to restore/quit the window. When the tray
                // backend is unavailable (headless / minimal Linux), fall back
                // to the OFF path so the user is never left with a hidden window
                // and no way back (spec §5 Linux edge case).
                let hide_to_tray =
                    tray::run_in_background_enabled(window) && tray::tray_available(window);
                match tray::close_action(hide_to_tray) {
                    // Background mode ON: keep the process alive and just hide
                    // the window. Terminal sessions are deliberately left
                    // running (spec Constraint 4).
                    tray::CloseAction::HideToTray => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    // Background mode OFF: preserve the original behaviour —
                    // tear down every active terminal session, then let the
                    // close proceed and the process exit.
                    tray::CloseAction::Cleanup => {
                        tray::cleanup_terminal_sessions(window);
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
            commands::agent_config::list_agents,
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
            commands::app_session::get_app_info,
            commands::app_session::get_workspace_dir,
            commands::app_session::get_workspace_dir_setting,
            commands::app_session::set_workspace_dir_setting,
            commands::app_session::get_run_in_background,
            commands::app_session::set_run_in_background,
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
            terminal::rename_terminal_session,
            terminal::reconnect_terminal_session,
            terminal::report_terminal_screen_activity,
            forward::start_port_forward,
            forward::stop_port_forward,
            sftp::sftp_list_dir,
            sftp::sftp_read_file,
            sftp::sftp_write_file,
            sftp::sftp_get_metadata,
            commands::providers::validate_provider_pat,
            commands::providers::fetch_provider_repos,
            commands::providers::fetch_provider_groups,
            commands::providers::provider_create_repo,
            commands::providers::connect_provider_instance,
            commands::providers::list_provider_instances,
            commands::providers::delete_provider_instance,
            commands::project::create_project,
            commands::project::get_projects,
            commands::project::seed_sample_project,
            commands::project::update_project,
            commands::project::delete_project,
            commands::project::check_repos_dirty,
            commands::project::probe_project_commands,
            commands::project::get_repositories_for_project,
            commands::project::get_workspace_health,
            commands::project::get_project_by_id,
            commands::project::resolve_repo_dir,
            commands::project::list_terminal_locations,
            commands::project::list_terminal_branches,
            commands::project::create_terminal_worktree,
            commands::project::remove_terminal_worktree,
            commands::project::project_memory_list,
            commands::project::project_memory_upsert,
            commands::project::project_memory_delete,
            commands::memory::memory_agent_config_get,
            commands::memory::memory_agent_config_set,
            commands::memory::memory_agent_test_connection,
            commands::memory::memory_agent_list_models,
            commands::timeouts::get_agent_timeouts,
            commands::timeouts::set_agent_timeouts,
            commands::project::get_workflow_overrides,
            commands::project::set_workflow_override,
            commands::features::fetch_active_features,
            commands::features::start_feature,
            commands::remote_runner::remote_submit_run,
            commands::remote_runner::remote_list_mirrored_runs,
            commands::remote_runner::remote_reconcile_runs,
            commands::remote_runner::remote_refresh_run,
            commands::remote_runner::remote_run_for_feature,
            commands::remote_runner::remote_get_status,
            commands::remote_runner::remote_run_diff_url,
            commands::remote_runner::remote_stream_events,
            commands::remote_runner::remote_get_feature,
            commands::remote_runner::remote_list_steps,
            commands::remote_runner::remote_read_artifact,
            commands::remote_runner::remote_list_messages,
            commands::remote_runner::remote_get_worktree,
            commands::remote_install::remote_runner_status,
            commands::remote_install::remote_runner_local_check,
            commands::remote_install::remote_enable_runs,
            adapters::tauri_ui::runner_download::remote_runner_download,
            adapters::tauri_ui::runner_download::remote_runner_download_cancel,
            commands::remote_runner::remote_decide_gate,
            commands::remote_runner::remote_cancel_run,
            commands::remote_runner::remote_retry_step,
            commands::remote_runner::remote_replay_step,
            commands::remote_runner::remote_reinject_credentials,
            commands::features::feature_pause,
            commands::features::feature_resume,
            commands::features::feature_cancel,
            commands::features::feature_get,
            commands::features::artifact_body,
            commands::features::step_get,
            commands::features::step_attempts_list,
            commands::features::sequence_tasks_list,
            commands::features::step_list_for_run,
            commands::features::run_events_since,
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
            commands::workflows::feature_workflow_graph,
            commands::workflows::node_types_list,
            commands::workflows::workflow_lint,
            commands::workflows::workflow_save,
            commands::workflows::workflow_delete,
            commands::workflows::workflow_versions,
            commands::workflows::workflow_version_graph,
            commands::workflows::workflow_restore_version,
            commands::workflows::workflow_export,
            commands::workflows::workflow_import,
            commands::workflows::workflow_revert_to_default,
            commands::workflows::workflow_save_schedule,
            commands::bootstrap::bootstrap_project,
            commands::bootstrap::get_proposed_strategy,
            commands::bootstrap::save_project_settings,
            commands::create_project::begin_create_project,
            commands::create_project::submit_create_project_step,
            commands::create_project::go_back_create_project,
            commands::agent_config_probe::get_agent_models,
            commands::app_version::get_app_version,
            commands::pricing::pricing_list,
            commands::pricing::pricing_for,
            commands::mr_publisher::publish_mr,
            commands::mr_publisher::fetch_mr_state,
            commands::feature_lifecycle::feature_cleanup,
            commands::notifications::notifications_list,
            commands::attachments::feature_add_attachment,
            commands::attachments::feature_list_attachments,
            commands::attachments::attachment_read,
            commands::attachments::feature_remove_attachment,
            commands::attachments::attachment_stage_metadata,
            commands::notifications::notification_mark_read,
            commands::notifications::notification_unread_count
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    // What this branch decides is tested in `env_path`, on every platform.
    // What is left here is the part no unit test reaches: the registry reads
    // and the in-place env mutation, which have no return value to assert. So
    // this is a smoke test that the Windows-only path runs to completion, and
    // it is gated to Windows because that is the only place it exists.
    #[cfg(target_os = "windows")]
    #[test]
    fn enrich_env_path_runs_without_panicking_on_windows() {
        crate::enrich_env_path();
    }
}
