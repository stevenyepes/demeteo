use serde::{Deserialize, Serialize};

use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::intercept::{ExecutionResult, InterceptPayload};
use crate::domain::models::EffortLevel;

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

    /// Emitted for each sub-step of the feature *bootstrap* phase — the
    /// work between "user clicked Launch" and "the first DAG step runs"
    /// (resolve context / SSH handshake / sync origin / create branch /
    /// register steps / start pipeline). Lets the UI animate an inline
    /// stepper instead of blocking on a spinner while the (possibly
    /// remote, possibly network-bound) bootstrap runs.
    ///
    /// `phase` is a stable machine id (e.g. `"connecting"`,
    /// `"syncing_origin"`) the frontend orders by; `label` is the
    /// human-readable text the frontend renders verbatim so the phase
    /// vocabulary lives in one place (the emitter). `status` is one of
    /// `"running" | "completed" | "failed" | "skipped"`. `detail`
    /// carries an optional log line or, for `"failed"`, the error text.
    BootstrapProgress {
        feature_id: FeatureId,
        phase: String,
        label: String,
        status: String,
        detail: Option<String>,
    },

    /// Emitted on every step state transition inside a feature, with
    /// accumulated cost, tokens, cache-savings telemetry, and elapsed
    /// time so the UI can render progress without a poll.
    ///
    /// `cache_read_input_tokens` and `cache_creation_input_tokens`
    /// are populated from the agent's `Usage` / `TurnComplete` events
    /// (opencode / hermes / claude-code all report these). The UI
    /// surfaces the implied $ savings based on the active model's
    /// pricing table.
    StepProgress {
        feature_id: FeatureId,
        step_id: String,
        status: String,
        cost_usd: Option<f64>,
        tokens: Option<i64>,
        wall_clock_secs: Option<u64>,
        cache_read_input_tokens: Option<u64>,
        cache_creation_input_tokens: Option<u64>,
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

    /// Emitted just before a step's agent session is spawned, recording what
    /// the agent was *actually* launched with.
    ///
    /// `effort` is the **effective** (post-clamp) level — what the adapter
    /// will really put on argv/env — not the level the user asked for. That
    /// distinction is the whole point of the event: it is the only way a user
    /// can tell that codex clamped `max` down to `xhigh`, that hermes injected
    /// no effort at all (`None`), or that a `demeteo-runner` older than the
    /// desktop app dropped the unknown `RunSpec::effort` field and ran at the
    /// agent's own default while the UI claimed `high` (AGENTS.md §9.1 —
    /// version skew is mitigated by observability, not prevention).
    AgentSpawned {
        feature_id: FeatureId,
        step_execution_id: StepExecutionId,
        agent_kind: String,
        model: Option<String>,
        /// `None` = no effort was injected on this spawn at all.
        effort: Option<EffortLevel>,
    },

    /// Emitted when an agent generates stdout stream text.
    AgentStream {
        feature_id: FeatureId,
        step_execution_id: StepExecutionId,
        content: String,
    },

    /// Emitted by the background MR-state monitor when
    /// `MrPublisher::fetch_mr_state` reports an MR has transitioned
    /// to `merged`. Carries the project + title so the notification
    /// bell can render without a follow-up DB lookup. The
    /// `notification_persistence` adapter is what translates this
    /// into a `Notification` row.
    MrMerged {
        feature_id: FeatureId,
        project_id: String,
        feature_title: String,
        mr_url: String,
    },

    /// Emitted when a step's `on_failure` redirect chain has
    /// exhausted its retry budget. The failing step's row is
    /// already marked `failed` with a "retry budget exhausted"
    /// error message; this event is the user-visible signal
    /// (notification bell entry + toast) that the engine gave
    /// up after `max_iterations` attempts and the user needs to
    /// intervene (e.g. by editing the spec, adjusting the
    /// workflow's `on_failure` target, or picking a different
    /// model). The `attempt` / `max` counts are included so the
    /// UI can render "3 of 3 attempts" without a follow-up DB
    /// lookup. `target_id` is the step the loop kept trying to
    /// jump to (e.g. `"s-implement"`) so the UI can deep-link
    /// to a useful place.
    RetryBudgetExhausted {
        feature_id: FeatureId,
        step_id: String,
        target_id: String,
        attempt: u32,
        max: u32,
        reason: String,
    },

    /// Emitted when a harness failure was triaged (C6) as an *environment*
    /// problem — the box is missing a system library / toolchain / service, or
    /// has a permission/network fault the coding agent cannot fix by editing
    /// source. Fired *immediately* (before the retry budget is spent), so the
    /// user is told to provision the machine instead of watching doomed
    /// retries. `reason` is the full remediation message (what to install, the
    /// failing command, and a copy-pasteable reproduce line).
    EnvironmentNotReady {
        feature_id: FeatureId,
        step_id: String,
        reason: String,
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
