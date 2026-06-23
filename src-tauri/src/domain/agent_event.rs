use serde::{Deserialize, Serialize};

use crate::domain::action::ActionKind;
use crate::domain::artifact::Artifact;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Streamed assistant text delta. The frontend appends to the most recent text block.
    Text { delta: String },

    /// A durable artifact was produced (a file the agent just wrote,
    /// a derived diff, a worktree navigation pointer, etc.). The
    /// `StepExecutor` collects these into a per-step buffer and
    /// resolves them against the step's declared `ArtifactDecl`s at
    /// `TurnComplete`. **This is the cross-restart durable record** —
    /// text events are ephemeral UI signals, this is what survives.
    ArtifactProduced { artifact: Artifact },

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
#[path = "../../tests/domain/agent_event.rs"]
mod tests;
