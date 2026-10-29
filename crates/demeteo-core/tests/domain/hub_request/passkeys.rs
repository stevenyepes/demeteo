// The trusted passkey set's challenges and verdicts. `super` =
// `domain::hub_request::passkeys`.
//
// Twins of vectors in `src-tauri/tests/hub_gate_assertion.rs`, kept here for
// the reason `tests/domain/hub_request/assertion.rs` gives.

use super::*;
use crate::domain::ids::{FeatureId, GateDecisionId, StepExecutionId, StepId};
use crate::support::soft_passkey::{
    digest, encode, flip_last, Gate, SoftPasskey, GATE, ORIGIN, RP_ID,
};
use demeteo_hub_protocol::{RequestPayload, PROTOCOL_VERSION};

fn hub_pins() -> HubPins {
    HubPins {
        hub_origin: ORIGIN.into(),
        instance_id: GATE.instance_id.into(),
        request_signing_key: String::new(),
        rp_id: RP_ID.into(),
        scopes: Vec::new(),
        device_jkt: String::new(),
        paired_at: 0,
        passkeys: Vec::new(),
    }
}

/// `payload` in an envelope carrying `envelope`'s instance, request id and
/// expiry.
fn relayed(envelope: Gate, payload: RequestPayload) -> SignedRequest {
    SignedRequest {
        v: PROTOCOL_VERSION,
        request_id: envelope.request_id.into(),
        instance_id: envelope.instance_id.into(),
        issued_at: envelope.expires_at - 600,
        expires_at: envelope.expires_at,
        payload,
        signature: String::new(),
    }
}

