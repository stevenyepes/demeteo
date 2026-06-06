use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub auth_type: String, // 'key', 'password', 'agent'
    pub key_path: Option<String>,
    pub agents: Option<String>,               // JSON-encoded array of enabled agents
    pub auto_approved_rules: Option<String>,   // JSON-encoded array of auto-approved commands (regexes)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentProfile {
    pub id: String,
    pub machine_id: String,
    pub name: String,
    pub agent_type: String, // 'ollama', 'openai', 'cli', 'custom_http'
    pub command: Option<String>,
    pub work_dir: Option<String>,
    pub port: Option<i32>,
    pub ready_check: Option<String>,
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
    pub sender: String, // 'user', 'agent'
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionHistory {
    pub id: String,
    pub machine_id: String,
    pub session_type: String, // 'terminal', 'agent'
    pub title: String,
    pub content: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ThreadSession {
    pub id: String,
    pub machine_id: String,
    pub title: String,
    pub mode: String, // 'worktree', 'adhoc'
    pub branch: Option<String>,
    pub repo_path: Option<String>,
    pub sandbox_path: Option<String>,
    pub status: String, // 'idle', 'running', 'pending_approval'
}
