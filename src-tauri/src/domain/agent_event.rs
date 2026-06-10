use serde::{Deserialize, Serialize};

use crate::domain::action::ActionKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Streamed assistant text delta. The frontend appends to the most recent text block.
    Text { delta: String },

    /// Agent wants to do something. The `tool_call_id` is the agent's id; the
    /// `intercept_id` is Demeteo's internal handle (always minted for traceability).
    ToolCall {
        tool_call_id: String,
        intercept_id: String,
        action: ActionKind,
        target: String,
        preview: Option<String>,
    },

    /// In-flight tool call update (status change, refreshed diff, etc.)
    ToolCallUpdate {
        tool_call_id: String,
        status: ToolCallStatus,
        preview: Option<String>,
    },

    /// Agent publishes an execution plan (opencode plan mode, etc.)
    Plan { entries: Vec<PlanEntry> },

    /// Token / cost telemetry
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Option<f64>,
    },

    /// Soft error from the agent
    Error {
        code: String,
        message: String,
        recoverable: bool,
    },

    /// Agent finished the turn. The channel closes after this.
    TurnComplete { stop_reason: StopReason },

    /// Agent switched modes (e.g., plan -> build). Carries the new mode id.
    ModeChanged { mode_id: String },

    /// Agent updated a config option (model, mode, reasoning level, etc.)
    ConfigChanged { config_id: String, value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    InProgress { message: Option<String> },
    Completed,
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEntry {
    pub step: String,
    pub status: String, // "pending" | "in_progress" | "done" | "blocked"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndOfTurn,
    Cancelled,
    MaxTokens,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_event_serializes_with_snake_case_kind() {
        let e = AgentEvent::Text { delta: "hi".into() };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"kind\":\"text\""));
    }

    #[test]
    fn turn_complete_serializes_with_snake_case_stop_reason() {
        let e = AgentEvent::TurnComplete { stop_reason: StopReason::EndOfTurn };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"stop_reason\":\"end_of_turn\""));
    }

    #[test]
    fn tool_call_status_failed_carries_reason() {
        let e = ToolCallStatus::Failed { reason: "no".into() };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"status\":\"failed\""));
        assert!(s.contains("\"reason\":\"no\""));
    }

    #[test]
    fn mode_changed_serializes_with_snake_case_kind() {
        let e = AgentEvent::ModeChanged { mode_id: "code".into() };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"kind\":\"mode_changed\""));
        assert!(s.contains("\"mode_id\":\"code\""));
    }

    #[test]
    fn config_changed_serializes_correctly() {
        let e = AgentEvent::ConfigChanged { config_id: "model".into(), value: "claude-4".into() };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"kind\":\"config_changed\""));
        assert!(s.contains("\"config_id\":\"model\""));
        assert!(s.contains("\"value\":\"claude-4\""));
    }
}
