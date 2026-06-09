use crate::adapters;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::db::DatabasePort;
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::NotificationPort;
use serde::Serialize;
use std::sync::Arc;

pub struct DatabaseState {
    pub db: Arc<dyn DatabasePort>,
}

pub struct ExecutionState {
    pub exec: Arc<dyn ExecutionPort>,
}

pub struct AgentExecutionState {
    pub agent_exec: Arc<dyn AgentExecutionPort>,
}

pub struct NotificationState {
    pub notif: Arc<dyn NotificationPort>,
}

pub struct AgentRegistryState {
    pub registry: Arc<adapters::agent::registry::AgentRegistry>,
    pub agent_exec: Arc<dyn AgentExecutionPort>,
}

pub const EVENT_THREAD_STATUS_CHANGED: &str = "thread_status_changed";
pub const EVENT_AGENT_EVENT: &str = "agent_event";

#[derive(Serialize, Clone)]
pub struct ThreadStatusChanged {
    pub thread_id: String,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct AgentConfigView {
    pub kind: String,
    pub enabled: bool,
    pub available: bool,
    pub install_command: String,
}
