//! `anomalyco/opencode` agent — the open-source coding agent. Demeteo's
//! integration targets this project; we are not affiliated with it.
//!
//! Wire format: `opencode run --format json` emits nd-JSON on stdout.
//! The prompt is passed via stdin to avoid OS ARG_MAX limits.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession, AgentStartError};

pub const OPENCODE_INSTALL: &str = "curl -fsSL https://opencode.ai/install | bash";

/// Parse an opencode CLI JSON-lines event into an `AgentEvent`.
pub fn parse_opencode_event(line: &str) -> Option<AgentEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let v: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    if let Some(kind) = v
        .get("type")
        .or_else(|| v.get("kind"))
        .and_then(|t| t.as_str())
    {
        if let Some(evt) = parse_part_shape(kind, &v) {
            return Some(evt);
        }
        if v.get("part").is_none() {
            if let Some(evt) = parse_top_level_kind(kind, &v) {
                return Some(evt);
            }
        }
    }

    if let Some(update) = v.get("update") {
        if let Some(discriminator) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
            return parse_nested_session_update(discriminator, update);
        }
    }

    None
}

fn parse_part_shape(kind: &str, v: &serde_json::Value) -> Option<AgentEvent> {
    let part = v.get("part")?;
    match kind {
        "text" => {
            let text = part
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                None
            } else {
                Some(AgentEvent::Text { delta: text })
            }
        }
        "step_finish" => {
            let reason = part.get("reason").and_then(|r| r.as_str()).unwrap_or("");
            if reason == "stop" {
                Some(AgentEvent::TurnComplete {
                    stop_reason: StopReason::EndOfTurn,
                })
            } else if let Some(tokens) = part.get("tokens") {
                let input_tokens = tokens.get("input").and_then(|t| t.as_u64()).unwrap_or(0);
                let output_tokens = tokens.get("output").and_then(|t| t.as_u64()).unwrap_or(0);
                let cost_usd = part.get("cost").and_then(|t| t.as_f64());
                Some(AgentEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cost_usd,
                })
            } else {
                None
            }
        }
        "tool_use" => {
            let tool = part
                .get("tool")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            let call_id = part.get("callID").and_then(|t| t.as_str()).unwrap_or("");
            let state = part.get("state");
            let status = state
                .and_then(|s| s.get("status"))
                .and_then(|t| t.as_str())
                .unwrap_or("running");
            let input = state
                .and_then(|s| s.get("input"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let output = state
                .and_then(|s| s.get("output"))
                .and_then(|t| t.as_str())
                .unwrap_or("");

            let line = if status == "completed" {
                let mut line = format!("[tool {tool} id={call_id}]");
                if !output.is_empty() {
                    line.push_str(&format!("\n{output}"));
                } else {
                    let input_str = serde_json::to_string(&input).unwrap_or_default();
                    line.push_str(&format!("\ninput: {input_str}"));
                }
                line
            } else {
                let err_output = state
                    .and_then(|s| s.get("output"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let detail = if !err_output.is_empty() {
                    format!(": {err_output}")
                } else if let (Some(input_val), _) = (input.as_object(), status) {
                    let input_str = serde_json::to_string(input_val).unwrap_or_default();
                    let max = 120;
                    if input_str.len() > max {
                        format!(" — input={}…", &input_str[..max])
                    } else {
                        format!(" — input={input_str}")
                    }
                } else {
                    String::new()
                };
                tracing::debug!(tool = %tool, status = %status, call_id = %call_id, detail = %detail, "opencode tool call");
                format!("[tool {tool} ({status}) id={call_id}]{detail}")
            };
            Some(AgentEvent::Text { delta: line })
        }
        "step_start" | "snapshot" | "patch" => None,
        _ => None,
    }
}

fn parse_top_level_kind(kind: &str, v: &serde_json::Value) -> Option<AgentEvent> {
    match kind {
        "text" | "message" | "assistant" | "text_delta" => {
            let delta = v
                .get("delta")
                .or_else(|| v.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if delta.is_empty() {
                None
            } else {
                Some(AgentEvent::Text { delta })
            }
        }
        "tool_call" | "tool_use" => {
            let tool = v
                .get("name")
                .or_else(|| v.get("tool"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let input = serde_json::to_string(
                &v.get("input")
                    .or_else(|| v.get("data"))
                    .unwrap_or(&serde_json::Value::Null),
            )
            .ok()?;
            Some(AgentEvent::Text {
                delta: format!("[tool: {}] {}", tool, input),
            })
        }
        "usage" | "usage_update" => {
            let input_tokens = v
                .get("inputTokens")
                .or_else(|| v.get("input_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output_tokens = v
                .get("outputTokens")
                .or_else(|| v.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cost_usd = v
                .get("costUsd")
                .or_else(|| v.get("cost_usd"))
                .and_then(|v| v.as_f64());
            Some(AgentEvent::Usage {
                input_tokens,
                output_tokens,
                cost_usd,
            })
        }
        "plan" => {
            let entries = serde_json::from_value(
                v.get("entries")
                    .or_else(|| v.get("steps"))
                    .cloned()
                    .unwrap_or_default(),
            )
            .ok()
            .unwrap_or_default();
            Some(AgentEvent::Plan { entries })
        }
        "end_turn" | "message_stop" | "done" => Some(AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
        }),
        "error" => {
            let message = v
                .get("message")
                .or_else(|| v.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            Some(AgentEvent::Error {
                code: "cli_error".to_string(),
                message,
                recoverable: false,
            })
        }
        _ => None,
    }
}

fn parse_nested_session_update(
    discriminator: &str,
    update: &serde_json::Value,
) -> Option<AgentEvent> {
    match discriminator {
        "agent_message_chunk" => {
            let delta = update
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if delta.is_empty() {
                None
            } else {
                Some(AgentEvent::Text { delta })
            }
        }
        "tool_call" => {
            let tool_call_id = update
                .get("toolCallId")
                .or_else(|| update.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let action_str = update
                .get("action")
                .or_else(|| update.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let target = update
                .get("path")
                .or_else(|| update.get("target"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let preview = update
                .get("input")
                .or_else(|| update.get("rawInput"))
                .map(|v| v.to_string());
            let action =
                serde_json::from_str::<crate::domain::action::ActionKind>(action_str).ok()?;
            let intercept_id = format!("oc-{}", tool_call_id);
            Some(AgentEvent::ToolCall {
                tool_call_id,
                intercept_id,
                action,
                target,
                preview,
            })
        }
        "tool_call_update" => {
            let tool_call_id = update
                .get("toolCallId")
                .or_else(|| update.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = if let Some(status_val) = update
                .get("status")
                .or_else(|| update.get("state"))
                .and_then(|v| v.as_str())
            {
                match status_val {
                    "completed" => crate::domain::agent_event::ToolCallStatus::Completed,
                    "failed" => crate::domain::agent_event::ToolCallStatus::Failed {
                        reason: update
                            .get("reason")
                            .or_else(|| update.get("error"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    },
                    "in_progress" => {
                        crate::domain::agent_event::ToolCallStatus::InProgress { message: None }
                    }
                    _ => crate::domain::agent_event::ToolCallStatus::Pending,
                }
            } else {
                crate::domain::agent_event::ToolCallStatus::Pending
            };
            let preview = update.get("preview").map(|v| v.to_string());
            Some(AgentEvent::ToolCallUpdate {
                tool_call_id,
                status,
                preview,
            })
        }
        "usage_update" => {
            let input_tokens = update
                .get("inputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output_tokens = update
                .get("outputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cost_usd = update.get("costUsd").and_then(|v| v.as_f64());
            Some(AgentEvent::Usage {
                input_tokens,
                output_tokens,
                cost_usd,
            })
        }
        "plan" => {
            let entries =
                serde_json::from_value(update.get("entries").cloned().unwrap_or_default())
                    .ok()
                    .unwrap_or_default();
            Some(AgentEvent::Plan { entries })
        }
        "current_mode_update" => {
            let mode_id = update
                .get("mode")
                .or_else(|| update.get("modeId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if mode_id.is_empty() {
                None
            } else {
                Some(AgentEvent::ModeChanged { mode_id })
            }
        }
        "agent_thought_chunk" | "available_commands_update" | "session_info_update" => None,
        _ => None,
    }
}

/// Construct command-line arguments for OpenCode run.
pub fn build_opencode_args(ctx: &AgentContext, captured_session_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];

    // Non-interactive auto-approval for allowed tools, avoiding execution hangs
    args.push("--dangerously-skip-permissions".to_string());

    if let Some(sid) = captured_session_id {
        args.push("--session".to_string());
        args.push(sid.to_string());
        args.push("--continue".to_string());
    }
    if let Some(ref m) = ctx.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }
    if let Some(ref title) = ctx.title {
        args.push("--title".to_string());
        args.push(title.clone());
    }
    args.push("--dir".to_string());
    args.push(ctx.cwd.clone());
    args
}

/// Runtime wrapper for OpenCode CLI.
pub struct OpencodeCliRuntime {
    inner: super::cli_runtime::UnifiedCliRuntime,
}

impl OpencodeCliRuntime {
    pub fn new() -> Self {
        Self {
            inner: super::cli_runtime::UnifiedCliRuntime {
                kind_str: "opencode",
                binary: "opencode",
                install_cmd: OPENCODE_INSTALL,
                parse_event: parse_opencode_event,
                build_args: build_opencode_args,
                perm_env: crate::ports::agent_runtime::opencode_permission_env,
            },
        }
    }
}

impl Default for OpencodeCliRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentRuntime for OpencodeCliRuntime {
    fn kind(&self) -> &'static str {
        "opencode"
    }

    async fn is_available(
        &self,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
    ) -> bool {
        self.inner.is_available(exec, machine_id).await
    }

    fn install_command(&self) -> &'static str {
        self.inner.install_command()
    }

    fn start(
        &self,
        ctx: AgentContext,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + '_>>
    {
        self.inner.start(ctx)
    }
}

pub fn runtime() -> OpencodeCliRuntime {
    OpencodeCliRuntime::new()
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/agent/opencode.rs"]
mod tests;
