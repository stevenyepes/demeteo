use crate::domain::ids::ThreadId;
use crate::ports::db::ThreadPatch;
use crate::state::AppContext;
use std::sync::Arc;

pub const EVENT_THREAD_STATUS_CHANGED: &str = "thread_status_changed";
pub const EVENT_AGENT_EVENT: &str = "agent_event";

#[derive(serde::Serialize, Clone)]
pub struct ThreadStatusChanged {
    pub thread_id: String,
    pub status: String,
    pub reason: Option<String>,
}

pub async fn build_agent_context(
    ctx: &AppContext,
    thread_id: &str,
    agent_kind: &str,
) -> Result<crate::ports::agent_runtime::AgentContext, String> {
    let threads = ctx
        .threads
        .get_thread_sessions_for_thread(&ThreadId::from(thread_id.to_string()))?;
    let thread = threads
        .into_iter()
        .next()
        .ok_or_else(|| format!("Thread not found: {}", thread_id))?;
    let machines = ctx.machines.get_machines()?;
    let machine = machines
        .into_iter()
        .find(|m| m.id == thread.machine_id)
        .ok_or_else(|| format!("Machine not found: {}", thread.machine_id))?;

    // No sandbox: the session runs in the machine's home, never in the
    // GUI process's cwd — that is the app bundle, and on Windows often
    // a system directory the agent would then write into.
    let cwd = match thread.sandbox_path.clone() {
        Some(path) => path,
        None if machine.auth_type == "local" || thread.machine_id.is_empty() => ctx
            .exec
            .resolve_home(machine.id.0.as_str())
            .await
            .map_err(|e| format!("Cannot resolve a working directory for this session: {}", e))?,
        None => ".".into(),
    };
    let binary = agent_kind.to_string();
    // Machine-aware HOME: the local-auth interactive terminal gets
    // the GUI's HOME; a remote machine gets the value cached on the
    // SSH adapter (probed via `printf %s "$HOME"`). See
    // `agent_base_env` for the full rationale.
    let env =
        crate::ports::agent_runtime::agent_base_env(ctx.exec.as_ref(), machine.id.0.as_str()).await;

    Ok(crate::ports::agent_runtime::AgentContext {
        thread_id: thread_id.to_string(),
        machine_id: machine.id.0.clone(),
        binary,
        args: vec![],
        env,
        cwd,
        model: None,
        effort: None,
        title: None,
        agent_exec: ctx.agent_exec.clone(),
        exec: ctx.exec.clone(),
        permissions: crate::domain::permission::PermissionProfile::all_allow(),
        bare_mode: false,
        tool_allowlist: None,
        max_turns: None,
        // Interactive session: no budget guardrail.
        max_budget_usd: None,
    })
}

pub async fn apply_thread_model(
    threads_repo: &dyn crate::ports::db::ThreadRepository,
    session: &Arc<dyn crate::ports::agent_runtime::AgentSession>,
    thread_id: &str,
) {
    if let Ok(threads) =
        threads_repo.get_thread_sessions_for_thread(&ThreadId::from(thread_id.to_string()))
    {
        if let Some(thread) = threads.into_iter().next() {
            if let Some(ref model) = thread.model {
                match session.set_config_option("model", model) {
                    Ok(_) => println!(
                        "[apply_thread_model] set_config_option model to '{}' succeeded",
                        model
                    ),
                    Err(e) => eprintln!(
                        "[apply_thread_model] set_config_option model to '{}' failed: {}",
                        model, e
                    ),
                }
            }
        }
    }
}

pub async fn start_with_install(
    ctx: &AppContext,
    thread_id: String,
    agent_kind: String,
) -> Result<String, String> {
    let threads = ctx
        .threads
        .get_thread_sessions_for_thread(&ThreadId::from(thread_id.clone()))?;
    let thread = threads
        .into_iter()
        .next()
        .ok_or_else(|| format!("Thread not found: {}", thread_id))?;
    let runtime = ctx
        .registry
        .runtime_for(&agent_kind)
        .ok_or_else(|| format!("No runtime registered for agent kind '{}'", agent_kind))?;
    let install_cmd = runtime.install_command();

    crate::adapters::agent::install::run_official_install(
        ctx.exec.as_ref(),
        thread.machine_id.as_str(),
        install_cmd,
    )
    .await
    .map_err(|e| format!("install failed: {}", e))?;

    if !runtime
        .availability(ctx.exec.as_ref(), thread.machine_id.as_str())
        .await
        .is_installed()
    {
        return Err(format!("INSTALL_BUT_STILL_MISSING:{}", runtime.kind()));
    }

    let agent_ctx = build_agent_context(ctx, &thread_id, &agent_kind).await?;
    runtime
        .start(agent_ctx)
        .await
        .map_err(|e| format!("start after install: {}", e))?;
    let _ = ctx.registry.session_handle(&thread_id, &agent_kind).await;
    Ok("ok".into())
}

