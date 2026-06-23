use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::domain::action::AgentAction;
use crate::domain::intercept::{ExecutionResult, InterceptPayload};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandOutcome {
    Executed {
        output: ExecutionResult,
    },
    Intercepted {
        intercept_id: String,
        payload: InterceptPayload,
    },
}

/// Typed error envelope. Per AGENT_INTEGRATION §9.1, every new error path
/// returns one of these; existing free-form `Err(String)` returns are
/// migrated incrementally.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionError {
    Network { message: String },
    NotFound { message: String },
    Internal { message: String },
}

impl ActionError {
    pub fn network(msg: impl Into<String>) -> Self {
        ActionError::Network {
            message: msg.into(),
        }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        ActionError::NotFound {
            message: msg.into(),
        }
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        ActionError::Internal {
            message: msg.into(),
        }
    }
}

/// Async execution port for agent-originated actions. Tauri v2 supports
/// async commands natively, and making the port async removed the
/// previous `tokio::task::spawn_blocking` wrappers that sat in every
/// `#[tauri::command]` calling into this trait.
#[async_trait]
pub trait AgentExecutionPort: Send + Sync {
    /// Submit a hand-rolled action (originating from the UI). On rejection,
    /// the result is returned as a free-form error string for backward
    /// compatibility; the new typed `ActionError` lives on the
    /// `submit_agent` path.
    async fn submit(
        &self,
        thread_id: &str,
        machine_id: &str,
        action: AgentAction,
    ) -> Result<CommandOutcome, String>;

    /// Submit an action originating from an agent tool call. The optional
    /// `tool_call_id` is recorded on the intercept payload and surfaces as a
    /// `Resolution::RejectAsToolFailure` on reject so the agent runtime can
    /// return the failure as a structured `tool_call/update` rather than a
    /// synthetic bash output.
    async fn submit_agent(
        &self,
        thread_id: &str,
        machine_id: &str,
        action: AgentAction,
        tool_call_id: Option<String>,
    ) -> Result<CommandOutcome, ActionError>;

    async fn approve(&self, intercept_id: &str) -> Result<(), String>;

    async fn reject(&self, intercept_id: &str, feedback: String) -> Result<(), String>;

    async fn register_result_responder(
        &self,
        intercept_id: &str,
        tx: oneshot::Sender<Result<ExecutionResult, String>>,
    ) -> Result<(), String>;
}
