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

    /// Token / cost telemetry. Emitted standalone by opencode and hermes
    /// (multiple times per turn); attached to `TurnComplete.usage` by
    /// Claude Code (one final snapshot per turn).
    ///
    /// `cache_read_input_tokens` and `cache_creation_input_tokens` are
    /// emitted by Claude Code today; opencode and hermes emit `0` until
    /// their wire formats expose them. The shared
    /// [`crate::domain::usage::UsageAccumulator`] treats all four
    /// numeric fields as monotonically cumulative per turn.
    Usage(Usage),

    /// Soft error from the agent
    Error {
        code: String,
        message: String,
        recoverable: bool,
    },

    /// Agent finished the turn. The channel closes after this.
    ///
    /// `usage` carries the terminal cumulative token/cost snapshot for
    /// the turn when the agent's wire format bundles them onto the
    /// result line (Claude Code). Parsers that emit usage as separate
    /// `Usage` events leave this `None`; the shared
    /// [`crate::domain::usage::UsageAccumulator`] handles both shapes.
    TurnComplete {
        stop_reason: StopReason,
        usage: Option<Usage>,
    },

    /// Agent switched modes (e.g., plan -> build). Carries the new mode id.
    ModeChanged { mode_id: String },

    /// Agent updated a config option (model, mode, reasoning level, etc.)
    ConfigChanged { config_id: String, value: String },
}

/// Token / cost snapshot.
///
/// A standalone struct (rather than an inline enum variant) so that the
/// `TurnComplete { usage: Option<Usage> }` carrier can hold the same
/// shape as the standalone `Usage` event without a self-referential
/// enum.
///
/// `cost_usd` is a client-side estimate from the agent's own bundled
/// price table (per Anthropic SDK cost-tracking docs). When `None`, the
/// [`UsageAccumulator`](crate::domain::usage::UsageAccumulator) will
/// compute a fallback from `PricingTable` if the model is known.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
