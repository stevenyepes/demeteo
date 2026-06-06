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

/// Persists a thread session in the database only (skips the remote worktree
/// provisioning step). Used by the frontend to seed demo data that mirrors
/// the design mockup without requiring a live SSH target.
#[tauri::command]
fn seed_thread_session(state: tauri::State<'_, DatabaseState>, thread: ThreadSession) -> Result<(), String> {
    state.db.add_thread_session(thread)
}

/// One-shot seeding command for the mockup demo state. Idempotent: returns
/// `false` if the demo machine is already present, `true` if data was just
/// inserted.
#[tauri::command]
fn seed_demo_data(state: tauri::State<'_, DatabaseState>) -> Result<bool, String> {
    let existing = state.db.get_machines().map_err(|e| e.to_string())?;
    if existing.iter().any(|m| m.id == "prod-db-cluster") {
        return Ok(false);
    }

    let demo_machines = vec![
        Machine {
            id: "prod-db-cluster".to_string(),
            name: "prod-db-cluster".to_string(),
            host: "10.0.5.12".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_type: "key".to_string(),
            key_path: Some("~/.ssh/id_rsa".to_string()),
            agents: Some(r#"["OpenCode","Claude Code"]"#.to_string()),
            auto_approved_rules: Some(r#"["^git status$","^cat .*"]"#.to_string()),
        },
        Machine {
            id: "staging-api".to_string(),
            name: "staging-api".to_string(),
            host: "192.168.1.5".to_string(),
            port: 22,
            username: "admin".to_string(),
            auth_type: "key".to_string(),
            key_path: Some("~/.ssh/id_rsa".to_string()),
            agents: Some(r#"["Hermes"]"#.to_string()),
            auto_approved_rules: Some(r#"["^git status$","^cat .*"]"#.to_string()),
        },
        Machine {
            id: "local-macbook".to_string(),
            name: "local-macbook".to_string(),
            host: "localhost".to_string(),
            port: 22,
            username: "dev".to_string(),
            auth_type: "local".to_string(),
            key_path: None,
            agents: Some(r#"["Claude Code","Hermes"]"#.to_string()),
            auto_approved_rules: Some(r#"["^git status$","^cat .*"]"#.to_string()),
        },
    ];

    for m in demo_machines {
        state.db.add_machine(m).map_err(|e| e.to_string())?;
    }

    let threads = vec![
        ThreadSession {
            id: "t1_prod-db-cluster".to_string(),
            machine_id: "prod-db-cluster".to_string(),
            title: "Implement OAuth2".to_string(),
            mode: "worktree".to_string(),
            branch: Some("feature/agent-oauth".to_string()),
            repo_path: Some("/home/ubuntu/project".to_string()),
            sandbox_path: Some(
                "/home/ubuntu/project/.demeteo/worktrees/feature-agent-oauth".to_string(),
            ),
            status: "pending_approval".to_string(),
        },
        ThreadSession {
            id: "t2_prod-db-cluster".to_string(),
            machine_id: "prod-db-cluster".to_string(),
            title: "Analyze syslog memory leak".to_string(),
            mode: "adhoc".to_string(),
            branch: None,
            repo_path: None,
            sandbox_path: None,
            status: "idle".to_string(),
        },
        ThreadSession {
            id: "t3_prod-db-cluster".to_string(),
            machine_id: "prod-db-cluster".to_string(),
            title: "Update Dockerfile".to_string(),
            mode: "worktree".to_string(),
            branch: Some("feature/docker-fix".to_string()),
            repo_path: Some("/home/ubuntu/project".to_string()),
            sandbox_path: Some(
                "/home/ubuntu/project/.demeteo/worktrees/feature-docker-fix".to_string(),
            ),
            status: "running".to_string(),
        },
    ];

    for thread in threads {
        state.db.add_thread_session(thread).map_err(|e| e.to_string())?;
    }

    Ok(true)
}

#[tauri::command]
fn delete_thread_session(state: tauri::State<'_, DatabaseState>, id: String) -> Result<(), String> {
    state.db.delete_thread_session(&id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK on Wayland frequently dispatches a Gdk protocol error
    // (Error 71) on the host process, which in turn causes Vite's esbuild
    // service to be torn down mid-request ("The service was stopped").
    // Forcing the GDK backend to X11 makes the webview route through the
    // stable XWayland shim and avoids the error entirely.
    #[cfg(target_os = "linux")]
    {
        if std::env::var("GDK_BACKEND").is_err() {
            std::env::set_var("GDK_BACKEND", "x11");
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
            seed_thread_session,
            seed_demo_data,
            delete_thread_session,
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
