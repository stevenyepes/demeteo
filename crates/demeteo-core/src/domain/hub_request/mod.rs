//! What the desktop does with one Hub-relayed request — the policy half
//! (docs/HUB.md §6.1, §7).
//!
//! See [`crate::domain`] for why nothing here is `async`. The decision is
//! split in two around the one write it depends on, the consumption of the
//! request id: [`precheck`] decides whether the id is spent at all, the
//! caller records it atomically, and [`decide`] reads the result. The id is
//! spent once signature, instance and expiry pass and before any effect, and
//! is never refunded, so a request that later fails a ceiling or a lookup is
//! still a replay the second time it arrives.

use demeteo_hub_protocol::{
    AttachmentRef, CancelFeature, Effort, RefusalReason, RequestPayload, Scope, SetStepAssignment,
    SignedRequest, StartFeature, StepOverride as WireStepOverride,
};
use serde::{Deserialize, Serialize};

use crate::domain::models::{EffortLevel, StepExecution, StepOverride};
use crate::domain::run_control::{out_of_band_refusal, RunAction};
use crate::domain::step_assignment::{
    assignment_refusal, harness_refusal, identifier_refusal, kind_refusal, StepAssignment,
};

pub mod ceiling;
pub mod log;
pub mod verify;

pub use ceiling::spend_verdict;
pub use log::{HubRequestLogEntry, LogKind, LogOutcome};
pub use verify::verify_signature;

/// The desktop's own word on what the Hub may do here. Written only from
/// Settings; no request kind carries a field that could change it (docs/HUB.md
/// §7). The default is everything off and both ceilings unset, which refuses
/// every spend.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubLocalPolicy {
    pub spend_enabled: bool,
    pub gates_enabled: bool,
    pub run_ceiling_cents: Option<u64>,
    pub daily_ceiling_cents: Option<u64>,
}

/// Mirrors `required_scope` in `crates/demeteo-hub/src/socket/requests.rs`,
/// which this crate must not depend on. `None` for the two passkey kinds:
/// they are authorised by the assertion they carry, and an instance with every
/// scope off must still be able to have a passkey revoked.
///
/// No wildcard arm, so a kind the protocol gains does not compile here until
/// someone has said which scope it spends.
pub fn required_scope(payload: &RequestPayload) -> Option<Scope> {
    match payload {
        RequestPayload::StartFeature(_)
        | RequestPayload::CancelFeature(_)
        | RequestPayload::SetStepAssignment(_) => Some(Scope::Spend),
        RequestPayload::GateDecision(_) => Some(Scope::Gates),
        RequestPayload::PasskeyEndorsement(_) | RequestPayload::PasskeyRevocation(_) => None,
    }
}

/// The scopes in force: those the user approved at pairing, narrowed by the
/// local toggles. Pairing records consent and the toggle is the later word,
/// so a toggle cannot widen past what was pinned. `read` has no toggle.
pub fn effective_scopes(pinned: &[Scope], policy: &HubLocalPolicy) -> Vec<Scope> {
    pinned
        .iter()
        .copied()
        .filter(|scope| match scope {
            Scope::Read => true,
            Scope::Spend => policy.spend_enabled,
            Scope::Gates => policy.gates_enabled,
        })
        .collect()
}

/// The result of checking the Hub's signature over `signing_bytes()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureCheck {
    Valid,
    Invalid,
}

/// What recording the request id found: the caller's insert either claimed it
/// or hit an id already recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consumption {
    Fresh,
    AlreadyConsumed,
}

/// Why a request gets no answer at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// `RefusalReason` has no variant for this on purpose: an answer to a
    /// request that does not verify would be an answer to whoever forged it,
    /// and the Hub keeps the first outcome it records (docs/HUB.md §6.1).
    BadSignature,
}

