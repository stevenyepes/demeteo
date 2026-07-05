//! Thin reqwest adapter for the memory agent's LLM calls against any
//! OpenAI-compatible endpoint (Ollama's `/v1`, llama.cpp server, vLLM, OpenAI).
//!
//! This is the one place Demeteo talks to a model provider directly; it is
//! scoped to the memory feature, opt-in, and user-configured. The coding-agent
//! orchestration path still goes exclusively through agent CLIs.

use crate::error::AppError;
use crate::ports::memory_llm::{ChatMessage, MemoryLlmPort};
use async_trait::async_trait;

pub struct ReqwestMemoryLlmAdapter {
    client: reqwest::Client,
}

impl Default for ReqwestMemoryLlmAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestMemoryLlmAdapter {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("demeteo-memory-agent")
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

/// Normalize a user-entered endpoint into a usable absolute base URL: trims
/// whitespace, errors clearly when empty, prepends `http://` when the scheme is
/// missing, and drops a trailing slash. Without this, an empty field yields a
/// relative URL and reqwest fails with a cryptic "builder error".
fn normalized_base(endpoint: &str) -> Result<String, AppError> {
    let e = endpoint.trim();
    if e.is_empty() {
        return Err(AppError::validation("endpoint is empty"));
    }
    let e = if e.contains("://") {
        e.to_string()
    } else {
        format!("http://{}", e)
    };
    Ok(e.trim_end_matches('/').to_string())
}

/// Join a normalized base endpoint with a path.
fn join_url(endpoint: &str, path: &str) -> Result<String, AppError> {
    Ok(format!("{}/{}", normalized_base(endpoint)?, path))
}

/// Strip a trailing `/v1` (and slashes) to reach a provider's native root, e.g.
/// `http://localhost:11434/v1` → `http://localhost:11434`.
fn provider_root(endpoint: &str) -> Result<String, AppError> {
    Ok(normalized_base(endpoint)?
        .trim_end_matches("/v1")
        .trim_end_matches('/')
        .to_string())
}

#[async_trait]
impl MemoryLlmPort for ReqwestMemoryLlmAdapter {
    async fn chat(
        &self,
        endpoint: &str,
        model: &str,
        api_key: Option<&str>,
        messages: Vec<ChatMessage>,
    ) -> Result<String, AppError> {
        let url = join_url(endpoint, "chat/completions")?;
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "temperature": 0.2,
        });

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let res = req.send().await.map_err(|e| AppError::Transport {
            message: e.to_string(),
        })?;
        if !res.status().is_success() {
            let status = res.status();
            return Err(AppError::Provider {
                message: format!("chat HTTP {}", status),
            });
        }
        let data: serde_json::Value = res.json().await.map_err(|e| AppError::Transport {
            message: e.to_string(),
        })?;
        let content = data["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| AppError::provider("chat response missing choices[0].message.content"))?
            .to_string();
        Ok(content)
    }

    async fn embed(
        &self,
        endpoint: &str,
        model: &str,
        api_key: Option<&str>,
        inputs: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, AppError> {
        let url = join_url(endpoint, "embeddings")?;
        let body = serde_json::json!({
            "model": model,
            "input": inputs,
        });

        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let res = req.send().await.map_err(|e| AppError::Transport {
            message: e.to_string(),
        })?;
        if !res.status().is_success() {
            let status = res.status();
            return Err(AppError::Provider {
                message: format!("embeddings HTTP {}", status),
            });
        }
        let data: serde_json::Value = res.json().await.map_err(|e| AppError::Transport {
            message: e.to_string(),
        })?;
        let rows = data["data"]
            .as_array()
            .ok_or_else(|| AppError::provider("embeddings response missing data[]"))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let vec = row["embedding"]
                .as_array()
                .ok_or_else(|| AppError::provider("embeddings row missing embedding[]"))?
                .iter()
                .map(|v| v.as_f64().unwrap_or(0.0) as f32)
                .collect::<Vec<f32>>();
            out.push(vec);
        }
        Ok(out)
    }

    async fn list_models(
        &self,
        endpoint: &str,
        api_key: Option<&str>,
    ) -> Result<Vec<String>, AppError> {
        // 1. OpenAI-compatible /models → { "data": [ { "id": ... } ] }
        let url = join_url(endpoint, "models")?;
        let mut req = self.client.get(&url);
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
        if let Ok(res) = req.send().await {
            if res.status().is_success() {
                if let Ok(data) = res.json::<serde_json::Value>().await {
                    let ids = collect_ids(&data["data"], "id");
                    if !ids.is_empty() {
                        return Ok(ids);
                    }
                }
            }
        }

        // 2. Ollama native /api/tags → { "models": [ { "name": ... } ] }
        let url = format!("{}/api/tags", provider_root(endpoint)?);
        let res = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Transport {
                message: e.to_string(),
            })?;
        if !res.status().is_success() {
            return Err(AppError::Provider {
                message: format!("models HTTP {}", res.status()),
            });
        }
        let data: serde_json::Value = res.json().await.map_err(|e| AppError::Transport {
            message: e.to_string(),
        })?;
        Ok(collect_ids(&data["models"], "name"))
    }
}

/// Pull a sorted, de-duplicated list of string fields from a JSON array.
fn collect_ids(arr: &serde_json::Value, field: &str) -> Vec<String> {
    let mut ids: Vec<String> = arr
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|m| m[field].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    ids
}
