//! Hermes agent from Nous Research. Speaks CLI mode with `--format json`.

use crate::adapters::agent::cli_runtime::{CliAgentRuntime, EventParser};
use crate::domain::agent_event::{AgentEvent, StopReason};

pub const HERMES_INSTALL: &str = "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash";

fn parse_hermes_event(line: &str) -> Option<AgentEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    if let Some(kind) = v.get("kind").or_else(|| v.get("type")).and_then(|v| v.as_str()) {
        match kind {
            "text" | "message" | "assistant" => {
                let delta = v.get("delta")
                    .or_else(|| v.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if delta.is_empty() { None } else { Some(AgentEvent::Text { delta }) }
            }
            "tool_call" | "tool_use" => {
                let tool = v.get("name").or_else(|| v.get("tool")).and_then(|v| v.as_str()).unwrap_or("unknown");
                let input = serde_json::to_string(&v.get("input").unwrap_or(&serde_json::Value::Null)).ok()?;
                Some(AgentEvent::Text { delta: format!("[tool: {}] {}", tool, input) })
            }
            "usage" | "usage_update" => {
                let input_tokens = v.get("inputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let output_tokens = v.get("outputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let cost_usd = v.get("costUsd").and_then(|v| v.as_f64());
                Some(AgentEvent::Usage { input_tokens, output_tokens, cost_usd })
            }
            "end_turn" | "message_stop" | "done" => {
                Some(AgentEvent::TurnComplete { stop_reason: StopReason::EndOfTurn })
            }
            "error" => {
                let message = v.get("message").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                Some(AgentEvent::Error { code: "cli_error".to_string(), message, recoverable: false })
            }
            _ => None,
        }
    } else if let Some(update) = v.get("update") {
        if let Some(discriminator) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
            match discriminator {
                "agent_message_chunk" => {
                    let delta = update.get("content")
                        .and_then(|c| c.get("text"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if delta.is_empty() { None } else { Some(AgentEvent::Text { delta }) }
                }
                "agent_thought_chunk" => None,
                "usage_update" => {
                    let input_tokens = update.get("inputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let output_tokens = update.get("outputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    let cost_usd = update.get("costUsd").and_then(|v| v.as_f64());
                    Some(AgentEvent::Usage { input_tokens, output_tokens, cost_usd })
                }
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    }
}

pub fn runtime() -> CliAgentRuntime {
    CliAgentRuntime {
        kind_str: "hermes",
        binary: "hermes",
        extra_args: &["run", "--format", "json"],
        install_cmd: HERMES_INSTALL,
        parse_event: parse_hermes_event as EventParser,
    }
}
