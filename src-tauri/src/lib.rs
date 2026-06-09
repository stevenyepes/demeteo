pub mod db;
pub mod terminal;
pub mod forward;
pub mod sftp;
pub mod domain;
pub mod ports;
pub mod adapters;

use domain::models::{AgentConfig, Machine, AgentProfile, ThreadSession, WorkingMemoryEntry};
use domain::action::AgentAction;
use ports::agent_execution::AgentExecutionPort;
use ports::agent_runtime::AgentRuntime;
use ports::db::DatabasePort;
use ports::execution::ExecutionPort;
use ports::notification::NotificationPort;
use terminal::SessionState;
use forward::ForwardState;
use serde::Serialize;
use tauri::Manager;
use std::sync::Arc;

pub struct DatabaseState {
    pub db: Arc<dyn DatabasePort>,
}

pub struct ExecutionState {
    pub exec: Arc<dyn ExecutionPort>,
}

pub struct AgentExecutionState {
    pub agent_exec: Arc<dyn AgentExecutionPort>,
}

pub struct NotificationState {
    pub notif: Arc<dyn NotificationPort>,
}

pub struct AgentRegistryState {
    pub registry: Arc<adapters::agent::registry::AgentRegistry>,
    pub agent_exec: Arc<dyn AgentExecutionPort>,
}

pub const EVENT_THREAD_STATUS_CHANGED: &str = "thread_status_changed";

#[derive(Serialize, Clone)]
pub struct ThreadStatusChanged {
    pub thread_id: String,
    pub status: String,
    pub reason: Option<String>,
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
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

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

    let addr = format!("{}:{}", host, port)
        .to_socket_addrs()
        .map_err(|e| format!("Failed to resolve host: {}", e))?
        .next()
        .ok_or_else(|| format!("No addresses for host: {}", host))?;
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("Cannot reach {}:{} (timeout after 5s) — {}", host, port, e))?;
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(10)));

    let mut sess = Session::new()
        .map_err(|e| format!("Failed to create SSH session: {}", e))?;
    sess.set_tcp_stream(tcp);
    sess.set_timeout(10_000);
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

#[tauri::command]
async fn request_action(
    state: tauri::State<'_, AgentExecutionState>,
    thread_id: String,
    machine_id: String,
    action: AgentAction,
) -> Result<ports::agent_execution::CommandOutcome, String> {
    // Must be async: `submit` may call `run_blocking` → `block_in_place`,
    // which panics if executed on a `spawn_blocking` thread (Tauri's default
    // for sync commands on Linux/WebKit). Running async puts us on a tokio
    // multi-thread worker where `block_in_place` is safe.
    let exec = state.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.submit(&thread_id, &machine_id, action))
        .await
        .map_err(|e| format!("request_action join: {}", e))?
}

#[tauri::command]
async fn approve_intercept(
    state: tauri::State<'_, AgentExecutionState>,
    intercept_id: String,
) -> Result<(), String> {
    let exec = state.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.approve(&intercept_id))
        .await
        .map_err(|e| format!("approve_intercept join: {}", e))?
}

#[tauri::command]
async fn reject_intercept(
    state: tauri::State<'_, AgentExecutionState>,
    intercept_id: String,
    feedback: String,
) -> Result<(), String> {
    let exec = state.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.reject(&intercept_id, feedback))
        .await
        .map_err(|e| format!("reject_intercept join: {}", e))?
}

// -------------------------------------------------------------------------
// Agent integration (Phase 7a)
// -------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AgentConfigView {
    pub kind: String,
    pub enabled: bool,
    pub available: bool,
    pub install_command: String,
}

