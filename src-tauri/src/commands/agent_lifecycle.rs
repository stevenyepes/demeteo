use std::sync::Arc;
use tauri::{Emitter, State};
use crate::state::{
    DatabaseState, ExecutionState, AgentRegistryState,
    ThreadStatusChanged, EVENT_THREAD_STATUS_CHANGED, EVENT_AGENT_EVENT,
};
use crate::ports::db::DatabasePort;
use crate::ports::execution::ExecutionPort;
use crate::ports::agent_execution::AgentExecutionPort;

/// Build the `AgentContext` for a (thread, agent_kind) pair. Looks up
/// the machine's auth type (to pick local vs SSH transport) and the
/// thread's sandbox (to use as cwd). The `AcpRuntime` uses both.
/// Apply the persisted model (if any) to a session after a fresh spawn.
/// Silently ignores errors — the session may already have the correct model,
/// or the agent may not support runtime model switching.
async fn apply_thread_model(
    db: &dyn DatabasePort,
    session: &Arc<dyn crate::ports::agent_runtime::AgentSession>,
    thread_id: &str,
) {
    if let Ok(threads) = db.get_thread_sessions_for_thread(thread_id) {
        if let Some(thread) = threads.into_iter().next() {
            if let Some(ref model) = thread.model {
                let _ = session.set_config_option("model", model);
            }
        }
    }
}

pub(crate) fn build_agent_context(
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
        if machine.auth_type == "local" || thread.machine_id.is_empty() {
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

#[tauri::command]
pub async fn agent_start(
    registry_state: State<'_, AgentRegistryState>,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
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
            let _ = registry_state.registry.session_handle(&thread_id, &agent_kind).await;
            Ok("ok".into())
        }
        Err(crate::ports::agent_runtime::AgentStartError::NotFound(binary)) => {
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

#[tauri::command]
pub async fn agent_install_and_start(
    registry_state: State<'_, AgentRegistryState>,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
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

    if !runtime.is_available(&thread.machine_id) {
        return Err(format!("INSTALL_BUT_STILL_MISSING:{}", runtime.kind()));
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

#[tauri::command]
pub async fn agent_prompt(
    registry_state: State<'_, AgentRegistryState>,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    app: tauri::AppHandle,
    thread_id: String,
    agent_kind: String,
    text: String,
) -> Result<(), String> {
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

    // Re-apply persisted model selection on fresh sessions
    apply_thread_model(db_state.db.as_ref(), &session, &thread_id).await;

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

        if let Err(e) = db.update_thread_status(&tid, &final_status) {
            eprintln!("[agent_prompt] failed to update thread status in DB: {}", e);
        }

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
pub async fn agent_cancel(
    registry_state: State<'_, AgentRegistryState>,
    thread_id: String,
) -> Result<(), String> {
    if let Some(session) = registry_state.registry.session_handle_any(&thread_id).await {
        session.cancel().map_err(|e| format!("cancel: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn agent_restart(
    registry_state: State<'_, AgentRegistryState>,
    db_state: State<'_, DatabaseState>,
    thread_id: String,
) -> Result<(), String> {
    let registry = registry_state.registry.clone();
    let db = db_state.db.clone();
    let tid = thread_id.clone();
    registry.kill(&tid).await;
    let _ = db.clear_working_memory(&tid);
    Ok(())
}

/// Resolve a session handle for a thread, auto-spawning if needed.
/// This mirrors the pattern used by `agent_prompt` so that
/// `agent_get_session_info` / `agent_set_mode` / `agent_set_config_option`
/// work even after the session has been cleaned up between turns.
async fn resolve_session(
    registry_state: &State<'_, AgentRegistryState>,
    db_state: &State<'_, DatabaseState>,
    exec_state: &State<'_, ExecutionState>,
    thread_id: &str,
) -> Result<Arc<dyn crate::ports::agent_runtime::AgentSession>, String> {
    // Fast path: session already alive
    if let Some(session) = registry_state.registry.session_handle_any(thread_id).await {
        return Ok(session);
    }

    // Slow path: look up thread, get agent_kind, build context, spawn
    let thread = db_state
        .db
        .get_thread_sessions_for_thread(thread_id)?
        .into_iter()
        .next()
        .ok_or_else(|| format!("Thread not found: {}", thread_id))?;
    let agent_kind = thread
        .agent_kind
        .as_deref()
        .ok_or_else(|| format!("Thread {} has no agent configured", thread_id))?;
    let ctx = build_agent_context(
        db_state.db.as_ref(),
        exec_state.exec.clone(),
        thread_id,
        agent_kind,
        registry_state.agent_exec.clone(),
    )?;
    let session = registry_state
        .registry
        .get_or_spawn(thread_id, agent_kind, ctx)
        .await
        .map_err(|e| format!("Failed to start agent session: {}", e))?;

    // Re-apply persisted model on fresh spawns
    if let Some(ref model) = thread.model {
        let _ = session.set_config_option("model", model);
    }

    Ok(session)
}

#[tauri::command]
pub async fn agent_get_session_info(
    registry_state: State<'_, AgentRegistryState>,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    thread_id: String,
) -> Result<crate::domain::models::SessionInfo, String> {
    let session = resolve_session(&registry_state, &db_state, &exec_state, &thread_id).await?;
    Ok(session.session_info())
}

#[tauri::command]
pub async fn agent_set_mode(
    registry_state: State<'_, AgentRegistryState>,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    thread_id: String,
    mode_id: String,
) -> Result<(), String> {
    let session = resolve_session(&registry_state, &db_state, &exec_state, &thread_id).await?;
    session.set_mode(&mode_id)?;
    Ok(())
}

#[tauri::command]
pub async fn agent_set_config_option(
    registry_state: State<'_, AgentRegistryState>,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    thread_id: String,
    config_id: String,
    value: String,
) -> Result<(), String> {
    let session = resolve_session(&registry_state, &db_state, &exec_state, &thread_id).await?;
    session.set_config_option(&config_id, &value)?;

    // Persist model selection to DB so it survives session restarts
    if config_id == "model" {
        let _ = db_state.db.update_thread_model(&thread_id, &value);
    }

    Ok(())
}
