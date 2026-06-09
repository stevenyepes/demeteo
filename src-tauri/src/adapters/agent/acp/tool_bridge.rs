use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::domain::intercept::ExecutionResult;
use crate::domain::action::AgentAction;
use crate::ports::agent_execution::AgentExecutionPort;

/// Result of dispatching a tool call through the bridge.
pub struct DispatchResult {
    /// JSON-RPC payload to send as `tool_call/update` immediately.
    /// `Some(payload)` for executed/rejected, `None` for intercepted.
    pub payload: Option<Value>,
    /// The intercept_id, populated when the action was intercepted.
    pub intercept_id: Option<String>,
    /// The tool_call_id from the agent, needed to send the response later.
    pub tool_call_id: String,
    /// Receiver that will get the execution result when the intercept resolves.
    pub intercept_rx: Option<oneshot::Receiver<Result<ExecutionResult, String>>>,
}

/// Bridge between ACP's agent-initiated `fs/*` / `terminal/*` requests and
/// the existing `PolicyEnforcedExecutionPort`. Every file operation the
/// agent attempts is funnelled through the policy engine (which applies
/// the scope fence and the user rules). On a Reject, the bridge returns
/// a `tool_call/update` payload that the agent sees as a structured
/// failure with the supervisor's feedback as the error text.
pub struct ToolBridge {
    pub agent_exec: Arc<dyn AgentExecutionPort>,
}

impl ToolBridge {
    pub fn new(agent_exec: Arc<dyn AgentExecutionPort>) -> Self {
        Self { agent_exec }
    }

    /// Handle `fs/read_text_file`.
    pub fn handle_read_text_file(
        &self,
        thread_id: &str,
        machine_id: &str,
        path: &str,
        tool_call_id: &str,
    ) -> DispatchResult {
        let action = AgentAction::Read { path: path.to_string() };
        self.dispatch(thread_id, machine_id, action, tool_call_id, |outcome| {
            if let crate::ports::agent_execution::CommandOutcome::Executed { output } = outcome {
                if let ExecutionResult::FileRead {
                    content_preview,
                    ..
                } = output
                {
                    return json!({
                        "content": content_preview,
                        "truncated": true
                    });
                }
            }
            json!({})
        })
    }

    /// Handle `fs/write_text_file`.
    pub fn handle_write_text_file(
        &self,
        thread_id: &str,
        machine_id: &str,
        path: &str,
        content: &str,
        tool_call_id: &str,
    ) -> DispatchResult {
        let action = AgentAction::Write {
            path: path.to_string(),
            content: content.to_string(),
        };
        self.dispatch(thread_id, machine_id, action, tool_call_id, |_| {
            json!({})
        })
    }

    /// Handle `fs/edit_text_file`.
    pub fn handle_edit_text_file(
        &self,
        thread_id: &str,
        machine_id: &str,
        path: &str,
        content: &str,
        tool_call_id: &str,
    ) -> DispatchResult {
        let action = AgentAction::Edit {
            path: path.to_string(),
            content: content.to_string(),
        };
        self.dispatch(thread_id, machine_id, action, tool_call_id, |_| {
            json!({})
        })
    }

    /// Handle `terminal/create`.
    pub fn handle_terminal_create(
        &self,
        thread_id: &str,
        machine_id: &str,
        cmd: &str,
        tool_call_id: &str,
    ) -> DispatchResult {
        let action = AgentAction::RunBash { cmd: cmd.to_string() };
        self.dispatch(thread_id, machine_id, action, tool_call_id, |outcome| {
            if let crate::ports::agent_execution::CommandOutcome::Executed { output } = outcome {
                if let ExecutionResult::Bash { output } = output {
                    return json!({ "output": output });
                }
            }
            json!({})
        })
    }