pub async fn prompt<F>(
    ctx: &AppContext,
    thread_id: String,
    agent_kind: String,
    text: String,
    emit_fn: F,
) -> Result<(), String>
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    let agent_ctx = build_agent_context(ctx, &thread_id, &agent_kind).await?;
    let session = ctx
        .registry
        .get_or_spawn(&thread_id, &agent_kind, agent_ctx)
        .await
        .map_err(|e| format!("agent_prompt: {}", e))?;

    // Re-apply persisted model selection on fresh sessions
    apply_thread_model(ctx.threads.as_ref(), &session, &thread_id).await;

    let mut stream = session.prompt(&text);
    let tid = thread_id.clone();
    let db = ctx.threads.clone();
    tokio::spawn(async move {
        use tokio_stream::StreamExt;
        let mut final_status = "idle".to_string();
        let mut final_reason = None;

        let mut buffered_text = String::new();
        let mut last_emit = std::time::Instant::now();

        loop {
            let next_event =
                tokio::time::timeout(std::time::Duration::from_millis(30), stream.next()).await;
            match next_event {
                Ok(Some(ev)) => match ev {
                    crate::domain::agent_event::AgentEvent::Text { delta } => {
                        buffered_text.push_str(&delta);
                        if last_emit.elapsed() >= std::time::Duration::from_millis(50)
                            && !buffered_text.is_empty()
                        {
                            let payload = serde_json::json!({
                                "thread_id": tid,
                                "event": crate::domain::agent_event::AgentEvent::Text { delta: std::mem::take(&mut buffered_text) },
                            });
                            emit_fn(EVENT_AGENT_EVENT, payload);
                            last_emit = std::time::Instant::now();
                        }
                    }
                    other_event => {
                        if !buffered_text.is_empty() {
                            let payload = serde_json::json!({
                                "thread_id": tid,
                                "event": crate::domain::agent_event::AgentEvent::Text { delta: std::mem::take(&mut buffered_text) },
                            });
                            emit_fn(EVENT_AGENT_EVENT, payload);
                        }

                        if let crate::domain::agent_event::AgentEvent::Error { message, .. } =
                            &other_event
                        {
                            if other_event.ends_turn() {
                                final_status = "error".to_string();
                                final_reason = Some(message.clone());
                            }
                        }

                        let payload = serde_json::json!({
                            "thread_id": tid,
                            "event": other_event,
                        });
                        emit_fn(EVENT_AGENT_EVENT, payload);
                        last_emit = std::time::Instant::now();
                    }
                },
                Ok(None) => {
                    if !buffered_text.is_empty() {
                        let payload = serde_json::json!({
                            "thread_id": tid,
                            "event": crate::domain::agent_event::AgentEvent::Text { delta: std::mem::take(&mut buffered_text) },
                        });
                        emit_fn(EVENT_AGENT_EVENT, payload);
                    }
                    break;
                }
                Err(_) => {
                    if !buffered_text.is_empty() {
                        let payload = serde_json::json!({
                            "thread_id": tid,
                            "event": crate::domain::agent_event::AgentEvent::Text { delta: std::mem::take(&mut buffered_text) },
                        });
                        emit_fn(EVENT_AGENT_EVENT, payload);
                        last_emit = std::time::Instant::now();
                    }
                }
            }
        }

        if let Err(e) = db.update_thread(
            &ThreadId::from(tid.clone()),
            &ThreadPatch {
                status: Some(final_status.clone()),
                ..Default::default()
            },
        ) {
            eprintln!("[agent_prompt] failed to update thread status in DB: {}", e);
        }

        let status_payload = ThreadStatusChanged {
            thread_id: tid,
            status: final_status,
            reason: final_reason,
        };
        match serde_json::to_value(status_payload) {
            Ok(value) => emit_fn(EVENT_THREAD_STATUS_CHANGED, value),
            Err(err) => eprintln!("[agent_prompt] failed to serialize status payload: {}", err),
        }
    });
    Ok(())
}