#[tauri::command]
fn get_agent_configs(
    state: tauri::State<'_, DatabaseState>,
    registry_state: tauri::State<'_, AgentRegistryState>,
    machine_id: String,
) -> Result<Vec<AgentConfigView>, String> {
    let configured = state.db.get_agent_configs(&machine_id)?;
    // The user-facing list is the union of: configured-but-known, plus every
    // runtime the registry knows about that the user hasn't configured yet
    // (auto-added as disabled, so the toggle is visible). For v1 the
    // configured list already covers this; the registry gives us availability
    // and install_command on the existing entries.
    let runtime_kinds: Vec<&'static str> = registry_state
        .registry
        .runtimes()
        .iter()
        .map(|r| r.kind())
        .collect();
    let mut views: Vec<AgentConfigView> = Vec::new();
    for cfg in configured {
        let available = runtime_kinds
            .iter()
            .find(|k| **k == cfg.kind)
            .map(|k| {
                registry_state
                    .registry
                    .runtime_for(k)
                    .map(|r| r.is_available(&machine_id))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        let install_command = runtime_kinds
            .iter()
            .find(|k| **k == cfg.kind)
            .and_then(|k| registry_state.registry.runtime_for(k).map(|r| r.install_command().to_string()))
            .unwrap_or_default();
        views.push(AgentConfigView {
            kind: cfg.kind,
            enabled: cfg.enabled,
            available,
            install_command,
        });
    }
    Ok(views)
}

#[tauri::command]
fn set_agent_configs(
    state: tauri::State<'_, DatabaseState>,
    machine_id: String,
    agents: Vec<AgentConfig>,
) -> Result<(), String> {
    let json = serde_json::to_string(&agents).map_err(|e| e.to_string())?;
    state.db.set_agent_configs(&machine_id, &json)
}

#[tauri::command]
fn get_working_memory(
    state: tauri::State<'_, DatabaseState>,
    thread_id: String,
) -> Result<Vec<WorkingMemoryEntry>, String> {
    state.db.get_working_memory(&thread_id)
}

#[tauri::command]
fn clear_working_memory(
    state: tauri::State<'_, DatabaseState>,
    thread_id: String,
) -> Result<(), String> {
    state.db.clear_working_memory(&thread_id)
}

/// Build the `AgentContext` for a (thread, agent_kind) pair. Looks up
/// the machine's auth type (to pick local vs SSH transport) and the
/// thread's sandbox (to use as cwd). The `AcpRuntime` uses both.
fn build_agent_context(
    db: &dyn DatabasePort,
    exec: Arc<dyn ExecutionPort>,
    thread_id: &str,
    agent_kind: &str,
    agent_exec: Arc<dyn AgentExecutionPort>,
) -> Result<crate::ports::agent_runtime::AgentContext, String> {
    let threads = db.get_thread_sessions_for_thread(thread_id)?;
    let thread = threads
        .into_iter()
        .next()
        .ok_or_else(|| format!("Thread not found: {}", thread_id))?;
    let machines = db.get_machines()?;
    let machine = machines
        .into_iter()
        .find(|m| m.id == thread.machine_id)
        .ok_or_else(|| format!("Machine not found: {}", thread.machine_id))?;

    let cwd = thread.sandbox_path.clone().unwrap_or_else(|| {
        if thread.machine_id == "local" || thread.machine_id.is_empty() {
            std::env::var("HOME").unwrap_or_else(|_| ".".into())
        } else {
            ".".into()
        }
    });
    let binary = agent_kind.to_string();
    let args = vec!["acp".to_string()];

    Ok(crate::ports::agent_runtime::AgentContext {
        thread_id: thread_id.to_string(),
        machine_id: machine.id.clone(),
        binary,
        args,
        env: Default::default(),
        cwd,
        agent_exec,
        exec,
    })
}

/// Tauri command: `agent_start`. Per spec §5.3, this is the eager
/// first-thread-per-machine spawn. On NotFound we return the
/// install_command in the error string so the frontend can show the
/// consent modal verbatim.
#[tauri::command]
async fn agent_start(
    registry_state: tauri::State<'_, AgentRegistryState>,
    db_state: tauri::State<'_, DatabaseState>,
    exec_state: tauri::State<'_, ExecutionState>,
    thread_id: String,
    agent_kind: String,
) -> Result<String, String> {
    let ctx = build_agent_context(
        db_state.db.as_ref(),
        exec_state.exec.clone(),
        &thread_id,
        &agent_kind,
        registry_state.agent_exec.clone(),
    )?;
    let runtime = registry_state
        .registry
        .runtime_for(&agent_kind)
        .ok_or_else(|| format!("No runtime registered for agent kind '{}'", agent_kind))?;
    match runtime.start(ctx).await {
        Ok(_session) => {
            // The session lives in the registry now; the caller will
            // hit `agent_prompt` to drive a turn. We return the
            // session id-less OK so the frontend can flip status
            // away from "spawning".
            let _ = registry_state.registry.session_handle(&thread_id, &agent_kind).await;
            Ok("ok".into())
        }
        Err(crate::ports::agent_runtime::AgentStartError::NotFound(binary)) => {
            // Surface the install command so the consent modal can
            // show it verbatim. We return a structured error string
            // with a marker the frontend pattern-matches on.
            let install = registry_state
                .registry
                .runtime_for(&agent_kind)
                .map(|r| r.install_command().to_string())
                .unwrap_or_default();
            Err(format!("NOT_FOUND:{}:{}", binary, install))
        }
        Err(e) => Err(format!("agent_start failed: {}", e)),
    }
}

/// Tauri command: `agent_install_and_start`. Runs the official install
/// command on the target machine, re-checks availability, and spawns
/// the agent. On success the session lives in the registry.
#[tauri::command]
async fn agent_install_and_start(
    registry_state: tauri::State<'_, AgentRegistryState>,
    db_state: tauri::State<'_, DatabaseState>,
    exec_state: tauri::State<'_, ExecutionState>,
    thread_id: String,
    agent_kind: String,
) -> Result<String, String> {
    let threads = db_state.db.get_thread_sessions_for_thread(&thread_id)?;
    let thread = threads
        .into_iter()
        .next()
        .ok_or_else(|| format!("Thread not found: {}", thread_id))?;
    let runtime = registry_state
        .registry
        .runtime_for(&agent_kind)
        .ok_or_else(|| format!("No runtime registered for agent kind '{}'", agent_kind))?;
    let install_cmd = runtime.install_command();

    crate::adapters::agent::acp::install::run_official_install(
        exec_state.exec.as_ref(),
        &thread.machine_id,
        install_cmd,
    )
    .map_err(|e| format!("install failed: {}", e))?;

    // Re-check availability; if the binary is still missing, surface a
    // structured error.
    if !runtime.is_available(&thread.machine_id) {
        return Err(format!(
            "INSTALL_BUT_STILL_MISSING:{}",
            runtime.kind()
        ));
    }

    let ctx = build_agent_context(
        db_state.db.as_ref(),
        exec_state.exec.clone(),
        &thread_id,
        &agent_kind,
        registry_state.agent_exec.clone(),
    )?;
    runtime
        .start(ctx)
        .await
        .map_err(|e| format!("start after install: {}", e))?;
    let _ = registry_state.registry.session_handle(&thread_id, &agent_kind).await;
    Ok("ok".into())
}

/// Tauri command: `agent_prompt`. Per spec §6, the per-turn stream is
/// delivered to the frontend via the global Tauri event bus with the
/// `thread_id` in the payload. The frontend filters events by
/// `thread_id` (a small amount of filtering; see §6.3 for the spec's
/// own precedent of `command_executed` being global-with-thread-id).
///
/// We use the global event bus because Tauri v2's `ipc::Channel` is
/// JS→Rust (frontend→backend) — there's no native Rust→JS per-turn
/// stream. The spec example `Channel::from(rx)` (§6.2) is illustrative;
/// the implementation uses `app.emit(EVENT_AGENT_EVENT, payload)`,
/// matching the existing `permission_requested` / `command_executed`
/// pattern.
pub const EVENT_AGENT_EVENT: &str = "agent_event";

#[tauri::command]
async fn agent_prompt(
    registry_state: tauri::State<'_, AgentRegistryState>,
    db_state: tauri::State<'_, DatabaseState>,
    exec_state: tauri::State<'_, ExecutionState>,
    app: tauri::AppHandle,
    thread_id: String,
    agent_kind: String,
    text: String,
) -> Result<(), String> {
    use tauri::Emitter;
    let ctx = build_agent_context(
        db_state.db.as_ref(),
        exec_state.exec.clone(),
        &thread_id,
        &agent_kind,
        registry_state.agent_exec.clone(),
    )?;
    let session = registry_state
        .registry
        .get_or_spawn(&thread_id, &agent_kind, ctx)
        .await
        .map_err(|e| format!("agent_prompt: {}", e))?;

    let mut stream = session.prompt(&text);
    let tid = thread_id.clone();
    let db = db_state.db.clone();
    let app_clone = app.clone();
    tokio::spawn(async move {
        use tokio_stream::StreamExt;
        let mut final_status = "idle".to_string();
        let mut final_reason = None;

        let mut buffered_text = String::new();
        let mut last_emit = std::time::Instant::now();

        loop {
            // Buffer Text events and throttle/batch emissions to prevent event-loop flooding and crashes on Linux (WebKitGTK).
            let next_event = tokio::time::timeout(std::time::Duration::from_millis(30), stream.next()).await;
            match next_event {
                Ok(Some(ev)) => {
                    match ev {
                        crate::domain::agent_event::AgentEvent::Text { delta } => {
                            buffered_text.push_str(&delta);
                            if last_emit.elapsed() >= std::time::Duration::from_millis(50) {
                                if !buffered_text.is_empty() {
                                    let payload = serde_json::json!({
                                        "thread_id": tid,
                                        "event": crate::domain::agent_event::AgentEvent::Text { delta: std::mem::take(&mut buffered_text) },
                                    });
                                    if let Err(e) = app_clone.emit(EVENT_AGENT_EVENT, payload) {
                                        eprintln!("[agent_prompt] emit failed: {}", e);
                                        break;
                                    }
                                    last_emit = std::time::Instant::now();
                                }
                            }
                        }
                        other_event => {
                            // Flush any buffered text first
                            if !buffered_text.is_empty() {
                                let payload = serde_json::json!({
                                    "thread_id": tid,
                                    "event": crate::domain::agent_event::AgentEvent::Text { delta: std::mem::take(&mut buffered_text) },
                                });
                                if let Err(e) = app_clone.emit(EVENT_AGENT_EVENT, payload) {
                                    eprintln!("[agent_prompt] emit failed: {}", e);
                                    break;
                                }
                            }

                            match &other_event {
                                crate::domain::agent_event::AgentEvent::Error { message, .. } => {
                                    final_status = "error".to_string();
                                    final_reason = Some(message.clone());
                                }
                                _ => {}
                            }

                            let payload = serde_json::json!({
                                "thread_id": tid,
                                "event": other_event,
                            });
                            if let Err(e) = app_clone.emit(EVENT_AGENT_EVENT, payload) {
                                eprintln!("[agent_prompt] emit failed: {}", e);
                                break;
                            }
                            last_emit = std::time::Instant::now();
                        }
                    }
                }
                Ok(None) => {
                    // Stream complete
                    if !buffered_text.is_empty() {
                        let payload = serde_json::json!({
                            "thread_id": tid,
                            "event": crate::domain::agent_event::AgentEvent::Text { delta: std::mem::take(&mut buffered_text) },
                        });
                        let _ = app_clone.emit(EVENT_AGENT_EVENT, payload);
                    }
                    break;
                }
                Err(_) => {
                    // Timeout (no new events for 30ms) - flush any buffered text delta
                    if !buffered_text.is_empty() {
                        let payload = serde_json::json!({
                            "thread_id": tid,
                            "event": crate::domain::agent_event::AgentEvent::Text { delta: std::mem::take(&mut buffered_text) },
                        });
                        if let Err(e) = app_clone.emit(EVENT_AGENT_EVENT, payload) {
                            eprintln!("[agent_prompt] emit failed: {}", e);
                            break;
                        }
                        last_emit = std::time::Instant::now();
                    }
                }
            }
        }

        // Update thread status in the database
        if let Err(e) = db.update_thread_status(&tid, &final_status) {
            eprintln!("[agent_prompt] failed to update thread status in DB: {}", e);
        }

        // Emit thread status changed event
        let status_payload = ThreadStatusChanged {
            thread_id: tid,
            status: final_status,
            reason: final_reason,
        };
        if let Err(e) = app_clone.emit(EVENT_THREAD_STATUS_CHANGED, status_payload) {
            eprintln!("[agent_prompt] failed to emit thread status changed: {}", e);
        }
    });
    Ok(())
}

#[tauri::command]
async fn agent_cancel(
    registry_state: tauri::State<'_, AgentRegistryState>,
    thread_id: String,
) -> Result<(), String> {
    if let Some(session) = registry_state.registry.session_handle_any(&thread_id).await {
        session.cancel().map_err(|e| format!("cancel: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
async fn agent_restart(
    registry_state: tauri::State<'_, AgentRegistryState>,
    db_state: tauri::State<'_, DatabaseState>,
    thread_id: String,
) -> Result<(), String> {
    // Per Phase 9.3: kill the session (drops the in-flight turn + any
    // pending intercepts) and clear working memory. The frontend sets
    // status back to idle after.
    let registry = registry_state.registry.clone();
    let db = db_state.db.clone();
    let tid = thread_id.clone();
    registry.kill(&tid).await;
    let _ = db.clear_working_memory(&tid);
    Ok(())
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

            // Phase 7b: the real AcpRuntime-backed adapters (opencode,
            // hermes) are registered. The NoopRuntime stays available for
            // tests and as a fallback when the user hasn't enabled either
            // agent on a machine.
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
            request_action,
            approve_intercept,
            reject_intercept,
            get_agent_configs,
            set_agent_configs,
            get_working_memory,
            clear_working_memory,
            agent_start,
            agent_install_and_start,
            agent_prompt,
            agent_cancel,
            agent_restart,
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
