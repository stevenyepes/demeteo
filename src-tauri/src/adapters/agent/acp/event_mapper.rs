use crate::domain::agent_event::{AgentEvent, PlanEntry, ToolCallStatus};
use crate::domain::action::ActionKind;
use serde_json::Value;

/// Map an ACP `session/update` notification (or `tool_call/update`) into
/// one or more `AgentEvent`s. The notification format follows the v1 wire
/// shape; unknown shapes produce a no-op so a future ACP version can
/// extend the protocol without breaking us.
///
/// Two wire shapes are supported, since the schema evolved during the v1
/// drafts:
///
/// 1. **Anthropic/Claude-style** (top-level `kind` discriminator):
///    ```json
///    {"kind": "text", "delta": "hello "}
///    {"kind": "tool_call", "toolCallId": "...", "action": "read", ...}
///    {"kind": "plan", "entries": [...]}
///    {"kind": "usage_update", "inputTokens": ..., "outputTokens": ...}
///    ```
///
/// 2. **Opencode-style** (nested `params.update.sessionUpdate` discriminator):
///    ```json
///    {"sessionId": "...", "update": {
///        "sessionUpdate": "agent_message_chunk" | "agent_thought_chunk" | ...,
///        "content": {"type": "text", "text": "..."},
///        "messageId": "..."
///    }}
///    ```
///    Where `sessionUpdate` values include:
///    - `agent_message_chunk`         → `AgentEvent::Text` (text content)
///    - `agent_thought_chunk`         → silently dropped (intermediate reasoning)
///    - `tool_call`                   → `AgentEvent::ToolCall`
///    - `tool_call_update`            → `AgentEvent::ToolCallUpdate`
///    - `plan`                        → `AgentEvent::Plan`
///    - `available_commands_update`   → silently dropped
///    - `current_mode_update`         → `AgentEvent::ModeChanged`
///    - `config_option_update`        → `AgentEvent::ConfigChanged` (one per option)
///    - `session_info_update`         → silently dropped
///    - `usage_update`                → `AgentEvent::Usage`
pub fn map_session_update(params: &Value) -> Vec<AgentEvent> {
    // Shape 1: top-level `kind`.
    if let Some(kind) = params.get("kind").and_then(|v| v.as_str()) {
        return map_session_update_kind(kind, params);
    }
    // Shape 2: nested `update.sessionUpdate`.
    let update = match params.get("update") {
        Some(u) => u,
        None => return vec![],
    };
    let discriminator = match update.get("sessionUpdate").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return vec![],
    };
    match discriminator {
        "agent_message_chunk" => {
            // Public-facing assistant text.
            let delta = update
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if delta.is_empty() {
                vec![]
            } else {
                vec![AgentEvent::Text { delta }]
            }
        }
        "agent_thought_chunk" => {
            // Internal reasoning. Don't surface to the user.
            vec![]
        }
        "tool_call" => {
            let tool_call_id = update
                .get("toolCallId")
                .or_else(|| update.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let raw = update.get("rawInput").or_else(|| update.get("input"));
            let action_str = raw
                .and_then(|r| r.get("action"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    update.get("kind").and_then(|v| v.as_str())
                })
                .unwrap_or("read");
            let action = action_kind_from_str(action_str);
            let target = raw
                .and_then(|r| r.get("path").or_else(|| r.get("cmd")))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    update.get("title").and_then(|v| v.as_str())
                })
                .unwrap_or("")
                .to_string();
            let preview = raw
                .and_then(|r| r.get("preview"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let intercept_id = format!("int_pending:{}", tool_call_id);
            vec![AgentEvent::ToolCall {
                tool_call_id,
                intercept_id,
                action,
                target,
                preview,
            }]
        }
        "tool_call_update" => return map_tool_call_update(update).into_iter().collect(),
        "plan" => {
            let entries: Vec<PlanEntry> = update
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let step = e.get("content").and_then(|s| s.as_str())
                                .or_else(|| e.get("step").and_then(|s| s.as_str()))?
                                .to_string();
                            let status = e.get("status").and_then(|s| s.as_str())?.to_string();
                            Some(PlanEntry { step, status })
                        })
                        .collect()
                })
                .unwrap_or_default();
            vec![AgentEvent::Plan { entries }]
        }
        "usage_update" => {
            let input = update
                .get("inputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output = update
                .get("outputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cost = update.get("costUsd").and_then(|v| v.as_f64());
            vec![AgentEvent::Usage {
                input_tokens: input,
                output_tokens: output,
                cost_usd: cost,
            }]
        }
        "current_mode_update" => {
            let mode_id = update
                .get("modeId")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            vec![AgentEvent::ModeChanged { mode_id }]
        }
        "config_option_update" => {
            let configs = update
                .get("configOptions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            let config_id = c.get("id").and_then(|v| v.as_str())?.to_string();
                            let value = c.get("currentValue").and_then(|v| v.as_str())?.to_string();
                            Some(AgentEvent::ConfigChanged { config_id, value })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            configs
        }
        // Benign no-op notifications: command list refresh, session info.
        "available_commands_update" | "session_info_update" => vec![],
        _ => vec![],
    }
}

fn map_session_update_kind(kind: &str, params: &Value) -> Vec<AgentEvent> {
    match kind {
        "text" => {
            let delta = params
                .get("delta")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            vec![AgentEvent::Text { delta }]
        }
        "tool_call" => {
            let tool_call_id = params
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let action_str = params
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("read");
            let action = action_kind_from_str(action_str);
            let target = params
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let preview = params
                .get("preview")
                .and_then(|v| v.as_str())
                .map(String::from);
            let intercept_id = format!("int_pending:{}", tool_call_id);
            vec![AgentEvent::ToolCall {
                tool_call_id,
                intercept_id,
                action,
                target,
                preview,
            }]
        }
        "plan" => {
            let entries: Vec<PlanEntry> = params
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let step = e.get("step").and_then(|s| s.as_str())?.to_string();
                            let status = e.get("status").and_then(|s| s.as_str())?.to_string();
                            Some(PlanEntry { step, status })
                        })
                        .collect()
                })
                .unwrap_or_default();
            vec![AgentEvent::Plan { entries }]
        }
        "usage_update" => {
            let input = params
                .get("inputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output = params
                .get("outputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cost = params.get("costUsd").and_then(|v| v.as_f64());
            vec![AgentEvent::Usage {
                input_tokens: input,
                output_tokens: output,
                cost_usd: cost,
            }]
        }
        _ => vec![],
    }
}

