use serde::{Deserialize, Serialize};

use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::intercept::{ExecutionResult, InterceptPayload};
use crate::domain::models::EffortLevel;
use crate::domain::sync_session::SyncSessionStatus;

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

    /// Emitted when a feature's sync session moves, so a pane rendering that
    /// row learns about a resolution it did not press for.
    ///
    /// The run's own progress cannot stand in for this. A sync records itself
    /// on a step every reader of a *run* excludes by design (the frontend's
    /// `isOutOfBandStep`), so a resolution running in the background moves
    /// nothing an open pane watches: it read the row when it mounted, and
    /// without this it reads it again only when a person presses something.
    ///
    /// `status` says which transition is being announced and is deliberately
    /// not what the pane renders — it re-reads through `sync_session_get`,
    /// which reconciles the row against the worktree on the way out, and a
    /// status taken from here would be the one reading that skipped that check.
    SyncStatusChanged {
        feature_id: FeatureId,
        status: SyncSessionStatus,
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

    /// A terminal-embedded agent (e.g. Claude Code) blocked on a permission/
    /// approval prompt and needs a human decision. Fires the OS notification when
    /// demeteo is backgrounded/unfocused (TERMINAL_ACTIVITY §3). The in-app
    /// surface is the activity indicator, so this variant exists mainly for the
    /// gated OS-notification path.
    TerminalAwaitingApproval {
        session_id: String,
        label: Option<String>,
    },

    /// Emitted when the retry-policy engine (P1.10) answers a step
    /// failure — every failure names the rule that decided its fate, not
    /// just the exhaustion case ([`RetryBudgetExhausted`] stays the
    /// user-facing alarm; this is the narrative record, P1.13).
    ///
    /// `rule_id` is the applied policy rule (`<class>.<strategy>`, e.g.
    /// `verdict.redirect`), identical to what the attempt row stores in
    /// `step_attempts.applied_rule`. `action` is what the driver actually
    /// did: `"redirect" | "in_place" | "exhausted" | "fail"`. `target_id`
    /// names the redirect target when one applies. `attempt`/`max` mirror
    /// the decision's budget arithmetic (`attempt` is the 1-based attempt
    /// a granted retry starts; for `exhausted`/`fail` it is the attempt
    /// that would have started had budget remained).
    ///
    /// [`RetryBudgetExhausted`]: DomainEvent::RetryBudgetExhausted
    RetryDecision {
        feature_id: FeatureId,
        step_id: String,
        /// Failure class (`environment | verdict | agent_failure |
        /// non_retryable`) — also the prefix of `rule_id`, kept explicit
        /// so consumers don't parse identifiers.
        error_class: String,
        rule_id: String,
        action: String,
        target_id: Option<String>,
        attempt: u32,
        max: u32,
        reason: String,
    },

    /// Emitted when a human (or policy) decision is applied to a gate —
    /// the moment `gate_decide` durably records `approve`/`reject`/
    /// `redirect`. [`GateRequired`] marks the wait; this marks the
    /// answer, so the run-event log tells both halves of the story
    /// (P1.13).
    ///
    /// [`GateRequired`]: DomainEvent::GateRequired
    GateDecided {
        feature_id: FeatureId,
        step_execution_id: StepExecutionId,
        decision: String,
        feedback: Option<String>,
    },

    /// A pull request reached a terminal state and, with it, one or more
    /// Tickets in a Discovery became startable (`docs/PRD_DISCOVERY.md` §6.4).
    ///
    /// Fired only when the set is non-empty: a merge that released nothing is
    /// not news, and a notice that arrived on every PR transition would be
    /// trained away. `message` is pre-phrased for the bell on the same terms
    /// as [`Notification::message`](crate::domain::models::Notification), and
    /// `ticket_ids` lets the open Discovery surface highlight what changed
    /// without a re-read.
    TicketsStartable {
        project_id: String,
        discovery_id: String,
        discovery_title: String,
        ticket_ids: Vec<String>,
        message: String,
    },

    /// A row was appended to the durable `run_events` log (P1.13). This
    /// is the live-push half of the unified event log: the local
    /// recorder appends the row, then forwards this variant so the UI
    /// receives **exactly the record shape** the remote path polls via
    /// `stream_events` — same `kind` vocabulary, same `payload_json`,
    /// plus the monotonic `offset` for gap-free catch-up. For local runs
    /// `run_id` is the feature id (local runs have no runner run row).
    ///
    /// Never itself recorded to the log (that would recurse); adapters
    /// other than the UI emitter ignore it.
    RunEventAppended {
        run_id: String,
        offset: i64,
        /// The stored row's `kind` (named `event_kind` here only because
        /// the enum's own serde tag claims `kind`; the UI emitter
        /// re-emits the bare record shape with `kind`).
        event_kind: String,
        payload_json: String,
        created_at: i64,
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
