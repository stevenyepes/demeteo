use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::domain::action::AgentAction;
use crate::domain::intercept::ExecutionResult;
use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
use crate::ports::execution::ExecutionPort;

/// Minimal port that delegates directly to `ExecutionPort` without
/// policy enforcement or intercept. Every action is executed immediately.
pub struct DirectExecutionPort {
    inner: Arc<dyn ExecutionPort>,
}

impl DirectExecutionPort {
    pub fn new(inner: Arc<dyn ExecutionPort>) -> Self {
        Self { inner }
    }

    async fn execute(
        inner: Arc<dyn ExecutionPort>,
        machine_id: String,
        action: AgentAction,
    ) -> Result<ExecutionResult, String> {
        match action {
            AgentAction::Read { path } => {
                let content = inner.read_file(&machine_id, &path).await?;
                let preview = content.lines().take(40).collect::<Vec<_>>().join("\n");
                Ok(ExecutionResult::FileRead {
                    path,
                    content_preview: preview,
                })
            }
            AgentAction::Edit { path, content } | AgentAction::Write { path, content } => {
                inner.write_file(&machine_id, &path, &content).await?;
                Ok(ExecutionResult::FileChanged {
                    path,
                    lines_added: 0,
                    lines_removed: 0,
                })
            }
            AgentAction::RunBash { cmd } => {
                let output = inner.run_command(&machine_id, &cmd).await?;
                Ok(ExecutionResult::Bash { output })
            }
        }
    }
}

#[async_trait]
impl AgentExecutionPort for DirectExecutionPort {
    async fn submit(
        &self,
        _thread_id: &str,
        machine_id: &str,
        action: AgentAction,
    ) -> Result<CommandOutcome, String> {
        let result = Self::execute(self.inner.clone(), machine_id.to_string(), action).await?;
        Ok(CommandOutcome::Executed { output: result })
    }

    async fn submit_agent(
        &self,
        _thread_id: &str,
        machine_id: &str,
        action: AgentAction,
        _tool_call_id: Option<String>,
    ) -> Result<CommandOutcome, ActionError> {
        let result = Self::execute(self.inner.clone(), machine_id.to_string(), action)
            .await
            .map_err(ActionError::internal)?;
        Ok(CommandOutcome::Executed { output: result })
    }

    async fn approve(&self, _intercept_id: &str) -> Result<(), String> {
        Err("no pending intercepts (policy engine removed)".into())
    }

    async fn reject(&self, _intercept_id: &str, _feedback: String) -> Result<(), String> {
        Err("no pending intercepts (policy engine removed)".into())
    }

    async fn register_result_responder(
        &self,
        _intercept_id: &str,
        _tx: oneshot::Sender<Result<ExecutionResult, String>>,
    ) -> Result<(), String> {
        Err("no pending intercepts (policy engine removed)".into())
    }
}
