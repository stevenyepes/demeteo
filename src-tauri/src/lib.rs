pub mod db;
pub mod terminal;
pub mod forward;
pub mod sftp;
pub mod domain;
pub mod ports;
pub mod adapters;

use domain::models::{Machine, AgentProfile, ThreadSession};
use ports::db::DatabasePort;
use ports::execution::ExecutionPort;
use terminal::SessionState;
use forward::ForwardState;
use tauri::Manager;
use std::sync::Arc;

pub struct DatabaseState {
    pub db: Arc<dyn DatabasePort>,
}

pub struct ExecutionState {
    pub exec: Arc<dyn ExecutionPort>,
}

#[tauri::command]
fn get_machines(state: tauri::State<'_, DatabaseState>) -> Result<Vec<Machine>, String> {
    state.db.get_machines()
}

#[tauri::command]
fn add_machine(state: tauri::State<'_, DatabaseState>, machine: Machine) -> Result<(), String> {
    state.db.add_machine(machine)
}

#[tauri::command]
fn delete_machine(state: tauri::State<'_, DatabaseState>, id: String) -> Result<(), String> {
    state.db.delete_machine(&id)
}

#[tauri::command]
fn update_machine(state: tauri::State<'_, DatabaseState>, machine: Machine) -> Result<(), String> {
    state.db.update_machine(machine)
}

#[tauri::command]
fn get_agent_profiles(state: tauri::State<'_, DatabaseState>, machine_id: String) -> Result<Vec<AgentProfile>, String> {
    state.db.get_agent_profiles(&machine_id)
}

#[tauri::command]
fn add_agent_profile(state: tauri::State<'_, DatabaseState>, profile: AgentProfile) -> Result<(), String> {
    state.db.add_agent_profile(profile)
}

#[tauri::command]
fn delete_agent_profile(state: tauri::State<'_, DatabaseState>, id: String) -> Result<(), String> {
    state.db.delete_agent_profile(&id)
}

#[tauri::command]
fn get_thread_sessions(state: tauri::State<'_, DatabaseState>, machine_id: String) -> Result<Vec<ThreadSession>, String> {
    state.db.get_thread_sessions(&machine_id)
}

#[tauri::command]
fn add_thread_session(
    db_state: tauri::State<'_, DatabaseState>,
    exec_state: tauri::State<'_, ExecutionState>,
    thread: ThreadSession,
) -> Result<(), String> {
    // 1. Persist thread in database
    db_state.db.add_thread_session(thread.clone())?;

    // 2. If it is worktree mode, run remote provisioning via ExecutionPort
    if thread.mode == "worktree" {
        if let (Some(repo_path), Some(branch), Some(sandbox_path)) = (&thread.repo_path, &thread.branch, &thread.sandbox_path) {
            exec_state.exec.setup_worktree(&thread.machine_id, repo_path, branch, sandbox_path)?;
        } else {
            return Err("Missing worktree details (repo_path, branch, or sandbox_path)".to_string());
        }
    }
    Ok(())
}

/// Tests SSH connectivity using parameters passed directly from the UI form.
/// This avoids stale-state bugs where the DB has outdated auth settings that the
/// user has already changed in the form but not yet saved.
#[tauri::command]
fn test_ssh_connection(
    host: String,
    port: i32,
    username: String,
    auth_type: String,
    key_path: Option<String>,
    secret: Option<String>,
) -> Result<(), String> {
    use ssh2::Session;
    use std::net::TcpStream;

    if auth_type == "local" {
        return Ok(());
    }

    // Reject public key files early with a clear message
    if let Some(ref kp) = key_path {
        if kp.trim_end().ends_with(".pub") {
            return Err(
                "Key path points to a public key (.pub). Provide the private key instead (e.g. ~/.ssh/id_ed25519)."
                    .to_string(),
            );
        }
    }

    let tcp = TcpStream::connect(format!("{}:{}", host, port))
        .map_err(|e| format!("Cannot reach {}:{} — {}", host, port, e))?;

    let mut sess = Session::new()
        .map_err(|e| format!("Failed to create SSH session: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.handshake()
        .map_err(|e| format!("SSH handshake failed: {}", e))?;

    match auth_type.as_str() {
        "password" => {
            let password = secret.ok_or_else(|| "SSH password is required".to_string())?;
            sess.userauth_password(&username, &password)
                .map_err(|e| format!("Password authentication failed: {}", e))?;
        }
        "key" => {
            let key_path_str = key_path
                .as_deref()
                .ok_or_else(|| "Private key path is required".to_string())?;
            let resolved = if key_path_str.starts_with('~') {
                let home = std::env::var("HOME")
                    .map_err(|_| "HOME environment variable not set".to_string())?;
                key_path_str.replacen('~', &home, 1)
            } else {
                key_path_str.to_string()
            };
            let key_file = std::path::Path::new(&resolved);
            if !key_file.exists() {
                return Err(format!("Private key file not found: {}", resolved));
            }
            sess.userauth_pubkey_file(&username, None, key_file, secret.as_deref())
                .map_err(|e| format!("Key authentication failed: {}", e))?;
        }
        "agent" => {
            sess.userauth_agent(&username)
                .map_err(|e| format!("SSH agent authentication failed: {}", e))?;
        }
        other => return Err(format!("Unknown auth type: {}", other)),
    }

    let _ = sess.disconnect(None, "test complete", None);
    Ok(())
}

#[tauri::command]
fn update_thread_status(
    state: tauri::State<'_, DatabaseState>,
    id: String,
    status: String,
) -> Result<(), String> {
    state.db.update_thread_status(&id, &status)
}

#[tauri::command]
fn delete_thread_session(state: tauri::State<'_, DatabaseState>, id: String) -> Result<(), String> {
    state.db.delete_thread_session(&id)
}

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
            let exec_adapter = Arc::new(adapters::ssh::client::SshClientAdapter::new(db_adapter.clone()));
            
            app.manage(DatabaseState { db: db_adapter });
            app.manage(ExecutionState { exec: exec_adapter });
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
        .invoke_handler(tauri::generate_handler![
            get_machines,
            add_machine,
            delete_machine,
            update_machine,
            get_agent_profiles,
            add_agent_profile,
            delete_agent_profile,
            get_thread_sessions,
            add_thread_session,
            update_thread_status,
            delete_thread_session,
            test_ssh_connection,
            terminal::set_machine_secret,
            terminal::delete_machine_secret,
            terminal::start_terminal_session,
            terminal::write_terminal_session,
            terminal::resize_terminal_session,
            terminal::close_terminal_session,
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
