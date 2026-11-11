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
    AttachmentRef, CancelFeature, Effort, GateDecision as GatePayload, PasskeyRevocation,
    RefusalReason, RequestPayload, RetryStep, Scope, SetStepAssignment, SignedRequest,
    StartFeature, StepOverride as WireStepOverride,
};
use serde::{Deserialize, Serialize};

use crate::domain::models::{EffortLevel, StepExecution, StepOverride};
use crate::domain::run_control::{out_of_band_refusal, retry_refusal, RunAction};
use crate::domain::step_assignment::{
    assignment_refusal, harness_refusal, identifier_refusal, kind_refusal, StepAssignment,
};
use crate::error::AppError;

pub mod assertion;
pub mod ceiling;
pub mod cose;
pub mod log;
pub mod passkeys;
pub mod runner_spec;
pub mod verify;

pub use ceiling::spend_verdict;
pub use log::{HubRequestLogEntry, HubRequestLogRow, LogKind, LogOutcome};
pub use runner_spec::{runner_run_spec, runner_start_refusal};
pub use verify::verify_signature;

/// The desktop's own word on what the Hub may do here. Written only from
/// Settings; no request kind carries a field that could change it (docs/HUB.md
/// §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubLocalPolicy {
    #[serde(default = "read_enabled_by_default")]
    pub read_enabled: bool,
    pub spend_enabled: bool,
    pub gates_enabled: bool,
    pub run_ceiling_cents: Option<u64>,
    pub daily_ceiling_cents: Option<u64>,
}

fn read_enabled_by_default() -> bool {
    true
}

/// `read` on, `spend` and `gates` off, both ceilings unset, which refuses
/// every spend. `read` is on because pairing already approved it and, until
/// V67 gave it a toggle, it was always in force: a desktop with no saved
/// policy, or one saved before the toggle existed, keeps relaying as before.
impl Default for HubLocalPolicy {
    fn default() -> Self {
        Self {
            read_enabled: read_enabled_by_default(),
            spend_enabled: false,
            gates_enabled: false,
            run_ceiling_cents: None,
            daily_ceiling_cents: None,
        }
    }
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
        | RequestPayload::SetStepAssignment(_)
        | RequestPayload::RetryStep(_) => Some(Scope::Spend),
        RequestPayload::GateDecision(_) => Some(Scope::Gates),
        RequestPayload::PasskeyEndorsement(_) | RequestPayload::PasskeyRevocation(_) => None,
    }
}

/// The scopes in force: those the user approved at pairing, narrowed by the
/// local toggles. Pairing records consent and the toggle is the later word,
/// so a toggle cannot widen past what was pinned. Every scope has a toggle,
/// `read` included.
pub fn effective_scopes(pinned: &[Scope], policy: &HubLocalPolicy) -> Vec<Scope> {
    pinned
        .iter()
        .copied()
        .filter(|scope| match scope {
            Scope::Read => policy.read_enabled,
            Scope::Spend => policy.spend_enabled,
            Scope::Gates => policy.gates_enabled,
        })
        .collect()
}

