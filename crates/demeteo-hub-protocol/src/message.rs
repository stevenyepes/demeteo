use serde::{Deserialize, Serialize};

use crate::Scope;

/// First message an instance sends on a fresh socket. `scopes` is what the
/// user has allowed *now*; it is a report, never a request — nothing in this
/// protocol enables a scope (see [`Scope`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub v: u32,
    pub install_id: String,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    pub scopes: Vec<Scope>,
}

/// What happened to a Feature, Step or Ticket. A closed allowlist, not a copy
/// of the instance's free-string event kinds: a kind the instance has no
/// variant for is dropped before it reaches the socket.
///
/// Deliberately no `#[serde(other)]`: an unknown kind must fail to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventKind {
    FeatureStarted,
    FeatureCompleted,
    FeatureFailed,
    FeatureCancelled,
    StepStarted,
    StepCompleted,
    StepFailed,
    TicketStarted,
    TicketCompleted,
    TicketFailed,
    GatePending,
    GateDecided,
}

/// Which Step, execution and Ticket an event concerns — ids only.
///
/// There is deliberately no message, error or summary field. Free text here
/// would be a leak: transcripts, agent output, diffs and file contents never
/// leave an instance (HUB.md §3), and the only way to guarantee that is a
/// payload with no place to put them. Do not add a `String` that is not an id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEventPayload {
    #[serde(default)]
    pub step_id: Option<String>,
    #[serde(default)]
    pub step_execution_id: Option<String>,
    #[serde(default)]
    pub ticket_id: Option<String>,
}

/// One entry of the instance's run log, relayed to the Hub. `offset` is the
/// table-global monotonic `run_events.id` (HUB.md §5), so the Hub's cursor is
/// a single number across Features.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEvent {
    pub v: u32,
    pub feature_id: String,
    pub offset: u64,
    pub kind: RunEventKind,
    /// Unix seconds.
    pub occurred_at: u64,
    pub payload: RunEventPayload,
}

/// Why an instance refused a request. A refusal is always one of these, never
/// free text: a reason string could carry a path or an error message, and
/// nothing like that leaves an instance (HUB.md §3).
///
/// `Expired` is a refusal reason, not a third outcome: [`RequestOutcome`] has
/// exactly two variants.
///
/// Deliberately no `#[serde(other)]`: an unknown reason must fail to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    ScopeOff,
    Expired,
    Replayed,
    WrongInstance,
    UnknownKind,
    MalformedBody,
    OverRunCeiling,
    OverRollingCeiling,
    UnknownProject,
    UnknownWorkflow,
    UnknownFeature,
    UnknownStep,
    GateNotPending,
    CredentialNotPinned,
    AssertionInvalid,
    StepNotReassignable,
    HarnessNotRegistered,
    AttachmentMismatch,
    LastCredential,
    SelfRevocation,
}

/// What the instance did with a request: it acted, or it refused for a reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RequestOutcome {
    Accepted,
    Refused { reason: RefusalReason },
}

/// The instance's answer to one request, keyed by the request's own id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestResult {
    pub v: u32,
    pub request_id: String,
    pub outcome: RequestOutcome,
}

/// Just enough of a request to answer it.
///
/// Deliberately without `deny_unknown_fields`: a request whose `payload.kind`
/// does not parse still has a readable `request_id`, so the consumer can answer
/// `refused` instead of leaving the Hub waiting. Do not add a field here that a
/// request might lack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestHeader {
    pub request_id: String,
}

/// The Hub's acknowledgement that every event up to `offset` is stored
/// durably; the instance may resume from there after a reconnect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorAck {
    pub v: u32,
    pub offset: u64,
}
