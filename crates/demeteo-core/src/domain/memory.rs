use crate::domain::ids::ProjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectMemoryEntry {
    pub id: String,
    pub project_id: ProjectId,
    pub key: String,
    pub value: String,
    pub source: MemorySource,
    pub confidence: f64,
    /// Typed category for agent-formed memories. `None` for legacy/manual rows.
    pub memory_type: Option<MemoryType>,
    /// Canonical prose statement (mirrors `value` for new rows).
    pub statement: Option<String>,
    /// Embedding of `statement`/`value` for semantic retrieval. `None` until embedded.
    pub embedding: Option<Vec<f32>>,
    pub embedding_model: Option<String>,
    pub last_used_at: Option<i64>,
    pub use_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    Agent,
    Human,
}

impl MemorySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemorySource::Agent => "agent",
            MemorySource::Human => "human",
        }
    }

    #[allow(clippy::should_implement_trait)] // lenient fallback to Human, not FromStr
    pub fn from_str(s: &str) -> Self {
        match s {
            "agent" => MemorySource::Agent,
            _ => MemorySource::Human,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Convention,
    Lesson,
    Decision,
    Preference,
    Fact,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryType::Convention => "convention",
            MemoryType::Lesson => "lesson",
            MemoryType::Decision => "decision",
            MemoryType::Preference => "preference",
            MemoryType::Fact => "fact",
        }
    }

    #[allow(clippy::should_implement_trait)] // returns Option, not a FromStr impl
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "convention" => Some(MemoryType::Convention),
            "lesson" => Some(MemoryType::Lesson),
            "decision" => Some(MemoryType::Decision),
            "preference" => Some(MemoryType::Preference),
            "fact" => Some(MemoryType::Fact),
            _ => None,
        }
    }
}

/// A raw observation captured synchronously during a feature run. The background
/// memory worker consumes unprocessed signals and distills them into memories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemorySignal {
    pub id: String,
    pub project_id: ProjectId,
    pub feature_id: String,
    pub step_execution_id: Option<String>,
    pub kind: SignalKind,
    pub content: String,
    pub created_at: i64,
    pub processed_at: Option<i64>,
    pub attempts: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    AgentSummary,
    Failure,
    Retry,
    GateFeedback,
}

impl SignalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalKind::AgentSummary => "agent_summary",
            SignalKind::Failure => "failure",
            SignalKind::Retry => "retry",
            SignalKind::GateFeedback => "gate_feedback",
        }
    }

    #[allow(clippy::should_implement_trait)] // lenient fallback to GateFeedback
    pub fn from_str(s: &str) -> Self {
        match s {
            "agent_summary" => SignalKind::AgentSummary,
            "failure" => SignalKind::Failure,
            "retry" => SignalKind::Retry,
            _ => SignalKind::GateFeedback,
        }
    }
}

/// User-configured settings for the background memory agent. Persisted as JSON
/// in `app_settings` under the `memory_agent_config` key. The API key itself is
/// stored in the OS keyring, not here; `has_api_key` only records whether one
/// is set so the UI can render the field state without exposing the secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAgentConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub chat_endpoint: String,
    #[serde(default)]
    pub chat_model: String,
    #[serde(default)]
    pub embed_endpoint: String,
    #[serde(default)]
    pub embed_model: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub min_confidence: f64,
}

fn default_top_k() -> usize {
    12
}

impl Default for MemoryAgentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            chat_endpoint: String::new(),
            chat_model: String::new(),
            embed_endpoint: String::new(),
            embed_model: String::new(),
            has_api_key: false,
            top_k: default_top_k(),
            min_confidence: 0.0,
        }
    }
}

impl MemoryAgentConfig {
    /// Embeddings endpoint, falling back to the chat endpoint when left blank
    /// (a common single-server setup, e.g. one Ollama instance).
    pub fn embed_endpoint_or_chat(&self) -> &str {
        if self.embed_endpoint.trim().is_empty() {
            self.chat_endpoint.trim()
        } else {
            self.embed_endpoint.trim()
        }
    }

    /// True when chat + embeddings are sufficiently configured to run.
    pub fn is_usable(&self) -> bool {
        self.enabled
            && !self.chat_endpoint.trim().is_empty()
            && !self.chat_model.trim().is_empty()
            && !self.embed_endpoint_or_chat().is_empty()
            && !self.embed_model.trim().is_empty()
    }
}

/// Cosine similarity between two equal-length vectors. Returns 0.0 if either is
/// empty, lengths differ, or a magnitude is zero.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Serialize an f32 embedding to a little-endian byte blob for SQLite storage.
pub fn embedding_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Decode a little-endian byte blob back into an f32 embedding.
pub fn blob_to_embedding(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_blob_roundtrips() {
        let v = vec![0.0f32, 1.0, -2.5, 123.456, 1e-9];
        assert_eq!(blob_to_embedding(&embedding_to_blob(&v)), v);
    }

    #[test]
    fn cosine_identical_is_one_orthogonal_is_zero() {
        let a = vec![1.0f32, 2.0, 3.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        // length mismatch / empty are guarded to 0.0
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn memory_type_str_roundtrips() {
        for t in [
            MemoryType::Convention,
            MemoryType::Lesson,
            MemoryType::Decision,
            MemoryType::Preference,
            MemoryType::Fact,
        ] {
            assert_eq!(MemoryType::from_str(t.as_str()), Some(t));
        }
        assert_eq!(MemoryType::from_str("bogus"), None);
    }
}