/// The kinds that reach an effect on this path. Gate decisions and passkey
/// changes are verified against pinned passkeys elsewhere and are deferred
/// here. Each borrows the payload [`decide`] was given, so the caller
/// dispatches on the verdict alone and has no arm for a kind it cannot run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExecuteKind<'a> {
    StartFeature(&'a StartFeature),
    CancelFeature(&'a CancelFeature),
    SetStepAssignment(&'a SetStepAssignment),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict<'a> {
    Execute(ExecuteKind<'a>),
    Refuse(RefusalReason),
    Drop(DropReason),
    /// Not consumed and not answered, so a redelivery reaches the handler
    /// that owns the kind instead of reading as a replay.
    Defer,
}

/// What [`precheck`] concluded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Precheck {
    /// Record the id, then ask [`decide`].
    Consume,
    /// Settled without spending the id. Never an `Execute`, so it borrows
    /// nothing.
    Settled(Verdict<'static>),
}

/// Signature, scope, instance and expiry, in the order docs/HUB.md §6.1
/// fixes. The signature comes first because every later check answers, and
/// an answer to a request that does not verify is the forger's (see
/// [`DropReason::BadSignature`]). A request is expired at
/// `now_secs >= expires_at`, the bound the Hub's `delivery_verdict` applies
/// before it sends, checked here against this instance's clock.
///
/// `scopes` is [`effective_scopes`], not the pinned set.
pub fn precheck(
    request: &SignedRequest,
    signature: SignatureCheck,
    pinned_instance_id: &str,
    now_secs: u64,
    scopes: &[Scope],
) -> Precheck {
    if signature == SignatureCheck::Invalid {
        return Precheck::Settled(Verdict::Drop(DropReason::BadSignature));
    }
    let scope = required_scope(&request.payload);
    if scope.is_some_and(|s| !scopes.contains(&s)) {
        return Precheck::Settled(Verdict::Refuse(RefusalReason::ScopeOff));
    }
    if request.instance_id != pinned_instance_id {
        return Precheck::Settled(Verdict::Refuse(RefusalReason::WrongInstance));
    }
    if now_secs >= request.expires_at {
        return Precheck::Settled(Verdict::Refuse(RefusalReason::Expired));
    }
    match &request.payload {
        RequestPayload::StartFeature(_)
        | RequestPayload::CancelFeature(_)
        | RequestPayload::SetStepAssignment(_) => Precheck::Consume,
        RequestPayload::GateDecision(_)
        | RequestPayload::PasskeyEndorsement(_)
        | RequestPayload::PasskeyRevocation(_) => Precheck::Settled(Verdict::Defer),
    }
}

/// Replay, then the kind-specific spend check, for a request [`precheck`]
/// said to consume.
///
/// `committed_cents` is the 24-hour committed total of Hub launches and is
/// read only for a start. `None` for a start refuses: a total the caller did
/// not load is not zero.
pub fn decide<'a>(
    payload: &'a RequestPayload,
    consumption: Consumption,
    committed_cents: Option<u64>,
    policy: &HubLocalPolicy,
) -> Verdict<'a> {
    if consumption == Consumption::AlreadyConsumed {
        return Verdict::Refuse(RefusalReason::Replayed);
    }
    match payload {
        RequestPayload::StartFeature(start) => {
            let Some(committed) = committed_cents else {
                return Verdict::Refuse(RefusalReason::OverRollingCeiling);
            };
            match spend_verdict(
                start.max_budget_cents,
                policy.run_ceiling_cents,
                committed,
                policy.daily_ceiling_cents,
            ) {
                Ok(()) => Verdict::Execute(ExecuteKind::StartFeature(start)),
                Err(reason) => Verdict::Refuse(reason),
            }
        }
        RequestPayload::CancelFeature(cancel) => {
            Verdict::Execute(ExecuteKind::CancelFeature(cancel))
        }
        RequestPayload::SetStepAssignment(change) => {
            Verdict::Execute(ExecuteKind::SetStepAssignment(change))
        }
        RequestPayload::GateDecision(_)
        | RequestPayload::PasskeyEndorsement(_)
        | RequestPayload::PasskeyRevocation(_) => Verdict::Defer,
    }
}

/// The longest `request_id` a body may carry and still be logged and
/// answered, in bytes, whether or not it parses as a [`SignedRequest`]. It is
/// checked before the signature, so nothing yet vouches for the id, and a
/// longer one is treated as a frame with no readable header; the Hub's own
/// ids are 22 base64url chars.
pub const MAX_REQUEST_ID_LEN: usize = 64;

/// The refusal for a body that is not a [`SignedRequest`] this build can
/// read, from the `payload.kind` it carried, if any: a kind this build knows
/// whose body did not parse is `MalformedBody`, anything else `UnknownKind`.
/// Nothing about such a body was verified, so this is the only thing said
/// about it (docs/HUB.md §6.1).
pub fn unparsed_refusal(kind: Option<&str>) -> RefusalReason {
    match kind.and_then(LogKind::parse) {
        None | Some(LogKind::Unknown) => RefusalReason::UnknownKind,
        Some(_) => RefusalReason::MalformedBody,
    }
}

pub fn effort_level(effort: Effort) -> EffortLevel {
    match effort {
        Effort::Low => EffortLevel::Low,
        Effort::Medium => EffortLevel::Medium,
        Effort::High => EffortLevel::High,
        Effort::Xhigh => EffortLevel::XHigh,
        Effort::Max => EffortLevel::Max,
    }
}

pub fn step_override(wire: &WireStepOverride) -> StepOverride {
    StepOverride {
        step_id: wire.step_id.clone(),
        agent_kind: wire.agent_kind.clone(),
        model: wire.model.clone(),
        effort: wire.effort.map(effort_level),
    }
}

pub fn step_assignment(wire: &SetStepAssignment) -> StepAssignment {
    StepAssignment {
        agent_kind: wire.agent_kind.clone(),
        model: wire.model.clone(),
        effort: wire.effort.map(effort_level),
    }
}

/// The execution a request naming a node by `step_id` addresses. A node
/// that ran more than once has a row per run, and only the newest is the one
/// a reassignment can still reach.
pub fn latest_execution<'a>(
    steps: &'a [StepExecution],
    step_id: &str,
) -> Option<&'a StepExecution> {
    steps
        .iter()
        .filter(|step| step.step_id.as_str() == step_id)
        .max_by_key(|step| (step.created_at, step.step_index))
}

