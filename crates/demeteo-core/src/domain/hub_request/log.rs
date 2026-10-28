//! One row of the Hub activity log (`hub_request_log`, V65), as the
//! Settings › Hub tab shows it.
//!
//! There is deliberately no field for a description, a title, an attachment
//! or anything secret: the log says what was asked and what was answered,
//! never what the request carried, save the credential ids of a change to the
//! trusted set ([`trust_change_note`]).

use demeteo_hub_protocol::{RequestPayload, Scope};
use serde::Serialize;

/// The request's `kind`, or `Unknown` for a body that did not parse as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    StartFeature,
    CancelFeature,
    SetStepAssignment,
    GateDecision,
    PasskeyEndorsement,
    PasskeyRevocation,
    Unknown,
}

impl LogKind {
    pub fn of(payload: &RequestPayload) -> Self {
        match payload {
            RequestPayload::StartFeature(_) => Self::StartFeature,
            RequestPayload::CancelFeature(_) => Self::CancelFeature,
            RequestPayload::SetStepAssignment(_) => Self::SetStepAssignment,
            RequestPayload::GateDecision(_) => Self::GateDecision,
            RequestPayload::PasskeyEndorsement(_) => Self::PasskeyEndorsement,
            RequestPayload::PasskeyRevocation(_) => Self::PasskeyRevocation,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StartFeature => "start_feature",
            Self::CancelFeature => "cancel_feature",
            Self::SetStepAssignment => "set_step_assignment",
            Self::GateDecision => "gate_decision",
            Self::PasskeyEndorsement => "passkey_endorsement",
            Self::PasskeyRevocation => "passkey_revocation",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "start_feature" => Some(Self::StartFeature),
            "cancel_feature" => Some(Self::CancelFeature),
            "set_step_assignment" => Some(Self::SetStepAssignment),
            "gate_decision" => Some(Self::GateDecision),
            "passkey_endorsement" => Some(Self::PasskeyEndorsement),
            "passkey_revocation" => Some(Self::PasskeyRevocation),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogOutcome {
    Accepted,
    Refused,
    /// No answer was sent: the signature did not verify.
    Dropped,
    /// No answer was sent and the id was not consumed, so a redelivery is
    /// judged afresh.
    Deferred,
}

impl LogOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Refused => "refused",
            Self::Dropped => "dropped",
            Self::Deferred => "deferred",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "accepted" => Some(Self::Accepted),
            "refused" => Some(Self::Refused),
            "dropped" => Some(Self::Dropped),
            "deferred" => Some(Self::Deferred),
            _ => None,
        }
    }
}

/// The protocol's own `Scope` spelling, so the column reads the same as the
/// wire.
pub fn scope_as_str(scope: Scope) -> &'static str {
    match scope {
        Scope::Read => "read",
        Scope::Spend => "spend",
        Scope::Gates => "gates",
    }
}

pub fn parse_scope(s: &str) -> Option<Scope> {
    match s {
        "read" => Some(Scope::Read),
        "spend" => Some(Scope::Spend),
        "gates" => Some(Scope::Gates),
        _ => None,
    }
}

/// How much of a launch failure's detail a row keeps, in chars.
const LAUNCH_DETAIL_MAX_CHARS: usize = 200;

/// The shortest `withheld` string [`launch_failure_reason`] cuts out, in
/// chars. A title of one letter would otherwise shred every word of the
/// detail that letter appears in.
pub const MIN_WITHHELD_CHARS: usize = 4;

/// The most rows one read of the activity log returns, whatever the caller
/// asks for.
pub const MAX_LOG_PAGE: usize = 500;

/// The `reason` of a start refused because its launch failed after every
/// check passed: the `malformed_body` the Hub was told, then the local
/// detail with each `withheld` string of at least [`MIN_WITHHELD_CHARS`]
/// cut out, longest first so one that contains another goes whole. The
/// detail is the launcher's error text, which can quote the request back (a
/// title in a branch name, a description in a validation message), so the
/// caller withholds the request's own text and each form it is rewritten into
/// on the way, rather than trusting either to be absent. Only exact copies
/// match: a form the caller does not pass survives.
pub fn launch_failure_reason(detail: &str, withheld: &[&str]) -> String {
    let mut withheld: Vec<&str> = withheld
        .iter()
        .copied()
        .filter(|text| text.chars().count() >= MIN_WITHHELD_CHARS)
        .collect();
    withheld.sort_by_key(|text| std::cmp::Reverse(text.len()));
    let mut detail = detail.to_owned();
    for text in withheld {
        detail = detail.replace(text, "[withheld]");
    }
    let detail: String = detail.chars().take(LAUNCH_DETAIL_MAX_CHARS).collect();
    format!("malformed_body: {detail}")
}

/// A change to the trusted passkey set that was written, by credential id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustChange<'a> {
    Endorsed {
        credential_id: &'a str,
        signer: &'a str,
    },
    Revoked {
        credential_id: &'a str,
        signer: &'a str,
    },
}

/// The `reason` of an accepted `passkey_endorsement` or `passkey_revocation`:
/// the key the set gained or lost, and the trusted key that signed for it.
/// A change is applied with no prompt, so this row is the desktop's only
/// record of it; no endorsement is accepted today (docs/HUB.md §11 item 20).
/// A credential id is a public identifier; the public key and the assertion
/// are never part of the note.
pub fn trust_change_note(change: TrustChange<'_>) -> String {
    match change {
        TrustChange::Endorsed {
            credential_id,
            signer,
        } => format!("endorsed {credential_id}, signed by {signer}"),
        TrustChange::Revoked {
            credential_id,
            signer,
        } => format!("revoked {credential_id}, signed by {signer}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HubRequestLogEntry {
    pub request_id: String,
    pub kind: LogKind,
    /// The scope the kind spends; `None` for the passkey kinds and for a body
    /// that did not parse.
    pub scope: Option<Scope>,
    pub outcome: LogOutcome,
    /// A `RefusalReason` in its snake_case spelling, `signature` for a drop,
    /// a note on a field that was ignored, [`launch_failure_reason`], or
    /// [`trust_change_note`].
    pub reason: Option<String>,
    pub feature_id: Option<String>,
    pub received_at_ms: i64,
}
