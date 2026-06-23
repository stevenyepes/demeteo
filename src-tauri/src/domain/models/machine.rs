use crate::domain::ids::{AgentProfileId, MachineId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Machine {
    pub id: MachineId,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub auth_type: String, // 'key', 'password', 'agent', 'local'
    pub key_path: Option<String>,
    pub agents: Option<String>, // JSON-encoded array of {kind, enabled} records
    pub auto_approved_rules: Option<String>, // JSON-encoded array of auto-approved commands (regexes, legacy)
    #[serde(default)]
    pub use_login_shell: Option<bool>, // null/false = no login shell; true = bash -l -c
    #[serde(default)]
    pub setup_commands: Option<String>, // JSON array of shell commands run after clone
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentProfile {
    pub id: AgentProfileId,
    pub machine_id: MachineId,
    pub name: String,
    pub agent_type: String, // 'ollama', 'openai', 'cli', 'custom_http'
    pub command: Option<String>,
    pub work_dir: Option<String>,
    pub port: Option<i32>,
    pub ready_check: Option<String>,
}
