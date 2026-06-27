use crate::adapters::agent::cli_runtime::{EventParser, UnifiedCliRuntime};
use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::ports::agent_runtime::AgentContext;

/// Parse an Antigravity CLI JSON-lines event.
/// Antigravity CLI streams `{"type":"...","data":{"text":"..."}}` objects.
fn parse_antigravity_event(line: &str) -> Option<AgentEvent> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
        match v["type"].as_str()? {
            "text_delta" | "message" | "assistant" => {
                let text = v["data"]["text"]
                    .as_str()
                    .or_else(|| v["content"].as_str())
                    .or_else(|| v["text"].as_str())
                    .unwrap_or("")
                    .to_string();
                if text.is_empty() {
                    return None;
                }
                Some(AgentEvent::Text { delta: text })
            }
            "tool_call" | "tool_use" => {
                let tool = v["data"]["tool"]
                    .as_str()
                    .or_else(|| v["name"].as_str())
                    .unwrap_or("unknown");
                let input = v["data"]["input"].to_string();
                Some(AgentEvent::Text {
                    delta: format!("[tool: {}] {}", tool, input),
                })
            }
            "done" | "end_turn" | "finish" => Some(AgentEvent::TurnComplete {
                stop_reason: StopReason::EndOfTurn,
                usage: None,
            }),
            "error" => {
                let msg = v["data"]["message"]
                    .as_str()
                    .or_else(|| v["message"].as_str())
                    .unwrap_or("unknown error")
                    .to_string();
                Some(AgentEvent::Error {
                    code: "cli_error".to_string(),
                    message: msg,
                    recoverable: false,
                })
            }
            _ => None,
        }
    } else if line.is_empty() {
        None
    } else {
        Some(AgentEvent::Text {
            delta: format!("{}\n", line),
        })
    }
}

/// Construct command-line arguments for Antigravity CLI (`agy`).
fn build_antigravity_args(ctx: &AgentContext, captured_session_id: Option<&str>) -> Vec<String> {
    let mut args = vec!["--print".to_string(), "-".to_string()];

    // Auto-approve tool permissions to run non-interactively without blocking
    args.push("--dangerously-skip-permissions".to_string());

    if let Some(sid) = captured_session_id {
        args.push("--conversation".to_string());
        args.push(sid.to_string());
        args.push("-c".to_string());
    }
    if let Some(ref m) = ctx.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }
    args
}

pub fn runtime() -> UnifiedCliRuntime {
    UnifiedCliRuntime {
        kind_str: "antigravity",
        binary: "agy",
        install_cmd: "npm install -g @antigravity/cli",
        parse_event: parse_antigravity_event as EventParser,
        build_args: build_antigravity_args,
        perm_env: crate::ports::agent_runtime::opencode_permission_env,
    }
}
