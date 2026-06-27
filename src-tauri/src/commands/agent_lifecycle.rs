use crate::domain::ids::ThreadId;
use crate::error::AppError;
use crate::ports::db::ThreadPatch;
use crate::state::AppContext;
use std::sync::Arc;
use tauri::{Emitter, State};

fn build_agent_context(
    ctx: &AppContext,
    thread_id: &str,
    agent_kind: &str,
) -> Result<crate::ports::agent_runtime::AgentContext, AppError> {
    crate::application::agents::build_agent_context(ctx, thread_id, agent_kind)
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn agent_start(
    ctx: State<'_, AppContext>,
    thread_id: String,
    agent_kind: String,
) -> Result<String, AppError> {
    let agent_ctx = build_agent_context(&ctx, &thread_id, &agent_kind)?;
    let runtime = ctx
        .registry
        .runtime_for(&agent_kind)
        .ok_or_else(|| format!("No runtime registered for agent kind '{}'", agent_kind))?;
    match runtime.start(agent_ctx).await {
        Ok(_session) => {
            let _ = ctx.registry.session_handle(&thread_id, &agent_kind).await;
            Ok("ok".into())
        }
        Err(crate::ports::agent_runtime::AgentStartError::NotFound(binary)) => {
            let install = ctx
                .registry
                .runtime_for(&agent_kind)
                .map(|r| r.install_command().to_string())
                .unwrap_or_default();
            Err(AppError::not_found(format!(
                "NOT_FOUND:{}:{}",
                binary, install
            )))
        }
        Err(e) => Err(AppError::agent(format!("agent_start failed: {}", e))),
    }
}

#[tauri::command]
pub async fn agent_install_and_start(
    ctx: State<'_, AppContext>,
    thread_id: String,
    agent_kind: String,
) -> Result<String, AppError> {
    crate::application::agents::start_with_install(&ctx, thread_id, agent_kind)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn agent_prompt(
    ctx: State<'_, AppContext>,
    app: tauri::AppHandle,
    thread_id: String,
    agent_kind: String,
    text: String,
) -> Result<(), AppError> {
    let app_clone = app.clone();
    crate::application::agents::prompt(&ctx, thread_id, agent_kind, text, move |event, payload| {
        let _ = app_clone.emit(event, payload);
    })
    .await
    .map_err(AppError::from)
}

#[tauri::command]
pub async fn agent_cancel(ctx: State<'_, AppContext>, thread_id: String) -> Result<(), AppError> {
    if let Some(session) = ctx.registry.session_handle_any(&thread_id).await {
        session.cancel()?;
    }
    Ok(())
}

#[tauri::command]
pub async fn agent_restart(ctx: State<'_, AppContext>, thread_id: String) -> Result<(), AppError> {
    let registry = ctx.registry.clone();
    let db = ctx.threads.clone();
    let tid = thread_id.clone();
    registry.kill(&tid).await;
    let _ = db.clear_working_memory(&ThreadId::from(tid));
    Ok(())
}

async fn resolve_session(
    ctx: &AppContext,
    thread_id: &str,
) -> Result<Arc<dyn crate::ports::agent_runtime::AgentSession>, AppError> {
    // Fast path: session already alive
    if let Some(session) = ctx.registry.session_handle_any(thread_id).await {
        return Ok(session);
    }

    // Slow path: look up thread, get agent_kind, build context, spawn
    let thread = ctx
        .threads
        .get_thread_sessions_for_thread(&ThreadId::from(thread_id.to_string()))?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::not_found(format!("Thread not found: {}", thread_id)))?;
    let agent_kind = thread.agent_kind.as_deref().ok_or_else(|| {
        AppError::validation(format!("Thread {} has no agent configured", thread_id))
    })?;
    let agent_ctx = build_agent_context(ctx, thread_id, agent_kind)?;
    let session = ctx
        .registry
        .get_or_spawn(thread_id, agent_kind, agent_ctx)
        .await
        .map_err(|e| AppError::agent(format!("Failed to start agent session: {}", e)))?;

    if let Some(ref model) = thread.model {
        match session.set_config_option("model", model) {
            Ok(_) => tracing::debug!(model, "set_config_option model succeeded"),
            Err(e) => tracing::warn!(model, error = %e, "set_config_option model failed"),
        }
    }

    Ok(session)
}

#[tauri::command]
pub async fn agent_get_session_info(
    ctx: State<'_, AppContext>,
    thread_id: String,
) -> Result<crate::domain::models::SessionInfo, AppError> {
    let session = resolve_session(&ctx, &thread_id).await?;
    Ok(session.session_info())
}

#[tauri::command]
pub async fn agent_set_mode(
    ctx: State<'_, AppContext>,
    thread_id: String,
    mode_id: String,
) -> Result<(), AppError> {
    let session = resolve_session(&ctx, &thread_id).await?;
    session.set_mode(&mode_id)?;
    Ok(())
}

#[tauri::command]
pub async fn agent_set_config_option(
    ctx: State<'_, AppContext>,
    thread_id: String,
    config_id: String,
    value: String,
) -> Result<(), AppError> {
    // Persist to DB FIRST
    if config_id == "model" {
        let _ = ctx.threads.update_thread(
            &ThreadId::from(thread_id.clone()),
            &ThreadPatch {
                model: Some(Some(value.clone())),
                ..Default::default()
            },
        );
    }

    let session = resolve_session(&ctx, &thread_id).await?;
    session.set_config_option(&config_id, &value)?;

    Ok(())
}
