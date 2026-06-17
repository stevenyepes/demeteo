use crate::adapters::agent::cli_runtime::{CliAgentRuntime, EventParser};
use crate::domain::agent_event::{AgentEvent, StopReason};

/// Parse an Antigravity CLI JSON-lines event.
/// Antigravity CLI streams `{"type":"...","data":{"text":"..."}}` objects.
fn parse_antigravity_event(line: &str) -> Option<AgentEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v["type"].as_str()? {
        "text_delta" | "message" | "assistant" => {
            let text = v["data"]["text"].as_str()
                .or_else(|| v["content"].as_str())
                .or_else(|| v["text"].as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() { return None; }
            Some(AgentEvent::Text { delta: text })
        }
        "tool_call" | "tool_use" => {
            let tool = v["data"]["tool"].as_str()
                .or_else(|| v["name"].as_str())
                .unwrap_or("unknown");
            let input = v["data"]["input"].to_string();
            Some(AgentEvent::Text { delta: format!("[tool: {}] {}", tool, input) })
        }
        "done" | "end_turn" | "finish" => Some(AgentEvent::TurnComplete { stop_reason: StopReason::EndOfTurn }),
        "error" => {
            let msg = v["data"]["message"].as_str()
                .or_else(|| v["message"].as_str())
                .unwrap_or("unknown error")
                .to_string();
            Some(AgentEvent::Error { code: "cli_error".to_string(), message: msg, recoverable: false })
        }
        _ => None,
    }
}

pub fn runtime() -> CliAgentRuntime {
    CliAgentRuntime {
        kind_str: "antigravity",
        binary: "antigravity",
        extra_args: &["run", "--json"],
        install_cmd: "npm install -g @antigravity/cli",
        parse_event: parse_antigravity_event as EventParser,
    }
}