/// Whether saving `new` over `old` changes what a running socket sends,
/// which it reads only when it starts: `read` decides whether events and
/// snapshots are relayed, and `spend` and `gates` decide which scopes its
/// `Hello` announces (`hub_socket::announced_scopes`). The ceilings are read
/// per request, so changing one restarts nothing. `None` is a previous policy
/// that could not be read: it may have said anything, so it counts as a
/// change.
pub fn socket_policy_changed(old: Option<&HubLocalPolicy>, new: &HubLocalPolicy) -> bool {
    old.is_none_or(|old| {
        old.read_enabled != new.read_enabled
            || old.spend_enabled != new.spend_enabled
            || old.gates_enabled != new.gates_enabled
    })
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

/// The kinds that reach an effect on this path: every kind but an
/// endorsement, which [`decide`] refuses. A gate decision or a revocation
/// reaches one only if its passkey assertion verifies, which
/// [`passkeys::gate_verdict`] and [`passkeys::revocation_verdict`] decide
/// after the id is spent. Each borrows the payload [`decide`] was given, so
/// the caller dispatches on the verdict alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExecuteKind<'a> {
    StartFeature(&'a StartFeature),
    CancelFeature(&'a CancelFeature),
    SetStepAssignment(&'a SetStepAssignment),
    RetryStep(&'a RetryStep),
    GateDecision(&'a GatePayload),
    PasskeyRevocation(&'a PasskeyRevocation),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict<'a> {
    Execute(ExecuteKind<'a>),
    Refuse(RefusalReason),
    Drop(DropReason),
    /// Not consumed and not answered, so a redelivery is judged afresh
    /// instead of reading as a replay. Never a kind's verdict: neither
    /// [`precheck`] nor [`decide`] returns it. The caller does, for a request
    /// whose judging could not read what it needs from storage.
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
/// `scopes` is [`effective_scopes`], not the pinned set. A gate decision
/// with `gates` off is refused here, before its assertion is looked at.
/// Every kind that passes these is consumed; nothing here defers.
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
    Precheck::Consume
}

/// Whether this instance launches the runs a Hub start asks for. A desktop
/// does. A `demeteo-runner` does not: a run there is submitted by the
/// desktop that owns it, and a Hub start has no desktop to own it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartSupport {
    Launches,
    Refused,
}

/// Whether judging `payload` reads the 24-hour committed total. Only a start
/// does, and not one this instance refuses outright: [`decide`] refuses that
/// one before any ceiling, so a total that fails to load must not defer it.
pub fn needs_committed_total(payload: &RequestPayload, starts: StartSupport) -> bool {
    matches!(payload, RequestPayload::StartFeature(_)) && starts == StartSupport::Launches
}

/// Replay, then the kind-specific spend check, for a request [`precheck`]
/// said to consume.
///
/// `committed_cents` is the 24-hour committed total of Hub launches and is
/// read only for a start. `None` for a start refuses: a total the caller did
/// not load is not zero. A start reaching an instance whose `starts` is
/// [`StartSupport::Refused`] is `UnknownKind`, before any ceiling: the id is
/// spent, so its redelivery reads `Replayed`, and nothing it names is read.
pub fn decide<'a>(
    payload: &'a RequestPayload,
    consumption: Consumption,
    committed_cents: Option<u64>,
    policy: &HubLocalPolicy,
    starts: StartSupport,
) -> Verdict<'a> {
    if consumption == Consumption::AlreadyConsumed {
        return Verdict::Refuse(RefusalReason::Replayed);
    }
    match payload {
        RequestPayload::StartFeature(_) if starts == StartSupport::Refused => {
            Verdict::Refuse(RefusalReason::UnknownKind)
        }
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
        // No ceiling: the ceilings bound the caps Hub starts commit, and a
        // retry commits no new cap. It re-runs the step under the run's own
        // `max_budget_usd`, which the start was already judged on (HUB.md §7).
        RequestPayload::RetryStep(retry) => Verdict::Execute(ExecuteKind::RetryStep(retry)),
        RequestPayload::GateDecision(gate) => Verdict::Execute(ExecuteKind::GateDecision(gate)),
        // Refused, assertion unread, until a Hub producer and a desktop-side
        // confirmation or trusted-set view exist (docs/HUB.md §11 item 20).
        RequestPayload::PasskeyEndorsement(_) => Verdict::Refuse(RefusalReason::UnknownKind),
        RequestPayload::PasskeyRevocation(revocation) => {
            Verdict::Execute(ExecuteKind::PasskeyRevocation(revocation))
        }
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

/// The refusals `StepExecutor::step_retry` would return before it touches a
/// row, as the reason the Hub is told — decided before the call for
/// [`reassignment_refusal`]'s reason. What the executor checks after these,
/// that no upstream step is still working, needs the workflow graph, and is
/// answered from the call's error.
pub fn hub_retry_refusal(step: &StepExecution) -> Option<RefusalReason> {
    (retry_refusal(&step.status).is_some()
        || out_of_band_refusal(RunAction::Retry, step.step_id.as_str()).is_some())
    .then_some(RefusalReason::StepNotRetryable)
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

/// A start a desktop refuses before its lookups: one carrying a
/// [`RunnerStart`](demeteo_hub_protocol::RunnerStart) was resolved for a
/// runner, and a desktop launching it would run what its own database holds
/// under the same ids, not what the Hub signed. It is `MalformedBody` because
/// a Hub that sends one has addressed the wrong kind of instance.
pub fn desktop_start_refusal(start: &StartFeature) -> Option<RefusalReason> {
    start
        .runner
        .is_some()
        .then_some(RefusalReason::MalformedBody)
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

/// The reason a cancel of a Feature this instance holds is refused when its
/// call fails.
///
/// `NotFound` is the instance holding the Feature but no run of it to stop, as
/// a runner answers for a Feature row with no `runner_runs` row under it. The
/// Hub names a run only by its Feature, so that is the Feature's run being
/// unknown here, the reason a desktop gives for a Feature it does not hold. Every other failure is the instance's own,
/// and `RefusalReason` has no generic failure (docs/HUB.md §6.1).
pub fn cancel_refusal(error: &AppError) -> RefusalReason {
    match error {
        AppError::NotFound { .. } => RefusalReason::UnknownFeature,
        _ => RefusalReason::MalformedBody,
    }
}

/// The reason a retry that passed [`hub_retry_refusal`] is refused when its
/// call fails.
///
/// `Validation` is the executor declining the step after all: an upstream
/// step still working, or a status that moved between the read and the call.
/// `NotFound` is [`cancel_refusal`]'s case, a runner holding the Feature but
/// no run of it. Every other failure is the instance's own.
pub fn retry_call_refusal(error: &AppError) -> RefusalReason {
    match error {
        AppError::Validation { .. } => RefusalReason::StepNotRetryable,
        _ => cancel_refusal(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> RequestPayload {
        RequestPayload::StartFeature(StartFeature {
            project_id: "p-1".into(),
            workflow_id: "w-1".into(),
            title: "t".into(),
            description: "d".into(),
            agent_kind: None,
            model: None,
            effort: None,
            step_overrides: Vec::new(),
            max_budget_cents: Some(1),
            max_wall_clock_secs: None,
            attachments: Vec::new(),
            runner: None,
        })
    }

    /// The total is loaded only for a start this instance would launch, so a
    /// runner never defers a start it is going to refuse because a total it
    /// does not need failed to load.
    #[test]
    fn only_a_launched_start_needs_the_committed_total() {
        let cancel = RequestPayload::CancelFeature(CancelFeature {
            feature_id: "f-1".into(),
        });
        let rows = [
            (start(), StartSupport::Launches, true),
            (start(), StartSupport::Refused, false),
            (cancel.clone(), StartSupport::Launches, false),
            (cancel, StartSupport::Refused, false),
        ];
        for (payload, starts, expected) in rows {
            assert_eq!(
                needs_committed_total(&payload, starts),
                expected,
                "{payload:?} with {starts:?}"
            );
        }
    }

    /// Refused without the total: `None` would otherwise read as an
    /// unloaded total and refuse `OverRollingCeiling`.
    #[test]
    fn a_refused_start_is_unknown_kind_without_a_total() {
        assert_eq!(
            decide(
                &start(),
                Consumption::Fresh,
                None,
                &HubLocalPolicy::default(),
                StartSupport::Refused
            ),
            Verdict::Refuse(RefusalReason::UnknownKind)
        );
    }

    #[test]
    fn a_cancel_with_no_run_to_stop_is_an_unknown_feature() {
        assert_eq!(
            cancel_refusal(&AppError::not_found("no run of feature f-1")),
            RefusalReason::UnknownFeature
        );
        for failed in [
            AppError::internal("run vanished during cancel"),
            AppError::database("database query failed"),
            AppError::transport("ssh error: -7"),
        ] {
            assert_eq!(
                cancel_refusal(&failed),
                RefusalReason::MalformedBody,
                "{failed}"
            );
        }
    }

    /// A retry commits no cap, so no total is loaded for it and a policy with
    /// both ceilings unset, which refuses every start, still lets it through.
    #[test]
    fn a_retry_reads_no_total_and_meets_no_ceiling() {
        let retry = RequestPayload::RetryStep(RetryStep {
            feature_id: "f-1".into(),
            step_id: "s-implement".into(),
        });
        assert!(!needs_committed_total(&retry, StartSupport::Launches));
        assert!(matches!(
            decide(
                &retry,
                Consumption::Fresh,
                None,
                &HubLocalPolicy::default(),
                StartSupport::Refused
            ),
            Verdict::Execute(ExecuteKind::RetryStep(_))
        ));
    }

    #[test]
    fn a_retry_the_executor_declines_is_not_retryable_and_any_other_failure_is_the_cancels() {
        assert_eq!(
            retry_call_refusal(&AppError::validation(
                "Step 's-plan' is still running; wait for it to finish before retrying this step."
            )),
            RefusalReason::StepNotRetryable
        );
        assert_eq!(
            retry_call_refusal(&AppError::not_found("no run of feature f-1")),
            RefusalReason::UnknownFeature
        );
        assert_eq!(
            retry_call_refusal(&AppError::internal("run vanished during retry")),
            RefusalReason::MalformedBody
        );
    }
}
