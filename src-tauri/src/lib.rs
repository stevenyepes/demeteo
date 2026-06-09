pub mod db;
pub mod terminal;
pub mod forward;
pub mod sftp;
pub mod domain;
pub mod ports;
pub mod adapters;
pub mod state;
pub mod ssh_util;
pub mod commands;

use state::{DatabaseState, ExecutionState, AgentExecutionState, NotificationState, AgentRegistryState};
use terminal::SessionState;
use forward::ForwardState;
use ports::agent_execution::AgentExecutionPort;
use ports::agent_runtime::AgentRuntime;
use ports::execution::ExecutionPort;
use ports::notification::NotificationPort;
use tauri::Manager;
use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            let app_data_dir = app.path().app_local_data_dir().expect("Failed to get local data dir");
            let conn = db::init_db(app_data_dir).expect("Failed to initialize database");

            let db_adapter = Arc::new(adapters::database::sqlite::SqliteAdapter::new(conn));
            let exec_inner: Arc<dyn ExecutionPort> =
                Arc::new(adapters::ssh::client::SshClientAdapter::new(db_adapter.clone()));
            let notif_adapter: Arc<dyn NotificationPort> = Arc::new(
                adapters::tauri_ui::notification::TauriNotificationAdapter::new(app.handle().clone()),
            );
            let agent_exec: Arc<dyn AgentExecutionPort> = Arc::new(
                adapters::agent::direct_execution::DirectExecutionPort::new(
                    exec_inner.clone(),
                ),
            );

            let agent_registry = Arc::new(
                adapters::agent::registry::AgentRegistry::new(vec![
                    Arc::new(adapters::agent::opencode::runtime())
                        as Arc<dyn AgentRuntime>,
                    Arc::new(adapters::agent::hermes::runtime())
                        as Arc<dyn AgentRuntime>,
                    Arc::new(adapters::agent::noop::NoopRuntime)
                        as Arc<dyn AgentRuntime>,
                ]),
            );

            app.manage(DatabaseState { db: db_adapter });
            app.manage(ExecutionState { exec: exec_inner });
            app.manage(AgentExecutionState { agent_exec: agent_exec.clone() });
            app.manage(NotificationState { notif: notif_adapter });
            app.manage(AgentRegistryState {
                registry: agent_registry,
                agent_exec,
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
                            if let Ok(mut chan) = active.channel.lock() {
                                let _ = chan.close();
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
            commands::app_session::get_app_session,
            commands::app_session::set_app_session,
            commands::app_session::delete_app_session,
            commands::thread_events::get_thread_events,
            commands::thread_events::append_thread_event,
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
            sftp::sftp_get_metadata
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
