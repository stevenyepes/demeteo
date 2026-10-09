//! Wire types for the Demeteo Hub socket, shared by the instance and the Hub.
//!
//! The trust model, the message set and the reasons behind them live in
//! [`docs/HUB.md`](../../../docs/HUB.md); this crate is the one place the
//! shapes are spelled, so the two ends cannot drift.
//!
//! What is deliberately absent is the point of the crate. There is no `logs`
//! scope and no `configure` scope, no `redirect` decision, and no field shaped
//! to carry a path, a transcript, a diff or a secret: a payload the protocol
//! cannot express is a payload no caller can leak.
//!
//! That holds for the structured fields only. A `String` carries whatever its
//! producer puts in it, and three hold material that can embed a path or a
//! credential: [`WorkflowVersionSnapshot::steps_json`],
//! [`WorkflowVersionSnapshot::definition_json`] and
//! [`ProjectSnapshot::remote_url`]. This crate cannot inspect them; what they
//! carry is the desktop snapshot builder's responsibility, stated on each type.
//!
//! The crate depends on `serde` and `serde_json` only — no `demeteo-core`, no
//! keyring, no I/O — so a Hub build pulls in nothing that holds an instance
//! secret.

mod canonical;
mod message;
mod request;
mod scope;
mod snapshot;

pub use canonical::{endorsement_challenge, gate_challenge, revocation_challenge, EncodeError};
pub use message::{
    CursorAck, Hello, RefusalReason, RequestHeader, RequestOutcome, RequestResult, RunEvent,
    RunEventKind, RunEventPayload,
};
pub use request::{
    AttachmentRef, CancelFeature, GateDecision, PasskeyEndorsement, PasskeyRevocation,
    RequestPayload, SetStepAssignment, SignedRequest, StartFeature, StepOverride,
    WebAuthnAssertion,
};
pub use scope::{Decision, Effort, Scope};
pub use snapshot::{ProjectSnapshot, RunShapeSettings, Snapshot, WorkflowVersionSnapshot};

/// Carried as `v` on every top-level message. Deserialisation never rejects a
/// different value; the mismatch policy belongs to the consumers.
pub const PROTOCOL_VERSION: u32 = 1;
