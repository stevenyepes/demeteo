//! The desktop's trusted passkey set and the challenges its members sign
//! (docs/HUB.md §8).
//!
//! Each digest is the SHA-256 of the pre-image its namesake in
//! `crates/demeteo-hub-protocol/src/canonical.rs` builds, over the same
//! parameters. A pre-image that cannot be encoded is returned as the error,
//! and a caller refuses on it.

use demeteo_hub_protocol::{
    endorsement_challenge, gate_challenge, revocation_challenge, Decision, EncodeError,
    GateDecision as GatePayload, PasskeyEndorsement, PasskeyRevocation, RefusalReason,
    SignedRequest, WebAuthnAssertion,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::assertion::{
    bounded_decode, verify_assertion, AssertionFault, RelyingPartyPins, VerifiedAssertion,
    MAX_CREDENTIAL_ID_LEN,
};
use super::cose::parse_cose_key;
use crate::domain::hub_pairing::HubPins;
use crate::domain::models::{GateDecision, StepExecution};
use crate::domain::run_control::gate_decision_refusal;

/// One trusted passkey. Both strings are unpadded base64url, as
/// `hub_pinned_passkeys` stores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedPasskey {
    pub credential_id: String,
    pub public_key: String,
    pub sign_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PasskeyOrigin {
    Pinned,
    Endorsed,
}

/// Derived rather than stored: `save` stamps every pairing passkey with the
/// pairing's own `paired_at`, so any other `pinned_at` is a later endorsement.
/// Both are whole seconds, so an endorsement inside the pairing's second would
/// read as pinned; an explicit column is the fix if endorsements ship
/// (docs/HUB.md §11 item 20 refuses them today).
pub fn passkey_origin(pinned_at: i64, paired_at: i64) -> PasskeyOrigin {
    if pinned_at == paired_at {
        PasskeyOrigin::Pinned
    } else {
        PasskeyOrigin::Endorsed
    }
}

/// A trusted passkey as Settings lists it. No public key: nothing a reader
/// shows needs it, so it never crosses IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HubTrustedPasskey {
    pub credential_id: String,
    pub sign_count: u32,
    pub pinned_at: i64,
    pub origin: PasskeyOrigin,
}

fn sha256(pre_image: Vec<u8>) -> [u8; 32] {
    Sha256::digest(pre_image).into()
}

pub fn gate_challenge_digest(
    instance_id: &str,
    feature_id: &str,
    step_execution_id: &str,
    decision: Decision,
    request_id: &str,
    expires_at: u64,
) -> Result<[u8; 32], EncodeError> {
    gate_challenge(
        instance_id,
        feature_id,
        step_execution_id,
        decision,
        request_id,
        expires_at,
    )
    .map(sha256)
}

pub fn endorsement_challenge_digest(
    instance_id: &str,
    request_id: &str,
    expires_at: u64,
    new_credential_id: &str,
    new_public_key: &str,
) -> Result<[u8; 32], EncodeError> {
    endorsement_challenge(
        instance_id,
        request_id,
        expires_at,
        new_credential_id,
        new_public_key,
    )
    .map(sha256)
}

pub fn revocation_challenge_digest(
    instance_id: &str,
    request_id: &str,
    expires_at: u64,
    credential_id: &str,
) -> Result<[u8; 32], EncodeError> {
    revocation_challenge(instance_id, request_id, expires_at, credential_id).map(sha256)
}

/// The decision as `GatePresenter::gate_decide` takes it. `classify` in
/// `domain/gate/decision.rs` cancels the run on any word it does not know, so
/// a misspelt `approve` would cancel rather than fail.
pub fn gate_decision_word(decision: Decision) -> &'static str {
    match decision {
        Decision::Approve => "approve",
        Decision::Cancel => "cancel",
    }
}

/// The refusal the Hub is told for a fault. Only an unknown credential has a
/// reason of its own; every other fault is `assertion_invalid`. No wildcard
/// arm, so a fault the verifier gains does not compile here until someone has
/// classified it.
pub fn assertion_refusal(fault: AssertionFault) -> RefusalReason {
    match fault {
        AssertionFault::CredentialNotTrusted => RefusalReason::CredentialNotPinned,
        AssertionFault::Malformed
        | AssertionFault::UnsupportedKey
        | AssertionFault::BadSignature
        | AssertionFault::WrongType
        | AssertionFault::WrongChallenge
        | AssertionFault::WrongOrigin
        | AssertionFault::WrongRpId
        | AssertionFault::UserNotPresent
        | AssertionFault::UserNotVerified
        | AssertionFault::CounterRegressed => RefusalReason::AssertionInvalid,
    }
}

