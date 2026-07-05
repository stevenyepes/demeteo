use crate::error::AppError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A single chat message for an OpenAI-compatible `/chat/completions` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "system" | "user" | "assistant"
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

/// LLM operations used by the memory agent. Stateless: endpoint/model/key are
/// passed per call so the worker can read fresh config each tick and so the same
/// adapter can serve chat and embeddings against different endpoints.
///
/// Implemented today by a thin reqwest adapter against any OpenAI-compatible API
/// (Ollama, llama.cpp server, vLLM, OpenAI, …). A richer rig-core backed adapter
/// could implement the same port later without touching callers.
#[async_trait]
pub trait MemoryLlmPort: Send + Sync {
    /// One-shot chat completion; returns the assistant message content.
    async fn chat(
        &self,
        endpoint: &str,
        model: &str,
        api_key: Option<&str>,
        messages: Vec<ChatMessage>,
    ) -> Result<String, AppError>;

    /// Embed one or more texts; returns one vector per input, in order.
    async fn embed(
        &self,
        endpoint: &str,
        model: &str,
        api_key: Option<&str>,
        inputs: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, AppError>;

    /// List model ids available at the endpoint, for UI selection. Tries the
    /// OpenAI-compatible `/models` route, then falls back to Ollama's native
    /// `/api/tags`.
    async fn list_models(
        &self,
        endpoint: &str,
        api_key: Option<&str>,
    ) -> Result<Vec<String>, AppError>;
}
