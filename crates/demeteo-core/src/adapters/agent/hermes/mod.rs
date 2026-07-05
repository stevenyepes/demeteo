//! Hermes agent from Nous Research. Speaks CLI mode with `--format json`.

use crate::adapters::agent::cli_runtime::{EventParser, UnifiedCliRuntime};
use crate::domain::agent_event::{AgentEvent, StopReason, Usage};
use crate::ports::agent_runtime::AgentContext;

pub const HERMES_INSTALL: &str =
    "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash";

fn parse_hermes_event(line: &str) -> Option<AgentEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    if let Some(kind) = v
        .get("kind")
        .or_else(|| v.get("type"))
        .and_then(|v| v.as_str())
    {
        match kind {
            "text" | "message" | "assistant" => {
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
                let input =
                    serde_json::to_string(&v.get("input").unwrap_or(&serde_json::Value::Null))
                        .ok()?;
                Some(AgentEvent::Text {
                    delta: format!("[tool: {}] {}", tool, input),
                })
            }
            "usage" | "usage_update" => {
                let input_tokens = v.get("inputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let output_tokens = v.get("outputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let cost_usd = v.get("costUsd").and_then(|v| v.as_f64());
                let cache_read_input_tokens = v
                    .get("cacheReadInputTokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let cache_creation_input_tokens = v
                    .get("cacheCreationInputTokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                Some(AgentEvent::Usage(Usage {
                    input_tokens,
                    output_tokens,
                    cost_usd,
                    cache_read_input_tokens,
                    cache_creation_input_tokens,
                }))
            }
            "end_turn" | "message_stop" | "done" => Some(AgentEvent::TurnComplete {
                stop_reason: StopReason::EndOfTurn,
                usage: None,
            }),
            "error" => {
                let message = v
                    .get("message")
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
    } else if let Some(update) = v.get("update") {
        if let Some(discriminator) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
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
                "agent_thought_chunk" => None,
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
                    let cache_read_input_tokens = update
                        .get("cacheReadInputTokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let cache_creation_input_tokens = update
                        .get("cacheCreationInputTokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    Some(AgentEvent::Usage(Usage {
                        input_tokens,
                        output_tokens,
                        cost_usd,
                        cache_read_input_tokens,
                        cache_creation_input_tokens,
                    }))
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

/// Construct command-line arguments for Hermes CLI.
fn build_hermes_args(ctx: &AgentContext, captured_session_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    // Cross-step continuation. Hermes's `--resume <sid>` (alias `-r`) is
    // the explicit form of `--continue`; we use the explicit id so the
    // orchestrator can thread the captured session id through and Hermes
    // replays the full conversation (system prompt + tools + prior turns)
    // from its local SQLite at `~/.hermes/state.db`. This unlocks the
    // vendor-side 1-hour `cache_control` TTL on the static prefix across
    // steps in the same feature.
    if let Some(sid) = captured_session_id {
        args.push("--resume".to_string());
        args.push(sid.to_string());
    }
    if let Some(ref m) = ctx.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }
    args
}

pub fn runtime() -> UnifiedCliRuntime {
    UnifiedCliRuntime {
        kind_str: "hermes",
        binary: "hermes",
        install_cmd: HERMES_INSTALL,
        parse_event: parse_hermes_event as EventParser,
        build_args: build_hermes_args,
        perm_env: crate::ports::agent_runtime::opencode_permission_env,
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/agent/hermes.rs"]
mod tests;