/// Whether `gate` may be applied: the step, then whether a `demeteo-runner`
/// owns its run, then whether its gate still awaits a decision, then the
/// assertion. `step` and `recorded` are what the caller loaded for
/// `gate.step_execution_id`, and `runner_owned` whether the remote-run mirror
/// holds `gate.feature_id`.
///
/// A runner-owned run is a mirror with no local gate row, so it is refused
/// before the pending check, which would call its gate decided, and before the
/// assertion, which would spend a counter on a gate this desktop cannot decide.
///
/// The challenge is recomputed from the envelope and this desktop's own
/// `pins.instance_id`, never taken from the request, so an assertion made for
/// another instance does not verify here even when the envelope names that
/// instance.
pub fn gate_verdict<'t>(
    request: &SignedRequest,
    gate: &GatePayload,
    pins: &HubPins,
    trusted: &'t [TrustedPasskey],
    step: Option<&StepExecution>,
    recorded: Option<&GateDecision>,
    runner_owned: bool,
) -> Result<VerifiedAssertion<'t>, RefusalReason> {
    let step = step
        .filter(|step| step.feature_id.as_str() == gate.feature_id)
        .ok_or(RefusalReason::UnknownStep)?;
    if runner_owned {
        return Err(RefusalReason::RunnerOwned);
    }
    if gate_decision_refusal(&step.status, recorded).is_some() {
        return Err(RefusalReason::GateNotPending);
    }
    let challenge = gate_challenge_digest(
        &pins.instance_id,
        &gate.feature_id,
        &gate.step_execution_id,
        gate.decision,
        &request.request_id,
        request.expires_at,
    )
    .map_err(|_| RefusalReason::MalformedBody)?;
    verify_assertion(
        &gate.assertion,
        trusted,
        RelyingPartyPins {
            rp_id: &pins.rp_id,
            origin: &pins.hub_origin,
        },
        &challenge,
    )
    .map_err(assertion_refusal)
}

/// The longer of the two encodings [`parse_cose_key`] accepts, ES256's.
const MAX_COSE_KEY_LEN: usize = 77;

/// Why an endorsement or a revocation leaves the trusted set as it is. Every
/// variant but `Assertion` is decided before the assertion is checked, and
/// [`trust_refusal`] is what the Hub is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TrustRefusal {
    #[error(transparent)]
    Assertion(AssertionFault),
    #[error("the new public key is not a canonical ES256 or Ed25519 COSE_Key")]
    UnsupportedNewKey,
    #[error("the new credential id is not unpadded base64url of 1 to 1023 bytes, or the challenge cannot be encoded")]
    Malformed,
    #[error("the new credential is already trusted")]
    AlreadyPinned,
    #[error("the new credential signed its own endorsement")]
    SelfEndorsement,
    #[error("the credential to revoke is not trusted")]
    TargetNotPinned,
    #[error("the credential to revoke is the last one trusted")]
    LastCredential,
    #[error("the credential to revoke signed its own revocation")]
    SelfRevocation,
}

/// What the storage port found when it tried to commit an endorsement or a
/// revocation together with the signer's counter. Anything but `Applied`
/// means a row changed after the verdict read the set, and nothing was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustWrite {
    Applied,
    /// The signer's row is gone, or its count is no longer the one verified.
    SignerChanged,
    /// The new credential id was pinned meanwhile.
    AlreadyPinned,
    /// The credential to revoke was revoked meanwhile.
    TargetGone,
    /// The credential to revoke became the last one.
    LastCredential,
}

/// What the Hub is told. A self-endorsement is `credential_not_pinned`, as an
/// untrusted signer is: the key being endorsed is not yet in the set.
pub fn trust_refusal(refusal: TrustRefusal) -> RefusalReason {
    match refusal {
        TrustRefusal::Assertion(fault) => assertion_refusal(fault),
        TrustRefusal::UnsupportedNewKey | TrustRefusal::Malformed | TrustRefusal::AlreadyPinned => {
            RefusalReason::MalformedBody
        }
        TrustRefusal::SelfEndorsement | TrustRefusal::TargetNotPinned => {
            RefusalReason::CredentialNotPinned
        }
        TrustRefusal::LastCredential => RefusalReason::LastCredential,
        TrustRefusal::SelfRevocation => RefusalReason::SelfRevocation,
    }
}