    /// Core dispatch. Calls `submit_agent` with the tool_call_id and
    /// packages the outcome as a JSON-RPC response payload.
    ///
    /// When the action is intercepted, registers a result responder so the
    /// runtime can await the execution result and send `tool_call/update`.
    fn dispatch(
        &self,
        thread_id: &str,
        machine_id: &str,
        action: AgentAction,
        tool_call_id: &str,
        on_executed: impl FnOnce(&crate::ports::agent_execution::CommandOutcome) -> Value,
    ) -> DispatchResult {
        match self.agent_exec.submit_agent(
            thread_id,
            machine_id,
            action,
            Some(tool_call_id.to_string()),
        ) {
            Ok(crate::ports::agent_execution::CommandOutcome::Intercepted { intercept_id, .. }) => {
                let (result_tx, result_rx) = oneshot::channel();
                let _ = self.agent_exec.register_result_responder(
                    &intercept_id,
                    result_tx,
                );
                DispatchResult {
                    payload: None,
                    intercept_id: Some(intercept_id),
                    tool_call_id: tool_call_id.to_string(),
                    intercept_rx: Some(result_rx),
                }
            }
            Ok(outcome) => DispatchResult {
                payload: Some(on_executed(&outcome)),
                intercept_id: None,
                tool_call_id: tool_call_id.to_string(),
                intercept_rx: None,
            },
            Err(e) => {
                let kind = match &e {
                    crate::ports::agent_execution::ActionError::NotFound { .. } => "not_found",
                    crate::ports::agent_execution::ActionError::Network { .. } => "network",
                    crate::ports::agent_execution::ActionError::Internal { .. } => "internal",
                };
                let message = match e {
                    crate::ports::agent_execution::ActionError::NotFound { message }
                    | crate::ports::agent_execution::ActionError::Network { message }
                    | crate::ports::agent_execution::ActionError::Internal { message } => message,
                };
                DispatchResult {
                    payload: Some(json!({
                        "status": "failed",
                        "kind": kind,
                        "content": [
                            { "type": "text", "text": message }
                        ]
                    })),
                    intercept_id: None,
                    tool_call_id: tool_call_id.to_string(),
                    intercept_rx: None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::intercept::ExecutionResult;
    use crate::domain::action::AgentAction;
    use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};

    /// Always-approve port (no real policy — just returns Executed).
    struct ApprovePort;
    impl AgentExecutionPort for ApprovePort {
        fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> {
            Ok(CommandOutcome::Executed {
                output: ExecutionResult::FileRead {
                    path: "/x".into(),
                    content_preview: "hello".into(),
                },
            })
        }
        fn submit_agent(
            &self,
            _: &str,
            _: &str,
            action: AgentAction,
            _: Option<String>,
        ) -> Result<CommandOutcome, ActionError> {
            // Echo a minimal success for whatever action.
            let out = match action {
                AgentAction::Read { path } => ExecutionResult::FileRead {
                    path,
                    content_preview: "line1\nline2".into(),
                },
                AgentAction::Write { path, .. } | AgentAction::Edit { path, .. } => {
                    ExecutionResult::FileChanged {
                        path,
                        lines_added: 1,
                        lines_removed: 0,
                    }
                }
                AgentAction::RunBash { cmd } => ExecutionResult::Bash {
                    output: format!("ran {}", cmd),
                },
            };
            Ok(CommandOutcome::Executed { output: out })
        }
        fn approve(&self, _: &str) -> Result<(), String> { Ok(()) }
        fn reject(&self, _: &str, _: String) -> Result<(), String> { Ok(()) }
        fn register_result_responder(
            &self,
            _: &str,
            _: tokio::sync::oneshot::Sender<Result<crate::domain::intercept::ExecutionResult, String>>,
        ) -> Result<(), String> { Ok(()) }
    }

    /// Always-reject port.
    struct RejectPort;
    impl AgentExecutionPort for RejectPort {
        fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> {
            Err("rejected".into())
        }
        fn submit_agent(
            &self,
            _: &str,
            _: &str,
            _: AgentAction,
            _: Option<String>,
        ) -> Result<CommandOutcome, ActionError> {
            Err(ActionError::NotFound {
                message: "out of scope".into(),
            })
        }
        fn approve(&self, _: &str) -> Result<(), String> { Ok(()) }
        fn reject(&self, _: &str, _: String) -> Result<(), String> { Ok(()) }
        fn register_result_responder(
            &self,
            _: &str,
            _: tokio::sync::oneshot::Sender<Result<crate::domain::intercept::ExecutionResult, String>>,
        ) -> Result<(), String> { Ok(()) }
    }

    fn bridge_with(port: Arc<dyn AgentExecutionPort>) -> ToolBridge {
        ToolBridge::new(port)
    }

    #[test]
    fn read_text_file_happy_path_returns_content() {
        let b = bridge_with(Arc::new(ApprovePort));
        let dr = b.handle_read_text_file("t1", "m1", "/a", "tc-1");
        let v = dr.payload.expect("should return Some");
        assert_eq!(v["content"], "line1\nline2");
        assert_eq!(v["truncated"], true);
    }

    #[test]
    fn write_text_file_reject_returns_tool_failure() {
        let b = bridge_with(Arc::new(RejectPort));
        let dr = b.handle_write_text_file("t1", "m1", "/a", "x", "tc-2");
        let v = dr.payload.expect("should return Some on reject");
        assert_eq!(v["status"], "failed");
        assert_eq!(v["kind"], "not_found");
        let content = v["content"].as_array().unwrap();
        assert_eq!(content[0]["text"], "out of scope");
    }

    #[test]
    fn terminal_create_happy_path_returns_output() {
        let b = bridge_with(Arc::new(ApprovePort));
        let dr = b.handle_terminal_create("t1", "m1", "ls -la", "tc-3");
        let v = dr.payload.expect("should return Some");
        assert_eq!(v["output"], "ran ls -la");
    }

    #[test]
    fn terminal_create_reject_returns_tool_failure() {
        let b = bridge_with(Arc::new(RejectPort));
        let dr = b.handle_terminal_create("t1", "m1", "rm -rf /", "tc-4");
        let v = dr.payload.expect("should return Some on reject");
        assert_eq!(v["status"], "failed");
        assert_eq!(v["kind"], "not_found");
    }
}
