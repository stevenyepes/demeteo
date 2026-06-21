use crate::adapters::agent::cli_runtime::{EventParser, UnifiedCliRuntime};
use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::ports::agent_runtime::AgentContext;

/// Parse a Claude Code JSON-lines event.
/// Claude Code streams `{"type":"...","content":"..."}` objects.
fn parse_claude_event(line: &str) -> Option<AgentEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v["type"].as_str()? {
        "text" | "assistant" => {
            let text = v["content"]
                .as_str()
                .or_else(|| v["text"].as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                return None;
            }
            Some(AgentEvent::Text { delta: text })
        }
        "tool_use" => {
            let tool = v["name"].as_str().unwrap_or("unknown");
            let input = v["input"].to_string();
            Some(AgentEvent::Text {
                delta: format!("[tool: {}] {}", tool, input),
            })
        }
        "end_turn" | "message_stop" => Some(AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
        }),
        "error" => {
            let msg = v["message"].as_str().unwrap_or("unknown error").to_string();
            Some(AgentEvent::Error {
                code: "cli_error".to_string(),
                message: msg,
                recoverable: false,
            })
        }
        _ => None,
    }
}

/// Construct command-line arguments for Claude Code.
fn build_claude_args(ctx: &AgentContext, _captured_session_id: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--print".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
    ];
    if let Some(ref m) = ctx.model {
        args.push("--model".to_string());
        args.push(m.clone());
    }
    args
}

pub fn runtime() -> UnifiedCliRuntime {
    UnifiedCliRuntime {
        kind_str: "claude-code",
        binary: "claude",
        install_cmd: "npm install -g @anthropic-ai/claude-code",
        parse_event: parse_claude_event as EventParser,
        build_args: build_claude_args,
    }
}