/// `None` for a write that committed, else what the Hub is told.
pub fn trust_write_refusal(write: TrustWrite) -> Option<RefusalReason> {
    match write {
        TrustWrite::Applied => None,
        TrustWrite::AlreadyPinned => Some(RefusalReason::MalformedBody),
        TrustWrite::SignerChanged | TrustWrite::TargetGone => {
            Some(RefusalReason::CredentialNotPinned)
        }
        TrustWrite::LastCredential => Some(RefusalReason::LastCredential),
    }
}

/// Whether a trusted passkey may add `endorsement`'s key to the set: the new
/// key, then the new id, then a signer other than the new key, then the
/// assertion (docs/HUB.md §8).
///
/// As in [`gate_verdict`], the challenge is recomputed from this desktop's
/// `pins.instance_id`, so only an instance-bound record verifies: one the
/// Hub bound to `"*"` (docs/HUB.md §11) is over another challenge.
///
/// No request path reaches this while [`super::decide`] refuses the kind
/// (docs/HUB.md §11 item 20).
pub fn endorsement_verdict<'t>(
    request: &SignedRequest,
    endorsement: &PasskeyEndorsement,
    pins: &HubPins,
    trusted: &'t [TrustedPasskey],
) -> Result<VerifiedAssertion<'t>, TrustRefusal> {
    bounded_decode(&endorsement.new_public_key, MAX_COSE_KEY_LEN)
        .ok()
        .and_then(|cose| parse_cose_key(&cose))
        .ok_or(TrustRefusal::UnsupportedNewKey)?;
    bounded_decode(&endorsement.new_credential_id, MAX_CREDENTIAL_ID_LEN)
        .ok()
        .filter(|id| !id.is_empty())
        .ok_or(TrustRefusal::Malformed)?;
    // Strict unpadded base64url has one spelling per byte string, so these
    // compare as text exactly as they would decoded, and as the store's
    // primary key does.
    if is_trusted(trusted, &endorsement.new_credential_id) {
        return Err(TrustRefusal::AlreadyPinned);
    }
    if endorsement.assertion.credential_id == endorsement.new_credential_id {
        return Err(TrustRefusal::SelfEndorsement);
    }
    let challenge = endorsement_challenge_digest(
        &pins.instance_id,
        &request.request_id,
        request.expires_at,
        &endorsement.new_credential_id,
        &endorsement.new_public_key,
    )
    .map_err(|_| TrustRefusal::Malformed)?;
    verify_trusted(&endorsement.assertion, pins, trusted, &challenge)
}

/// Whether a trusted passkey may remove `revocation`'s credential: the target
/// is trusted, it is not the last, and its own key did not sign, all before
/// the signer is looked up, then the assertion (docs/HUB.md §8). Bound to
/// `pins.instance_id` as [`endorsement_verdict`] is.
pub fn revocation_verdict<'t>(
    request: &SignedRequest,
    revocation: &PasskeyRevocation,
    pins: &HubPins,
    trusted: &'t [TrustedPasskey],
) -> Result<VerifiedAssertion<'t>, TrustRefusal> {
    if !is_trusted(trusted, &revocation.credential_id) {
        return Err(TrustRefusal::TargetNotPinned);
    }
    if trusted.len() <= 1 {
        return Err(TrustRefusal::LastCredential);
    }
    if revocation.assertion.credential_id == revocation.credential_id {
        return Err(TrustRefusal::SelfRevocation);
    }
    let challenge = revocation_challenge_digest(
        &pins.instance_id,
        &request.request_id,
        request.expires_at,
        &revocation.credential_id,
    )
    .map_err(|_| TrustRefusal::Malformed)?;
    verify_trusted(&revocation.assertion, pins, trusted, &challenge)
}

fn is_trusted(trusted: &[TrustedPasskey], credential_id: &str) -> bool {
    trusted
        .iter()
        .any(|passkey| passkey.credential_id == credential_id)
}

fn verify_trusted<'t>(
    assertion: &WebAuthnAssertion,
    pins: &HubPins,
    trusted: &'t [TrustedPasskey],
    challenge: &[u8; 32],
) -> Result<VerifiedAssertion<'t>, TrustRefusal> {
    verify_assertion(
        assertion,
        trusted,
        RelyingPartyPins {
            rp_id: &pins.rp_id,
            origin: &pins.hub_origin,
        },
        challenge,
    )
    .map_err(TrustRefusal::Assertion)
}

#[cfg(test)]
#[path = "../../../tests/domain/hub_request/passkeys.rs"]
mod passkeys_tests;
