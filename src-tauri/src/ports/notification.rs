use serde::{Deserialize, Serialize};

use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::intercept::{ExecutionResult, InterceptPayload};

/// The set of events the orchestrator emits to the UI.
///
/// All variants serialise to a JSON body whose `kind` tag is the event
/// name (e.g. `"feature_status_changed"`). The body shape for each
/// variant mirrors the legacy per-method payload 1:1, so the wire
/// format is byte-identical to the previous 6-method port surface.
/// See the documentation in `docs/DECISIONS.md` for details on system events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DomainEvent {
    /// Emitted when an `AgentAction` has been intercepted and needs
    /// user approval. The full payload is included so the UI can
    /// render the preview without a follow-up DB lookup.
    PermissionRequested(InterceptPayload),

    /// Emitted after an action executes (or is rejected). `intercept_id`
    /// is `Some(_)` when the action was the resolution of a previously
    /// emitted `PermissionRequested`, otherwise `None`.
    CommandExecuted {
        thread_id: String,
        machine_id: String,
        result: ExecutionResult,
        intercept_id: Option<String>,
    },

    /// Emitted when a feature's overall status changes
    /// (e.g. "running" → "completed").
    FeatureStatusChanged {
        feature_id: FeatureId,
        status: String,
    },

    /// Emitted on every step state transition inside a feature, with
    /// accumulated cost, tokens and elapsed time so the UI can render progress
    /// without a poll.
    StepProgress {
        feature_id: FeatureId,
        step_id: String,
        status: String,
        cost_usd: Option<f64>,
        tokens: Option<i64>,
        wall_clock_secs: Option<u64>,
    },

    /// Emitted when a step of kind `gate` finishes and is waiting on
    /// user input.
    GateRequired {
        feature_id: FeatureId,
        step_execution_id: StepExecutionId,
    },

    /// Emitted when the merge executor detects a conflict between two
    /// subtask branches on the same feature.
    ConflictDetected {
        feature_id: FeatureId,
        subtask_id: String,
    },

    /// Emitted when an agent generates stdout stream text.
    AgentStream {
        feature_id: FeatureId,
        step_execution_id: StepExecutionId,
        content: String,
    },
}

/// The single deep interface for orchestrator → UI event emission.
///
/// Collapsed from 6 near-identical `emit_*` methods (R1 of the
/// deep-modules refactor). The Tauri adapter is a single `match` over
/// [`DomainEvent`]; the wire format is unchanged.
pub trait NotificationPort: Send + Sync {
    fn emit(&self, event: &DomainEvent) -> Result<(), String>;
}