/// The refusals `StepExecutor::step_set_assignment` would return, in its
/// order, as the reason the Hub is told — decided before the call because
/// its error is text, and text is never what a refusal carries.
///
/// An identifier the executor would store unchecked is refused the way the
/// MCP surface refuses it: the Hub's user typed it, and a model is an argv
/// value where a leading `-` reads as a flag.
pub fn reassignment_refusal(
    step: &StepExecution,
    assignment: &StepAssignment,
    registered: &[&str],
) -> Option<RefusalReason> {
    if out_of_band_refusal(RunAction::Assign, step.step_id.as_str()).is_some()
        || kind_refusal(&step.step_kind).is_some()
        || assignment_refusal(&step.status).is_some()
    {
        return Some(RefusalReason::StepNotReassignable);
    }
    if assignment
        .agent_kind
        .as_deref()
        .is_some_and(|kind| harness_refusal(kind, registered).is_some())
    {
        return Some(RefusalReason::HarnessNotRegistered);
    }
    identifier_refusal(assignment).map(|_| RefusalReason::MalformedBody)
}

/// [`reassignment_refusal`]'s harness and identifier refusals, applied to a
/// start's feature-wide pair and to every override, harnesses first. No
/// launch path the app itself offers checks these — the UI picks from the
/// registry and a probe, and MCP leaves both to the project — so without this
/// a Hub start would store whatever its user typed.
pub fn start_refusal(start: &StartFeature, registered: &[&str]) -> Option<RefusalReason> {
    let assignments: Vec<StepAssignment> = std::iter::once((&start.agent_kind, &start.model))
        .chain(
            start
                .step_overrides
                .iter()
                .map(|o| (&o.agent_kind, &o.model)),
        )
        .map(|(agent_kind, model)| StepAssignment {
            agent_kind: agent_kind.clone(),
            model: model.clone(),
            effort: None,
        })
        .collect();
    if assignments.iter().any(|a| {
        a.agent_kind
            .as_deref()
            .is_some_and(|kind| harness_refusal(kind, registered).is_some())
    }) {
        return Some(RefusalReason::HarnessNotRegistered);
    }
    assignments
        .iter()
        .find_map(identifier_refusal)
        .map(|_| RefusalReason::MalformedBody)
}

/// The bounds the bootstrap holds a Feature's attachments to, passed in
/// because they are `application::attachments`' constants and `domain/` does
/// not import `application/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentLimits {
    pub max_count: usize,
    pub max_bytes: u64,
    pub max_total_bytes: u64,
}

/// A start whose declared attachments the bootstrap would refuse, judged
/// before any is fetched: the Hub deletes each one as it hands it over, and
/// the fetcher buffers up to the declared size, so by the bootstrap's own
/// check the memory is spent and the Hub's copies are gone. An empty
/// attachment is refused with them, as the commit refuses it.
pub fn attachment_refusal(
    attachments: &[AttachmentRef],
    limits: AttachmentLimits,
) -> Option<RefusalReason> {
    if attachments.len() > limits.max_count {
        return Some(RefusalReason::AttachmentMismatch);
    }
    let total = attachments.iter().try_fold(0u64, |total, attachment| {
        if attachment.size == 0 || attachment.size > limits.max_bytes {
            return None;
        }
        total.checked_add(attachment.size)
    });
    match total {
        Some(total) if total <= limits.max_total_bytes => None,
        _ => Some(RefusalReason::AttachmentMismatch),
    }
}
