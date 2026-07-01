use crate::domain::ids::{MachineId, MessageId, ThreadId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn deserialize_lax_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(deserializer)?;
    match val {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        _ => Ok(val.to_string()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<SessionModeState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<ConfigOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionModeState {
    #[serde(rename = "currentModeId")]
    pub current_mode_id: String,
    #[serde(rename = "availableModes")]
    pub available_modes: Vec<SessionModeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionModeInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(rename = "type", default)]
    pub option_type: String,
    #[serde(rename = "currentValue", deserialize_with = "deserialize_lax_string")]
    pub current_value: String,
    #[serde(default)]
    pub options: Vec<ConfigOptionValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOptionValue {
    #[serde(deserialize_with = "deserialize_lax_string")]
    pub value: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Best-effort signal that the chosen model can accept attached images.
    ///
    /// Driven by `application::agent_probe::fallback_models` for bundled
    /// model lists and by a substring heuristic for dynamically probed
    /// models. Defaults to `false` on deserialize (unknown models never
    /// silently claim vision support) and is *always* serialized so the
    /// UI can render a soft warning without an extra lookup.
    #[serde(default)]
    pub supports_images: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ThreadSession {
    pub id: ThreadId,
    pub machine_id: MachineId,
    pub title: String,
    pub mode: String,
    pub branch: Option<String>,
    pub repo_path: Option<String>,
    pub sandbox_path: Option<String>,
    pub status: String,
    pub agent_kind: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentConfig {
    pub kind: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkingMemoryEntry {
    pub file_path: String,
    pub line_count: Option<u32>,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<i64>,
    pub first_read_at: i64,
    pub last_read_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub role: String,
    pub content: String,
    pub metadata: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatSession {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub sender: String,
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionHistory {
    pub id: String,
    pub machine_id: MachineId,
    pub session_type: String,
    pub title: String,
    pub content: Option<String>,
    pub created_at: String,
}