fn gate_step(status: &str) -> StepExecution {
    StepExecution {
        id: StepExecutionId::new(GATE.step_execution_id),
        feature_id: FeatureId::new(GATE.feature_id),
        step_id: StepId::new("review"),
        step_index: 0,
        step_kind: "gate".into(),
        status: status.into(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn pending_row() -> GateDecision {
    GateDecision {
        id: GateDecisionId::new("gd-1"),
        step_execution_id: StepExecutionId::new(GATE.step_execution_id),
        decision: None,
        feedback: None,
        created_at: 0,
        auto_approved: false,
    }
}

/// `envelope` relayed with `assertion` to [`GATE`]'s gate, which awaits a
/// decision, reduced to the signer it verified.
fn gate_judged(
    pins: &HubPins,
    envelope: Gate,
    assertion: WebAuthnAssertion,
    trusted: &[TrustedPasskey],
    runner_owned: bool,
) -> Result<String, RefusalReason> {
    let gate = GatePayload {
        feature_id: envelope.feature_id.into(),
        step_execution_id: envelope.step_execution_id.into(),
        decision: envelope.decision,
        feedback: None,
        assertion,
    };
    let request = relayed(envelope, RequestPayload::GateDecision(gate.clone()));
    gate_verdict(
        &request,
        &gate,
        pins,
        trusted,
        Some(&gate_step("awaiting_gate")),
        Some(&pending_row()),
        runner_owned,
    )
    .map(|verified| verified.passkey.credential_id.clone())
}

/// `signer`'s endorsement of `new`'s credential id and key, over their
/// instance-bound challenge.
fn endorsement_of(signer: &SoftPasskey, new: &SoftPasskey) -> PasskeyEndorsement {
    let new = new.trusted(0);
    let challenge = endorsement_challenge_digest(
        GATE.instance_id,
        GATE.request_id,
        GATE.expires_at,
        &new.credential_id,
        &new.public_key,
    )
    .expect("the endorsement challenge encodes");
    PasskeyEndorsement {
        assertion: signer.assert(&challenge),
        new_credential_id: new.credential_id,
        new_public_key: new.public_key,
    }
}

/// `body` relayed for [`GATE`]'s instance, reduced to the signer it verified.
fn endorsed(body: PasskeyEndorsement, set: &[TrustedPasskey]) -> Result<String, TrustRefusal> {
    let request = relayed(GATE, RequestPayload::PasskeyEndorsement(body.clone()));
    endorsement_verdict(&request, &body, &hub_pins(), set)
        .map(|verified| verified.passkey.credential_id.clone())
}

/// `signer`'s revocation of `target`, over its instance-bound challenge.
fn revocation(signer: &SoftPasskey, target: &str) -> PasskeyRevocation {
    let challenge =
        revocation_challenge_digest(GATE.instance_id, GATE.request_id, GATE.expires_at, target)
            .expect("the revocation challenge encodes");
    PasskeyRevocation {
        credential_id: target.into(),
        assertion: signer.assert(&challenge),
    }
}

fn revoked(body: PasskeyRevocation, set: &[TrustedPasskey]) -> Result<String, TrustRefusal> {
    let request = relayed(GATE, RequestPayload::PasskeyRevocation(body.clone()));
    revocation_verdict(&request, &body, &hub_pins(), set)
        .map(|verified| verified.passkey.credential_id.clone())
}

#[test]
fn gate_challenge_digest_matches_the_hub_md_vector() {
    let digest = gate_challenge_digest(
        "i-7f3a9c",
        "f-1791004167577",
        "se-0042",
        Decision::Approve,
        "Zk3m0R8pQnVx2tL5aYw9bA",
        1_791_020_400,
    )
    .expect("the HUB.md example encodes");
    assert_eq!(
        encode(&digest),
        "ss9J2G8tlsYSapyixbEJ-ruL5ykKX4OTGBZeezoMZZY"
    );
}

#[test]
fn gate_verdict_accepts_a_pending_gate_with_a_valid_assertion() {
    let signer = SoftPasskey::es256();
    let set = [SoftPasskey::es256().trusted(0), signer.trusted(0)];
    assert_eq!(
        gate_judged(&hub_pins(), GATE, signer.assert(&digest(GATE)), &set, false),
        Ok(signer.credential_id())
    );
}

#[test]
fn gate_verdict_refuses_a_runner_owned_step_runner_owned() {
    let signer = SoftPasskey::es256();
    let set = [signer.trusted(0)];
    assert_eq!(
        gate_judged(&hub_pins(), GATE, signer.assert(&digest(GATE)), &set, true),
        Err(RefusalReason::RunnerOwned)
    );
}

/// Signed over every field the request carries, its `instance_id` included,
/// so only the pinned instance id tells it apart.
#[test]
fn gate_verdict_recomputes_the_challenge_from_the_pinned_instance_id() {
    let elsewhere = Gate {
        instance_id: "i-0000ff",
        ..GATE
    };
    let signer = SoftPasskey::es256();
    let set = [signer.trusted(0)];
    let assertion = signer.assert(&digest(elsewhere));

    assert_eq!(
        gate_judged(&hub_pins(), elsewhere, assertion.clone(), &set, false),
        Err(RefusalReason::AssertionInvalid)
    );
    let pinned_elsewhere = HubPins {
        instance_id: elsewhere.instance_id.into(),
        ..hub_pins()
    };
    assert_eq!(
        gate_judged(&pinned_elsewhere, elsewhere, assertion, &set, false),
        Ok(signer.credential_id())
    );
}

#[test]
fn endorsement_verdict_accepts_a_key_endorsed_by_another_trusted_key() {
    let (a, b, c) = (
        SoftPasskey::es256(),
        SoftPasskey::es256(),
        SoftPasskey::es256(),
    );
    let set = [b.trusted(0), a.trusted(0)];
    assert_eq!(
        endorsed(endorsement_of(&a, &c), &set),
        Ok(a.credential_id())
    );
}

#[test]
fn endorsement_verdict_refuses_a_key_endorsing_itself() {
    let (a, c) = (SoftPasskey::es256(), SoftPasskey::es256());
    let set = [a.trusted(0)];
    let refused = endorsed(endorsement_of(&c, &c), &set);
    assert_eq!(refused, Err(TrustRefusal::SelfEndorsement));
    assert_eq!(
        refused.map_err(trust_refusal),
        Err(RefusalReason::CredentialNotPinned)
    );
}

#[test]
fn endorsement_verdict_refuses_an_untrusted_signer() {
    let (a, c, d) = (
        SoftPasskey::es256(),
        SoftPasskey::es256(),
        SoftPasskey::es256(),
    );
    let set = [a.trusted(0)];
    let refused = endorsed(endorsement_of(&d, &c), &set);
    assert_eq!(
        refused,
        Err(TrustRefusal::Assertion(
            AssertionFault::CredentialNotTrusted
        ))
    );
    assert_eq!(
        refused.map_err(trust_refusal),
        Err(RefusalReason::CredentialNotPinned)
    );
}

#[test]
fn endorsement_verdict_accepts_an_ed25519_signer() {
    let (a, b, c) = (
        SoftPasskey::ed25519(),
        SoftPasskey::es256(),
        SoftPasskey::es256(),
    );
    let set = [b.trusted(0), a.trusted(0)];
    assert_eq!(
        endorsed(endorsement_of(&a, &c), &set),
        Ok(a.credential_id())
    );
}

/// The endorsed key goes into the set as the store writes it: both strings
/// exactly as the endorsement carried them, and a count of zero.
#[test]
fn an_ed25519_key_endorsed_by_an_es256_signer_decides_a_gate() {
    let (a, c) = (SoftPasskey::es256(), SoftPasskey::ed25519());
    let body = endorsement_of(&a, &c);
    assert_eq!(body.new_public_key, encode(&c.public_key_cose()));
    assert_eq!(
        endorsed(body.clone(), &[a.trusted(0)]),
        Ok(a.credential_id())
    );

    let set = [
        a.trusted(0),
        TrustedPasskey {
            credential_id: body.new_credential_id,
            public_key: body.new_public_key,
            sign_count: 0,
        },
    ];
    assert_eq!(
        gate_judged(&hub_pins(), GATE, c.assert(&digest(GATE)), &set, false),
        Ok(c.credential_id())
    );
}

#[test]
fn endorsement_verdict_refuses_a_flipped_ed25519_signature() {
    let (a, b, c) = (
        SoftPasskey::ed25519(),
        SoftPasskey::es256(),
        SoftPasskey::ed25519(),
    );
    let set = [b.trusted(0), a.trusted(0)];
    let mut body = endorsement_of(&a, &c);
    body.assertion.signature = flip_last(&body.assertion.signature);
    let refused = endorsed(body, &set);
    assert_eq!(
        refused,
        Err(TrustRefusal::Assertion(AssertionFault::BadSignature))
    );
    assert_eq!(
        refused.map_err(trust_refusal),
        Err(RefusalReason::AssertionInvalid)
    );
}

/// `parse_cose_key` refuses anything longer than a canonical ES256 key on its
/// own, so no verdict sees `MAX_COSE_KEY_LEN` raised above it: only this does.
#[test]
fn max_cose_key_len_is_the_canonical_es256_length() {
    let (a, c) = (SoftPasskey::es256(), SoftPasskey::es256());
    assert_eq!(MAX_COSE_KEY_LEN, c.public_key_cose().len());
    assert!(MAX_COSE_KEY_LEN >= SoftPasskey::ed25519().public_key_cose().len());
    assert_eq!(
        endorsed(endorsement_of(&a, &c), &[a.trusted(0)]),
        Ok(a.credential_id())
    );
}

#[test]
fn revocation_verdict_accepts_a_key_revoked_by_another_trusted_key() {
    let (a, b) = (SoftPasskey::es256(), SoftPasskey::es256());
    let set = [a.trusted(0), b.trusted(0)];
    assert_eq!(
        revoked(revocation(&a, &b.credential_id()), &set),
        Ok(a.credential_id())
    );
}

#[test]
fn revocation_verdict_refuses_the_last_trusted_key() {
    let a = SoftPasskey::es256();
    let set = [a.trusted(0)];
    let refused = revoked(revocation(&a, &a.credential_id()), &set);
    assert_eq!(refused, Err(TrustRefusal::LastCredential));
    assert_eq!(
        refused.map_err(trust_refusal),
        Err(RefusalReason::LastCredential)
    );
}