pub fn map_tool_call_update(params: &Value) -> Option<AgentEvent> {
    let tool_call_id = params
        .get("toolCallId")
        .and_then(|v| v.as_str())?
        .to_string();
    let status_raw = params.get("status")?;
    let status = match status_raw.as_str() {
        Some("pending") => ToolCallStatus::Pending,
        Some("in_progress") => ToolCallStatus::InProgress {
            message: params
                .get("message")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        Some("completed") => ToolCallStatus::Completed,
        Some("failed") => ToolCallStatus::Failed {
            reason: params
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        _ => return None,
    };
    let preview = params
        .get("preview")
        .and_then(|v| v.as_str())
        .map(String::from);
    Some(AgentEvent::ToolCallUpdate {
        tool_call_id,
        status,
        preview,
    })
}

pub fn map_usage_notification(params: &Value) -> Option<AgentEvent> {
    let input = params.get("inputTokens").and_then(|v| v.as_u64())?;
    let output = params.get("outputTokens").and_then(|v| v.as_u64())?;
    let cost = params.get("costUsd").and_then(|v| v.as_f64());
    Some(AgentEvent::Usage {
        input_tokens: input,
        output_tokens: output,
        cost_usd: cost,
    })
}

fn action_kind_from_str(s: &str) -> ActionKind {
    match s {
        "read" => ActionKind::Read,
        "edit" => ActionKind::Edit,
        "write" => ActionKind::Write,
        "run_bash" | "bash" => ActionKind::RunBash,
        _ => ActionKind::Read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent_event::StopReason;
    use serde_json::json;

    #[test]
    fn text_update_yields_text_event() {
        let v = json!({"kind": "text", "delta": "hello "});
        let mut events = map_session_update(&v);
        assert_eq!(events.len(), 1);
        match events.remove(0) {
            AgentEvent::Text { delta } => assert_eq!(delta, "hello "),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    #[test]
    fn tool_call_update_yields_tool_call_event() {
        let v = json!({
            "kind": "tool_call",
            "toolCallId": "tc-1",
            "action": "read",
            "target": "/etc/passwd",
            "preview": null
        });
        let mut events = map_session_update(&v);
        assert_eq!(events.len(), 1);
        match events.remove(0) {
            AgentEvent::ToolCall {
                tool_call_id,
                action,
                target,
                ..
            } => {
                assert_eq!(tool_call_id, "tc-1");
                assert_eq!(action, ActionKind::Read);
                assert_eq!(target, "/etc/passwd");
            }
            other => panic!("expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn plan_update_yields_plan_event() {
        let v = json!({
            "kind": "plan",
            "entries": [
                {"step": "read files", "status": "done"},
                {"step": "edit", "status": "in_progress"}
            ]
        });
        let mut events = map_session_update(&v);
        match events.remove(0) {
            AgentEvent::Plan { entries } => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].status, "done");
            }
            other => panic!("expected Plan, got {:?}", other),
        }
    }

    #[test]
    fn tool_call_update_status_serializes() {
        let v = json!({"toolCallId": "tc-2", "status": "failed", "reason": "denied"});
        let ev = map_tool_call_update(&v).unwrap();
        match ev {
            AgentEvent::ToolCallUpdate { tool_call_id, status, .. } => {
                assert_eq!(tool_call_id, "tc-2");
                match status {
                    ToolCallStatus::Failed { reason } => assert_eq!(reason, "denied"),
                    other => panic!("expected Failed, got {:?}", other),
                }
            }
            other => panic!("expected ToolCallUpdate, got {:?}", other),
        }
    }

    #[test]
    fn unknown_kind_yields_no_events() {
        let v = json!({"kind": "future_thing"});
        let events = map_session_update(&v);
        assert!(events.is_empty());
    }

    #[test]
    fn stop_reason_serializes_snake_case() {
        let ev = AgentEvent::TurnComplete { stop_reason: StopReason::MaxTokens };
        let s = serde_json::to_string(&ev).unwrap();
        assert!(s.contains("\"stop_reason\":\"max_tokens\""));
    }

    // -- opencode-style (nested `update.sessionUpdate`) -------------------

    #[test]
    fn opencode_message_chunk_yields_text_event() {
        let v = json!({
            "sessionId": "ses_x",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "messageId": "msg_1",
                "content": {"type": "text", "text": "Hello, world"}
            }
        });
        let mut events = map_session_update(&v);
        assert_eq!(events.len(), 1);
        match events.remove(0) {
            AgentEvent::Text { delta } => assert_eq!(delta, "Hello, world"),
            other => panic!("expected Text, got {:?}", other),
        }
    }

    #[test]
    fn opencode_thought_chunk_is_silently_dropped() {
        let v = json!({
            "sessionId": "ses_x",
            "update": {
                "sessionUpdate": "agent_thought_chunk",
                "content": {"type": "text", "text": "thinking..."}
            }
        });
        let events = map_session_update(&v);
        assert!(events.is_empty());
    }

    #[test]
    fn opencode_available_commands_is_silently_dropped() {
        let v = json!({
            "sessionId": "ses_x",
            "update": {
                "sessionUpdate": "available_commands_update",
                "availableCommands": []
            }
        });
        let events = map_session_update(&v);
        assert!(events.is_empty());
    }

    #[test]
    fn opencode_tool_call_yields_tool_call_event() {
        let v = json!({
            "sessionId": "ses_x",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tc_42",
                "title": "Read file",
                "rawInput": {
                    "action": "read",
                    "path": "/etc/passwd"
                }
            }
        });
        let mut events = map_session_update(&v);
        assert_eq!(events.len(), 1);
        match events.remove(0) {
            AgentEvent::ToolCall {
                tool_call_id,
                action,
                target,
                ..
            } => {
                assert_eq!(tool_call_id, "tc_42");
                assert_eq!(action, ActionKind::Read);
                assert_eq!(target, "/etc/passwd");
            }
            other => panic!("expected ToolCall, got {:?}", other),
        }
    }
}
