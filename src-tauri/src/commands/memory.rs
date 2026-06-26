//! Tauri commands for the memory agent: read/write its global config (endpoint,
//! models, enable flag) and test the configured LLM connection.

use serde::Serialize;
use tauri::State;

use crate::application::memory as mem_app;
use crate::domain::memory::MemoryAgentConfig;
use crate::error::AppError;
use crate::ports::memory_llm::ChatMessage;
use crate::state::AppContext;

/// Return the persisted memory agent config (disabled defaults if unset).
#[tauri::command]
pub fn memory_agent_config_get(ctx: State<'_, AppContext>) -> Result<MemoryAgentConfig, AppError> {
    Ok(mem_app::load_config(ctx.app_settings.as_ref()))
}

/// Persist the memory agent config. `api_key` semantics:
/// - `Some(non-empty)` → store in keyring, set `has_api_key = true`
/// - `Some("")`        → clear keyring, set `has_api_key = false`
/// - `None`            → leave the existing key untouched
#[tauri::command]
pub fn memory_agent_config_set(
    ctx: State<'_, AppContext>,
    mut config: MemoryAgentConfig,
    api_key: Option<String>,
) -> Result<(), AppError> {
    match api_key.as_deref() {
        Some("") => {
            mem_app::clear_api_key().map_err(AppError::from)?;
            config.has_api_key = false;
        }
        Some(key) => {
            mem_app::set_api_key(key).map_err(AppError::from)?;
            config.has_api_key = true;
        }
        None => {
            config.has_api_key = mem_app::load_api_key().is_some();
        }
    }
    mem_app::save_config(ctx.app_settings.as_ref(), &config).map_err(AppError::from)
}

/// List models available at the given endpoint (OpenAI `/models`, falling back
/// to Ollama `/api/tags`). `api_key` falls back to the stored keyring value.
#[tauri::command]
pub async fn memory_agent_list_models(
    ctx: State<'_, AppContext>,
    endpoint: String,
    api_key: Option<String>,
) -> Result<Vec<String>, AppError> {
    let key = api_key
        .filter(|k| !k.is_empty())
        .or_else(mem_app::load_api_key);
    ctx.memory_llm.list_models(&endpoint, key.as_deref()).await
}

#[derive(Serialize)]
pub struct MemoryAgentTestResult {
    pub chat_ok: bool,
    pub embed_ok: bool,
    pub embed_dims: Option<usize>,
    pub error: Option<String>,
}

/// Test connectivity to the configured chat + embeddings endpoints. Accepts the
/// config from the request (so the UI can test before saving); `api_key` falls
/// back to the stored keyring value when omitted.
#[tauri::command]
pub async fn memory_agent_test_connection(
    ctx: State<'_, AppContext>,
    config: MemoryAgentConfig,
    api_key: Option<String>,
) -> Result<MemoryAgentTestResult, AppError> {
    let key = api_key
        .filter(|k| !k.is_empty())
        .or_else(mem_app::load_api_key);

    let chat_res = ctx
        .memory_llm
        .chat(
            &config.chat_endpoint,
            &config.chat_model,
            key.as_deref(),
            vec![ChatMessage::user("Reply with the single word: ok")],
        )
        .await;

    if let Err(e) = chat_res {
        return Ok(MemoryAgentTestResult {
            chat_ok: false,
            embed_ok: false,
            embed_dims: None,
            error: Some(format!("chat: {}", e)),
        });
    }

    let embed_res = ctx
        .memory_llm
        .embed(
            config.embed_endpoint_or_chat(),
            &config.embed_model,
            key.as_deref(),
            vec!["demeteo connectivity check".to_string()],
        )
        .await;

    match embed_res {
        Ok(vecs) => Ok(MemoryAgentTestResult {
            chat_ok: true,
            embed_ok: true,
            embed_dims: vecs.first().map(|v| v.len()),
            error: None,
        }),
        Err(e) => Ok(MemoryAgentTestResult {
            chat_ok: true,
            embed_ok: false,
            embed_dims: None,
            error: Some(format!("embeddings: {}", e)),
        }),
    }
}
