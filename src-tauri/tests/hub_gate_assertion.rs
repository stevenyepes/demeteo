#![cfg(target_os = "linux")]
//! The desktop's pure checks on a passkey-decided Hub request (docs/HUB.md §8).

use demeteo_core::domain::hub_pairing::HubPins;
use demeteo_core::domain::hub_request::assertion::{
    counter_advances, verify_assertion, verify_assertion_bytes, AssertionBytes, AssertionFault,
    RelyingPartyPins, VerifiedAssertion, FLAG_USER_PRESENT, FLAG_USER_VERIFIED,
};
use demeteo_core::domain::hub_request::cose::{parse_cose_key, PasskeyKey};
use demeteo_core::domain::hub_request::passkeys::{
    assertion_refusal, endorsement_challenge_digest, endorsement_verdict, gate_challenge_digest,
    gate_decision_word, gate_verdict, revocation_challenge_digest, revocation_verdict,
    trust_refusal, trust_write_refusal, TrustRefusal, TrustWrite, TrustedPasskey,
};
use demeteo_core::domain::ids::{FeatureId, GateDecisionId, StepExecutionId, StepId};
use demeteo_core::domain::models::{GateDecision, StepExecution};
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::challenge::{base64url_decode, base64url_encode};
use demeteo_hub::auth::testing::SoftAuthenticator;
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::signing_key::SigningKey;
use demeteo_hub::ports::KeyService;
use demeteo_hub::protocol::{
    Decision, GateDecision as GatePayload, PasskeyEndorsement, PasskeyRevocation, RefusalReason,
    RequestPayload, SignedRequest, WebAuthnAssertion, PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};

const ES256_ALG_OFFSET: usize = 4;
const CRV_OFFSET: usize = 6;

const RP_ID: &str = "hub.example";
const ORIGIN: &str = "https://hub.example";
const UP_AND_UV: u8 = FLAG_USER_PRESENT | FLAG_USER_VERIFIED;

/// The six fields `gate_challenge` binds.
#[derive(Clone, Copy)]
struct Gate {
    instance_id: &'static str,
    feature_id: &'static str,
    step_execution_id: &'static str,
    decision: Decision,
    request_id: &'static str,
    expires_at: u64,
}

/// The gate every assertion below is checked against.
const GATE: Gate = Gate {
    instance_id: "i-7f3a9c",
    feature_id: "f-1791004167577",
    step_execution_id: "se-0042",
    decision: Decision::Approve,
    request_id: "Zk3m0R8pQnVx2tL5aYw9bA",
    expires_at: 1_791_020_400,
};

fn digest(gate: Gate) -> [u8; 32] {
    gate_challenge_digest(
        gate.instance_id,
        gate.feature_id,
        gate.step_execution_id,
        gate.decision,
        gate.request_id,
        gate.expires_at,
    )
    .expect("the gate challenge encodes")
}

fn rp() -> RelyingParty {
    RelyingParty {
        rp_id: RP_ID.to_owned(),
        origin: ORIGIN.to_owned(),
    }
}

fn pins() -> RelyingPartyPins<'static> {
    RelyingPartyPins {
        rp_id: RP_ID,
        origin: ORIGIN,
    }
}

fn trusted(authenticator: &SoftAuthenticator, sign_count: u32) -> TrustedPasskey {
    TrustedPasskey {
        credential_id: authenticator.credential_id(),
        public_key: base64url_encode(&authenticator.public_key_cose()),
        sign_count,
    }
}

/// `assertion` checked against [`GATE`], reduced to the counter it reported.
fn checked(
    assertion: &WebAuthnAssertion,
    trusted: &[TrustedPasskey],
) -> Result<u32, AssertionFault> {
    verify_assertion(assertion, trusted, pins(), &digest(GATE)).map(|verified| verified.sign_count)
}

/// [`checked`] through `verify_assertion_bytes`, the fields decoded first.
fn checked_decoded(
    assertion: &WebAuthnAssertion,
    trusted: &[TrustedPasskey],
) -> Result<u32, AssertionFault> {
    let credential_id = decoded(&assertion.credential_id);
    let authenticator_data = decoded(&assertion.authenticator_data);
    let client_data_json = decoded(&assertion.client_data_json);
    let signature = decoded(&assertion.signature);
    let bytes = AssertionBytes {
        credential_id: &credential_id,
        authenticator_data: &authenticator_data,
        client_data_json: &client_data_json,
        signature: &signature,
    };
    verify_assertion_bytes(bytes, trusted, pins(), &digest(GATE))
        .map(|verified| verified.sign_count)
}

/// An assertion by a trusted key over `gate`, checked against [`GATE`].
fn checked_over(gate: Gate) -> Result<u32, AssertionFault> {
    let authenticator = SoftAuthenticator::new();
    checked(
        &authenticator.assert(&rp(), &digest(gate)),
        &[trusted(&authenticator, 0)],
    )
}

fn decoded(text: &str) -> Vec<u8> {
    base64url_decode(text).expect("unpadded base64url")
}

/// The authenticator data and client data of a valid assertion over [`GATE`],
/// to be altered and re-signed.
fn signed_parts(authenticator: &SoftAuthenticator) -> (Vec<u8>, Vec<u8>) {
    let get = authenticator.assert(&rp(), &digest(GATE));
    (
        decoded(&get.authenticator_data),
        decoded(&get.client_data_json),
    )
}

fn client_data(kind: &str) -> Vec<u8> {
    serde_json::json!({
        "type": kind,
        "challenge": base64url_encode(&digest(GATE)),
        "origin": ORIGIN,
    })
    .to_string()
    .into_bytes()
}

fn flip_last(b64url: &str) -> String {
    let mut bytes = decoded(b64url);
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    base64url_encode(&bytes)
}

fn ed25519_cose(x: &[u8; 32]) -> Vec<u8> {
    [
        &[0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20][..],
        x,
    ]
    .concat()
}

fn replaced(bytes: &[u8], offset: usize, value: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[offset] = value;
    out
}

#[test]
fn cose_accepts_the_soft_authenticators_key() {
    let cose = SoftAuthenticator::new().public_key_cose();
    let Some(PasskeyKey::Es256(sec1)) = parse_cose_key(&cose) else {
        panic!("a canonical ES256 COSE_Key must parse as Es256");
    };
    assert_eq!(sec1[0], 0x04);
    assert_eq!(sec1[1..33], cose[10..42], "x");
    assert_eq!(sec1[33..], cose[45..], "y");
}

#[test]
fn cose_accepts_a_canonical_ed25519_key() {
    let x: [u8; 32] = std::array::from_fn(|i| 0x11 + i as u8);
    let cose = ed25519_cose(&x);
    assert_eq!(cose.len(), 42);
    assert_eq!(parse_cose_key(&cose), Some(PasskeyKey::Ed25519(x)));
}

#[test]
fn cose_rejects_non_canonical_keys() {
    let es256 = SoftAuthenticator::new().public_key_cose();
    assert_eq!(es256.len(), 77);
    let eddsa = ed25519_cose(&[0x5a; 32]);

    // `y` behind a two-byte length header: the same key a CBOR decoder would
    // accept, one byte longer.
    let non_minimal_y = [&es256[..43], &[0x59, 0x00, 0x20], &es256[45..]].concat();

    let cases: [(&str, Vec<u8>); 9] = [
        ("empty", Vec::new()),
        ("76-byte ES256", es256[..76].to_vec()),
        ("78-byte ES256", non_minimal_y),
        (
            "ES256 with alg EdDSA",
            replaced(&es256, ES256_ALG_OFFSET, 0x27),
        ),
        ("ES256 with crv P-384", replaced(&es256, CRV_OFFSET, 0x02)),
        ("ES256 with a trailing byte", [&es256[..], &[0x00]].concat()),
        ("EdDSA with crv X25519", replaced(&eddsa, CRV_OFFSET, 0x04)),
        ("EdDSA with a trailing byte", [&eddsa[..], &[0x00]].concat()),
        ("41-byte EdDSA", eddsa[..41].to_vec()),
    ];
    for (case, bytes) in cases {
        assert_eq!(parse_cose_key(&bytes), None, "{case} must be refused");
    }
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
        base64url_encode(&digest),
        "ss9J2G8tlsYSapyixbEJ-ruL5ykKX4OTGBZeezoMZZY"
    );
}

#[test]
fn gate_decision_word_pins_both_spellings() {
    assert_eq!(gate_decision_word(Decision::Approve), "approve");
    assert_eq!(gate_decision_word(Decision::Cancel), "cancel");
}

#[test]
fn accepts_a_valid_es256_assertion() {
    let other = SoftAuthenticator::new();
    let authenticator = SoftAuthenticator::new().with_sign_count(7);
    let set = [trusted(&other, 0), trusted(&authenticator, 0)];
    let assertion = authenticator.assert(&rp(), &digest(GATE));

    assert_eq!(
        verify_assertion(&assertion, &set, pins(), &digest(GATE)),
        Ok(VerifiedAssertion {
            passkey: &set[1],
            sign_count: 7,
        })
    );
}

#[test]
fn rejects_a_challenge_over_another_instance_id() {
    let gate = Gate {
        instance_id: "i-0000ff",
        ..GATE
    };
    assert_eq!(checked_over(gate), Err(AssertionFault::WrongChallenge));
}

#[test]
fn rejects_a_challenge_over_another_feature_id() {
    let gate = Gate {
        feature_id: "f-1791004167578",
        ..GATE
    };
    assert_eq!(checked_over(gate), Err(AssertionFault::WrongChallenge));
}

#[test]
fn rejects_a_challenge_over_another_step_execution_id() {
    let gate = Gate {
        step_execution_id: "se-0043",
        ..GATE
    };
    assert_eq!(checked_over(gate), Err(AssertionFault::WrongChallenge));
}

#[test]
fn rejects_a_challenge_over_another_decision() {
    let gate = Gate {
        decision: Decision::Cancel,
        ..GATE
    };
    assert_eq!(checked_over(gate), Err(AssertionFault::WrongChallenge));
}

#[test]
fn rejects_a_challenge_over_another_request_id() {
    let gate = Gate {
        request_id: "Zk3m0R8pQnVx2tL5aYw9bB",
        ..GATE
    };
    assert_eq!(checked_over(gate), Err(AssertionFault::WrongChallenge));
}

#[test]
fn rejects_a_challenge_over_another_expires_at() {
    let gate = Gate {
        expires_at: GATE.expires_at + 1,
        ..GATE
    };
    assert_eq!(checked_over(gate), Err(AssertionFault::WrongChallenge));
}

#[test]
fn rejects_wrong_origin() {
    let authenticator = SoftAuthenticator::new();
    let set = [trusted(&authenticator, 0)];
    for origin in [
        "https://hub.example.evil.example",
        "https://hub.example/",
        "https://HUB.example",
        "http://hub.example",
        "https://hub.example:443",
    ] {
        let assertion = authenticator.assert_with(RP_ID, origin, UP_AND_UV, &digest(GATE));
        assert_eq!(
            checked(&assertion, &set),
            Err(AssertionFault::WrongOrigin),
            "{origin}"
        );
    }
}

#[test]
fn rejects_wrong_rp_id_hash() {
    let authenticator = SoftAuthenticator::new();
    let assertion = authenticator.assert_with("other.example", ORIGIN, UP_AND_UV, &digest(GATE));
    assert_eq!(
        checked(&assertion, &[trusted(&authenticator, 0)]),
        Err(AssertionFault::WrongRpId)
    );
}

#[test]
fn rejects_uv_unset() {
    let authenticator = SoftAuthenticator::new();
    let assertion = authenticator.assert_with(RP_ID, ORIGIN, FLAG_USER_PRESENT, &digest(GATE));
    assert_eq!(
        checked(&assertion, &[trusted(&authenticator, 0)]),
        Err(AssertionFault::UserNotVerified)
    );
}

#[test]
fn rejects_up_unset() {
    let authenticator = SoftAuthenticator::new();
    let assertion = authenticator.assert_with(RP_ID, ORIGIN, FLAG_USER_VERIFIED, &digest(GATE));
    assert_eq!(
        checked(&assertion, &[trusted(&authenticator, 0)]),
        Err(AssertionFault::UserNotPresent)
    );
}

/// A registration's client data replayed as an assertion, signed by the
/// trusted key, so the type is the only thing wrong with it.
#[test]
fn rejects_wrong_type() {
    let authenticator = SoftAuthenticator::new();
    let (authenticator_data, _) = signed_parts(&authenticator);
    let assertion = authenticator.sign(&authenticator_data, &client_data("webauthn.create"));
    assert_eq!(
        checked(&assertion, &[trusted(&authenticator, 0)]),
        Err(AssertionFault::WrongType)
    );
}

#[test]
fn rejects_an_untrusted_credential() {
    let authenticator = SoftAuthenticator::new();
    let assertion = authenticator.assert(&rp(), &digest(GATE));
    assert_eq!(
        checked(&assertion, &[]),
        Err(AssertionFault::CredentialNotTrusted)
    );
    assert_eq!(
        checked(&assertion, &[trusted(&SoftAuthenticator::new(), 0)]),
        Err(AssertionFault::CredentialNotTrusted)
    );
}

/// The authenticator's own credential id, trusted under the 76-byte ES256 key
/// of `cose_rejects_non_canonical_keys`.
#[test]
fn rejects_a_trusted_key_that_is_not_canonical_cose() {
    let authenticator = SoftAuthenticator::new();
    let set = [TrustedPasskey {
        public_key: base64url_encode(&authenticator.public_key_cose()[..76]),
        ..trusted(&authenticator, 0)
    }];
    assert_eq!(
        checked(&authenticator.assert(&rp(), &digest(GATE)), &set),
        Err(AssertionFault::UnsupportedKey)
    );
    assert_eq!(
        assertion_refusal(AssertionFault::UnsupportedKey),
        RefusalReason::AssertionInvalid
    );
}

#[test]
fn rejects_a_signature_by_another_key() {
    let trusted_key = SoftAuthenticator::new();
    let impostor = SoftAuthenticator::new();
    let mut assertion = impostor.assert(&rp(), &digest(GATE));
    assertion.credential_id = trusted_key.credential_id();
    assert_eq!(
        checked(&assertion, &[trusted(&trusted_key, 0)]),
        Err(AssertionFault::BadSignature)
    );
}

#[test]
fn rejects_a_flipped_signature_byte() {
    let authenticator = SoftAuthenticator::new();
    let mut assertion = authenticator.assert(&rp(), &digest(GATE));
    assertion.signature = flip_last(&assertion.signature);
    assert_eq!(
        checked(&assertion, &[trusted(&authenticator, 0)]),
        Err(AssertionFault::BadSignature)
    );
}

/// Client data the trusted key did not sign is refused for its signature
/// before it is parsed: not JSON at all here, it would otherwise be
/// `Malformed`.
#[test]
fn checks_the_signature_before_reading_client_data() {
    let authenticator = SoftAuthenticator::new();
    let mut assertion = authenticator.assert(&rp(), &digest(GATE));
    assertion.client_data_json = base64url_encode(b"webauthn.get");
    assert_eq!(
        checked(&assertion, &[trusted(&authenticator, 0)]),
        Err(AssertionFault::BadSignature)
    );
}

/// Re-signed, so the parser does run: `Malformed`, where the unsigned copy in
/// `checks_the_signature_before_reading_client_data` is `BadSignature`.
#[test]
fn rejects_re_signed_client_data_that_is_not_json() {
    let authenticator = SoftAuthenticator::new();
    let (authenticator_data, _) = signed_parts(&authenticator);
    let assertion = authenticator.sign(&authenticator_data, b"webauthn.get");
    assert_eq!(
        checked(&assertion, &[trusted(&authenticator, 0)]),
        Err(AssertionFault::Malformed)
    );
}

/// 36 bytes, re-signed under a stored count of 0, which a verifier reading
/// the missing counter as 0 would accept. The 37-byte side of the floor is
/// `accepts_a_valid_es256_assertion`.
#[test]
fn rejects_authenticator_data_shorter_than_37_bytes() {
    let authenticator = SoftAuthenticator::new();
    let (authenticator_data, client_data_json) = signed_parts(&authenticator);
    assert_eq!(authenticator_data.len(), 37);
    let assertion = authenticator.sign(&authenticator_data[..36], &client_data_json);
    assert_eq!(
        checked(&assertion, &[trusted(&authenticator, 0)]),
        Err(AssertionFault::Malformed)
    );
}

#[test]
fn counter_rule_table() {
    let regressed = Err(AssertionFault::CounterRegressed);
    let rows = [
        (0, 0, Ok(0)),
        (0, 3, Ok(3)),
        (5, 6, Ok(6)),
        (5, 5, regressed),
        (5, 4, regressed),
        (5, 0, regressed),
    ];
    for (stored, reported, expected) in rows {
        let authenticator = SoftAuthenticator::new().with_sign_count(reported);
        let assertion = authenticator.assert(&rp(), &digest(GATE));
        assert_eq!(
            checked(&assertion, &[trusted(&authenticator, stored)]),
            expected,
            "stored {stored}, reported {reported}"
        );
        assert_eq!(
            counter_advances(stored, reported),
            expected.is_ok(),
            "stored {stored}, reported {reported}"
        );
    }
}

/// An Ed25519 passkey under a hand-built OKP `COSE_Key`, signing what an
/// authenticator signs with the Hub's own Ed25519 signer.
struct Ed25519Passkey {
    keys: MemoryKeys,
    signer: SigningKey,
}

impl Ed25519Passkey {
    const NAME: &str = "passkey";
    const CREDENTIAL_ID: [u8; 16] = [0x42; 16];

    fn new() -> Self {
        Self {
            keys: MemoryKeys::with_keys([Self::NAME]),
            signer: SigningKey::new(Self::NAME),
        }
    }

    async fn trusted(&self) -> TrustedPasskey {
        let public_key = self
            .keys
            .verifying_key(Self::NAME)
            .await
            .expect("read the public key");
        TrustedPasskey {
            credential_id: base64url_encode(&Self::CREDENTIAL_ID),
            public_key: base64url_encode(&ed25519_cose(&public_key)),
            sign_count: 0,
        }
    }

    async fn sign(&self, authenticator_data: &[u8], client_data_json: &[u8]) -> Vec<u8> {
        let signed = [authenticator_data, &Sha256::digest(client_data_json)].concat();
        self.signer
            .sign(&self.keys, &signed)
            .await
            .expect("sign the assertion")
            .to_vec()
    }
}

fn ed25519_authenticator_data(sign_count: u32) -> Vec<u8> {
    [
        &Sha256::digest(RP_ID.as_bytes())[..],
        &[UP_AND_UV],
        &sign_count.to_be_bytes(),
    ]
    .concat()
}

#[tokio::test]
async fn accepts_an_ed25519_assertion() {
    let passkey = Ed25519Passkey::new();
    let set = [passkey.trusted().await];
    let authenticator_data = ed25519_authenticator_data(9);
    let client_data_json = client_data("webauthn.get");
    let signature = passkey.sign(&authenticator_data, &client_data_json).await;

    let assertion = AssertionBytes {
        credential_id: &Ed25519Passkey::CREDENTIAL_ID,
        authenticator_data: &authenticator_data,
        client_data_json: &client_data_json,
        signature: &signature,
    };
    assert_eq!(
        verify_assertion_bytes(assertion, &set, pins(), &digest(GATE)),
        Ok(VerifiedAssertion {
            passkey: &set[0],
            sign_count: 9,
        })
    );
}

#[tokio::test]
async fn rejects_a_flipped_ed25519_signature() {
    let passkey = Ed25519Passkey::new();
    let set = [passkey.trusted().await];
    let authenticator_data = ed25519_authenticator_data(9);
    let client_data_json = client_data("webauthn.get");
    let mut signature = passkey.sign(&authenticator_data, &client_data_json).await;
    signature[0] ^= 0x01;

    let assertion = AssertionBytes {
        credential_id: &Ed25519Passkey::CREDENTIAL_ID,
        authenticator_data: &authenticator_data,
        client_data_json: &client_data_json,
        signature: &signature,
    };
    assert_eq!(
        verify_assertion_bytes(assertion, &set, pins(), &digest(GATE)),
        Err(AssertionFault::BadSignature)
    );
}

/// Each field at its bound is checked further, and one byte over it is
/// `Malformed` even where every later check would pass: re-signed, and under
/// a credential id the trusted set holds. `verify_assertion` measures the text
/// before decoding it and `verify_assertion_bytes` the bytes it is handed, so
/// each case goes through both.
#[test]
fn oversized_fields_are_malformed_before_decode() {
    const MAX_CREDENTIAL_ID: usize = 1023;
    const MAX_AUTHENTICATOR_DATA: usize = 1024;
    const MAX_CLIENT_DATA_JSON: usize = 4096;
    const MAX_SIGNATURE: usize = 128;

    let authenticator = SoftAuthenticator::new();
    let (authenticator_data, client_data_json) = signed_parts(&authenticator);
    let padded = |bytes: &[u8], len: usize, fill: u8| {
        let mut out = bytes.to_vec();
        out.resize(len, fill);
        out
    };
    let both = |case: &str,
                assertion: &WebAuthnAssertion,
                set: &[TrustedPasskey],
                expected: Result<u32, AssertionFault>| {
        assert_eq!(checked(assertion, set), expected, "{case}, as text");
        assert_eq!(
            checked_decoded(assertion, set),
            expected,
            "{case}, as bytes"
        );
    };

    for (len, expected) in [
        (MAX_CREDENTIAL_ID, Ok(0)),
        (MAX_CREDENTIAL_ID + 1, Err(AssertionFault::Malformed)),
    ] {
        let id = base64url_encode(&vec![0x5a; len]);
        let mut assertion = authenticator.sign(&authenticator_data, &client_data_json);
        assertion.credential_id = id.clone();
        let set = [TrustedPasskey {
            credential_id: id,
            ..trusted(&authenticator, 0)
        }];
        both(
            &format!("credential id of {len}"),
            &assertion,
            &set,
            expected,
        );
    }

    let set = [trusted(&authenticator, 0)];
    for (len, expected) in [
        (MAX_AUTHENTICATOR_DATA, Ok(0)),
        (MAX_AUTHENTICATOR_DATA + 1, Err(AssertionFault::Malformed)),
    ] {
        let long = padded(&authenticator_data, len, 0x00);
        let assertion = authenticator.sign(&long, &client_data_json);
        both(
            &format!("authenticator data of {len}"),
            &assertion,
            &set,
            expected,
        );
    }

    for (len, expected) in [
        (MAX_CLIENT_DATA_JSON, Ok(0)),
        (MAX_CLIENT_DATA_JSON + 1, Err(AssertionFault::Malformed)),
    ] {
        let long = padded(&client_data_json, len, b' ');
        let assertion = authenticator.sign(&authenticator_data, &long);
        both(&format!("client data of {len}"), &assertion, &set, expected);
    }

    for (len, expected) in [
        (MAX_SIGNATURE, Err(AssertionFault::BadSignature)),
        (MAX_SIGNATURE + 1, Err(AssertionFault::Malformed)),
    ] {
        let mut assertion = authenticator.sign(&authenticator_data, &client_data_json);
        assertion.signature = base64url_encode(&padded(&decoded(&assertion.signature), len, 0x00));
        both(&format!("signature of {len}"), &assertion, &set, expected);
    }
}

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

/// A relayed decision whose envelope and payload carry `envelope`'s fields.
fn delivery(envelope: Gate, assertion: WebAuthnAssertion) -> (SignedRequest, GatePayload) {
    let gate = GatePayload {
        feature_id: envelope.feature_id.into(),
        step_execution_id: envelope.step_execution_id.into(),
        decision: envelope.decision,
        feedback: None,
        assertion,
    };
    let request = relayed(envelope, RequestPayload::GateDecision(gate.clone()));
    (request, gate)
}

fn gate_step(feature_id: &str, status: &str) -> StepExecution {
    StepExecution {
        id: StepExecutionId::new(GATE.step_execution_id),
        feature_id: FeatureId::new(feature_id),
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

fn gate_row(decision: Option<&str>) -> GateDecision {
    GateDecision {
        id: GateDecisionId::new("gd-1"),
        step_execution_id: StepExecutionId::new(GATE.step_execution_id),
        decision: decision.map(Into::into),
        feedback: None,
        created_at: 0,
        auto_approved: false,
    }
}

/// `envelope` relayed with `assertion` to a gate of [`GATE`]'s feature that
/// awaits a decision.
fn pending_verdict<'t>(
    pins: &HubPins,
    envelope: Gate,
    assertion: WebAuthnAssertion,
    trusted: &'t [TrustedPasskey],
) -> Result<VerifiedAssertion<'t>, RefusalReason> {
    let (request, gate) = delivery(envelope, assertion);
    gate_verdict(
        &request,
        &gate,
        pins,
        trusted,
        Some(&gate_step(GATE.feature_id, "awaiting_gate")),
        Some(&gate_row(None)),
        false,
    )
}

/// A valid assertion over [`GATE`], judged against `step` and `recorded`.
fn verdict_at(
    step: Option<&StepExecution>,
    recorded: Option<&GateDecision>,
) -> Result<(), RefusalReason> {
    let authenticator = SoftAuthenticator::new();
    let set = [trusted(&authenticator, 0)];
    let (request, gate) = delivery(GATE, authenticator.assert(&rp(), &digest(GATE)));
    gate_verdict(&request, &gate, &hub_pins(), &set, step, recorded, false).map(|_| ())
}

#[test]
fn gate_verdict_accepts_a_pending_gate_with_a_valid_assertion() {
    let other = SoftAuthenticator::new();
    let signer = SoftAuthenticator::new().with_sign_count(3);
    let set = [trusted(&other, 0), trusted(&signer, 0)];
    let assertion = signer.assert(&rp(), &digest(GATE));

    let verified = pending_verdict(&hub_pins(), GATE, assertion, &set)
        .expect("a pending gate with a valid assertion is accepted");
    assert_eq!(verified.passkey.credential_id, signer.credential_id());
    assert_eq!(verified.sign_count, 3);
}

#[test]
fn gate_verdict_refuses_a_missing_step_unknown_step() {
    assert_eq!(
        verdict_at(None, Some(&gate_row(None))),
        Err(RefusalReason::UnknownStep)
    );
}

#[test]
fn gate_verdict_refuses_a_step_of_another_feature_unknown_step() {
    let step = gate_step("f-1791004167578", "awaiting_gate");
    assert_eq!(
        verdict_at(Some(&step), Some(&gate_row(None))),
        Err(RefusalReason::UnknownStep)
    );
}

#[test]
fn gate_verdict_refuses_a_decided_gate_gate_not_pending() {
    let step = gate_step(GATE.feature_id, "completed");
    assert_eq!(
        verdict_at(Some(&step), Some(&gate_row(Some("approve")))),
        Err(RefusalReason::GateNotPending)
    );
}

#[test]
fn gate_verdict_refuses_a_step_with_no_gate_row_gate_not_pending() {
    let step = gate_step(GATE.feature_id, "awaiting_gate");
    assert_eq!(
        verdict_at(Some(&step), None),
        Err(RefusalReason::GateNotPending)
    );
}

/// A run a `demeteo-runner` owns is mirrored here without its gate row, so
/// this would read `gate_not_pending` if the pending check came first, and an
/// assertion nobody trusts would read `credential_not_pinned`.
#[test]
fn gate_verdict_refuses_a_runner_owned_step_runner_owned() {
    let authenticator = SoftAuthenticator::new();
    let set = [trusted(&authenticator, 0)];
    let step = gate_step(GATE.feature_id, "awaiting_gate");
    let owned_by_a_runner = |assertion| {
        let (request, gate) = delivery(GATE, assertion);
        gate_verdict(&request, &gate, &hub_pins(), &set, Some(&step), None, true).map(|_| ())
    };

    assert_eq!(
        owned_by_a_runner(authenticator.assert(&rp(), &digest(GATE))),
        Err(RefusalReason::RunnerOwned)
    );
    assert_eq!(
        owned_by_a_runner(SoftAuthenticator::new().assert(&rp(), &digest(GATE))),
        Err(RefusalReason::RunnerOwned)
    );
}

#[test]
fn gate_verdict_refuses_an_untrusted_credential_not_pinned() {
    let stranger = SoftAuthenticator::new();
    let set = [trusted(&SoftAuthenticator::new(), 0)];
    let assertion = stranger.assert(&rp(), &digest(GATE));
    assert_eq!(
        pending_verdict(&hub_pins(), GATE, assertion, &set),
        Err(RefusalReason::CredentialNotPinned)
    );
}

/// The pinned `hub_origin` is the origin, not `https://` followed by the RP
/// ID: here the two differ, and only the pinned one verifies.
#[test]
fn gate_verdict_refuses_another_origin_assertion_invalid() {
    let authenticator = SoftAuthenticator::new();
    let set = [trusted(&authenticator, 0)];
    let pins = HubPins {
        hub_origin: "https://app.hub.example".into(),
        ..hub_pins()
    };
    let made_at = |origin| authenticator.assert_with(RP_ID, origin, UP_AND_UV, &digest(GATE));

    assert!(pending_verdict(&pins, GATE, made_at("https://app.hub.example"), &set).is_ok());
    assert_eq!(
        pending_verdict(&pins, GATE, made_at(ORIGIN), &set),
        Err(RefusalReason::AssertionInvalid)
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
    let authenticator = SoftAuthenticator::new();
    let set = [trusted(&authenticator, 0)];
    let assertion = authenticator.assert(&rp(), &digest(elsewhere));

    assert_eq!(
        pending_verdict(&hub_pins(), elsewhere, assertion.clone(), &set),
        Err(RefusalReason::AssertionInvalid)
    );
    let pinned_elsewhere = HubPins {
        instance_id: elsewhere.instance_id.into(),
        ..hub_pins()
    };
    assert!(pending_verdict(&pinned_elsewhere, elsewhere, assertion, &set).is_ok());
}

/// Keys A, B and C, each with its own credential id, and D, which no set
/// below trusts.
struct Keys {
    a: SoftAuthenticator,
    b: SoftAuthenticator,
    c: SoftAuthenticator,
    d: SoftAuthenticator,
}

impl Keys {
    fn new() -> Self {
        Self {
            a: SoftAuthenticator::new(),
            b: SoftAuthenticator::new(),
            c: SoftAuthenticator::new(),
            d: SoftAuthenticator::new(),
        }
    }
}

fn endorsement_digest(
    instance_id: &str,
    new_credential_id: &str,
    new_public_key: &str,
) -> [u8; 32] {
    endorsement_challenge_digest(
        instance_id,
        GATE.request_id,
        GATE.expires_at,
        new_credential_id,
        new_public_key,
    )
    .expect("the endorsement challenge encodes")
}

fn revocation_digest(instance_id: &str, credential_id: &str) -> [u8; 32] {
    revocation_challenge_digest(instance_id, GATE.request_id, GATE.expires_at, credential_id)
        .expect("the revocation challenge encodes")
}

/// `signer`'s endorsement of `new_credential_id` and `new_public_key`, over
/// their own instance-bound challenge.
fn endorsement(
    signer: &SoftAuthenticator,
    new_credential_id: &str,
    new_public_key: &str,
) -> PasskeyEndorsement {
    let challenge = endorsement_digest(GATE.instance_id, new_credential_id, new_public_key);
    PasskeyEndorsement {
        new_credential_id: new_credential_id.into(),
        new_public_key: new_public_key.into(),
        assertion: signer.assert(&rp(), &challenge),
    }
}

/// `signer`'s endorsement of `new`'s own credential id and key.
fn endorsement_of(signer: &SoftAuthenticator, new: &SoftAuthenticator) -> PasskeyEndorsement {
    let new = trusted(new, 0);
    endorsement(signer, &new.credential_id, &new.public_key)
}

/// `body` relayed for [`GATE`]'s instance, reduced to the signer it verified.
fn endorsed(body: PasskeyEndorsement, set: &[TrustedPasskey]) -> Result<String, TrustRefusal> {
    let request = relayed(GATE, RequestPayload::PasskeyEndorsement(body.clone()));
    endorsement_verdict(&request, &body, &hub_pins(), set)
        .map(|verified| verified.passkey.credential_id.clone())
}

/// `signer`'s revocation of `target`, over its instance-bound challenge.
fn revocation(signer: &SoftAuthenticator, target: &str) -> PasskeyRevocation {
    PasskeyRevocation {
        credential_id: target.into(),
        assertion: signer.assert(&rp(), &revocation_digest(GATE.instance_id, target)),
    }
}

/// `body` relayed for [`GATE`]'s instance, reduced to the signer it verified.
fn revoked(body: PasskeyRevocation, set: &[TrustedPasskey]) -> Result<String, TrustRefusal> {
    let request = relayed(GATE, RequestPayload::PasskeyRevocation(body.clone()));
    revocation_verdict(&request, &body, &hub_pins(), set)
        .map(|verified| verified.passkey.credential_id.clone())
}

#[test]
fn endorsement_verdict_accepts_a_key_endorsed_by_another_trusted_key() {
    let keys = Keys::new();
    let set = [trusted(&keys.b, 0), trusted(&keys.a, 0)];
    assert_eq!(
        endorsed(endorsement_of(&keys.a, &keys.c), &set),
        Ok(keys.a.credential_id())
    );
}

#[test]
fn endorsement_verdict_refuses_a_key_endorsing_itself() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0)];
    let refused = endorsed(endorsement_of(&keys.c, &keys.c), &set);
    assert_eq!(refused, Err(TrustRefusal::SelfEndorsement));
    assert_eq!(
        refused.map_err(trust_refusal),
        Err(RefusalReason::CredentialNotPinned)
    );
}

#[test]
fn endorsement_verdict_refuses_an_untrusted_signer() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0)];
    let refused = endorsed(endorsement_of(&keys.d, &keys.c), &set);
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

/// The challenge commits to the key it endorses: A signed for D's key, and
/// the body names C's.
#[test]
fn endorsement_verdict_refuses_a_challenge_over_another_new_public_key() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0)];
    let c = trusted(&keys.c, 0);
    let signed_for_d = endorsement(&keys.a, &c.credential_id, &trusted(&keys.d, 0).public_key);
    let body = PasskeyEndorsement {
        new_public_key: c.public_key,
        ..signed_for_d
    };
    assert_eq!(
        endorsed(body, &set),
        Err(TrustRefusal::Assertion(AssertionFault::WrongChallenge))
    );
}

/// Records bound to every instance, the binding the Hub's own chain uses
/// (docs/HUB.md §11), are signed by a trusted key over every other field the
/// request carries, and still refused.
#[test]
fn trust_verdicts_refuse_a_challenge_bound_to_every_instance() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0), trusted(&keys.b, 0)];
    let c = trusted(&keys.c, 0);
    let wrong_challenge = Err(TrustRefusal::Assertion(AssertionFault::WrongChallenge));

    let unbound = endorsement_digest("*", &c.credential_id, &c.public_key);
    let body = PasskeyEndorsement {
        assertion: keys.a.assert(&rp(), &unbound),
        ..endorsement_of(&keys.a, &keys.c)
    };
    assert_eq!(endorsed(body, &set), wrong_challenge);

    let target = keys.b.credential_id();
    let body = PasskeyRevocation {
        assertion: keys.a.assert(&rp(), &revocation_digest("*", &target)),
        ..revocation(&keys.a, &target)
    };
    assert_eq!(revoked(body, &set), wrong_challenge);
}

#[test]
fn endorsement_verdict_refuses_a_non_canonical_new_key_before_the_assertion() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0)];
    let cose = keys.c.public_key_cose();
    let encoded = base64url_encode(&cose);
    let cases = [
        ("76-byte ES256", base64url_encode(&cose[..76])),
        (
            "a trailing byte",
            base64url_encode(&[&cose[..], &[0x00]].concat()),
        ),
        ("padded", format!("{encoded}=")),
        ("a standard-alphabet `+`", format!("+{}", &encoded[1..])),
        ("oversized", "A".repeat(1 << 20)),
    ];
    for (case, new_public_key) in cases {
        let body = endorsement(&keys.a, &keys.c.credential_id(), &new_public_key);
        let refused = endorsed(body, &set);
        assert_eq!(refused, Err(TrustRefusal::UnsupportedNewKey), "{case}");
        assert_eq!(
            refused.map_err(trust_refusal),
            Err(RefusalReason::MalformedBody),
            "{case}"
        );
    }
}

/// Refused whoever signs: A, whose id it is, and D, whom nothing trusts.
#[test]
fn endorsement_verdict_refuses_an_id_already_pinned_under_another_key() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0)];
    let c_key = trusted(&keys.c, 0).public_key;
    for (signer, name) in [(&keys.a, "A"), (&keys.d, "D")] {
        let refused = endorsed(endorsement(signer, &keys.a.credential_id(), &c_key), &set);
        assert_eq!(
            refused,
            Err(TrustRefusal::AlreadyPinned),
            "signed by {name}"
        );
        assert_eq!(
            refused.map_err(trust_refusal),
            Err(RefusalReason::MalformedBody),
            "signed by {name}"
        );
    }
}

#[test]
fn endorsement_verdict_refuses_an_invalid_new_credential_id() {
    const MAX_CREDENTIAL_ID: usize = 1023;
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0)];
    let c_key = trusted(&keys.c, 0).public_key;
    let id = base64url_encode(&[0x5a; 16]);

    let at_bound = base64url_encode(&vec![0x5a; MAX_CREDENTIAL_ID]);
    assert_eq!(
        endorsed(endorsement(&keys.a, &at_bound, &c_key), &set),
        Ok(keys.a.credential_id()),
        "an id of {MAX_CREDENTIAL_ID} bytes"
    );

    let cases = [
        ("empty", String::new()),
        (
            "over the bound",
            base64url_encode(&vec![0x5a; MAX_CREDENTIAL_ID + 1]),
        ),
        ("padded", format!("{id}==")),
        ("not base64url", "not base64url!".to_owned()),
    ];
    for (case, new_credential_id) in cases {
        let refused = endorsed(endorsement(&keys.a, &new_credential_id, &c_key), &set);
        assert_eq!(refused, Err(TrustRefusal::Malformed), "{case}");
        assert_eq!(
            refused.map_err(trust_refusal),
            Err(RefusalReason::MalformedBody),
            "{case}"
        );
    }
}

#[test]
fn revocation_verdict_refuses_the_last_trusted_key() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0)];
    let refused = revoked(revocation(&keys.a, &keys.a.credential_id()), &set);
    assert_eq!(refused, Err(TrustRefusal::LastCredential));
    assert_eq!(
        refused.map_err(trust_refusal),
        Err(RefusalReason::LastCredential)
    );
}

#[test]
fn revocation_verdict_refuses_a_key_revoking_itself() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0), trusted(&keys.b, 0)];
    let refused = revoked(revocation(&keys.b, &keys.b.credential_id()), &set);
    assert_eq!(refused, Err(TrustRefusal::SelfRevocation));
    assert_eq!(
        refused.map_err(trust_refusal),
        Err(RefusalReason::SelfRevocation)
    );
}

#[test]
fn revocation_verdict_accepts_a_key_revoked_by_another_trusted_key() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0), trusted(&keys.b, 0)];
    assert_eq!(
        revoked(revocation(&keys.a, &keys.b.credential_id()), &set),
        Ok(keys.a.credential_id())
    );
}

#[test]
fn revocation_verdict_refuses_an_unknown_target() {
    let keys = Keys::new();
    let set = [trusted(&keys.a, 0), trusted(&keys.b, 0)];
    let refused = revoked(revocation(&keys.a, &keys.c.credential_id()), &set);
    assert_eq!(refused, Err(TrustRefusal::TargetNotPinned));
    assert_eq!(
        refused.map_err(trust_refusal),
        Err(RefusalReason::CredentialNotPinned)
    );
}

#[test]
fn trust_refusal_table() {
    let rows = [
        (
            TrustRefusal::Assertion(AssertionFault::CredentialNotTrusted),
            RefusalReason::CredentialNotPinned,
        ),
        (
            TrustRefusal::Assertion(AssertionFault::BadSignature),
            RefusalReason::AssertionInvalid,
        ),
        (
            TrustRefusal::UnsupportedNewKey,
            RefusalReason::MalformedBody,
        ),
        (TrustRefusal::Malformed, RefusalReason::MalformedBody),
        (TrustRefusal::AlreadyPinned, RefusalReason::MalformedBody),
        (
            TrustRefusal::SelfEndorsement,
            RefusalReason::CredentialNotPinned,
        ),
        (
            TrustRefusal::TargetNotPinned,
            RefusalReason::CredentialNotPinned,
        ),
        (TrustRefusal::LastCredential, RefusalReason::LastCredential),
        (TrustRefusal::SelfRevocation, RefusalReason::SelfRevocation),
    ];
    for (refusal, reason) in rows {
        assert_eq!(trust_refusal(refusal), reason, "{refusal:?}");
    }
}

#[test]
fn trust_write_refusal_table() {
    let rows = [
        (TrustWrite::Applied, None),
        (
            TrustWrite::AlreadyPinned,
            Some(RefusalReason::MalformedBody),
        ),
        (
            TrustWrite::SignerChanged,
            Some(RefusalReason::CredentialNotPinned),
        ),
        (
            TrustWrite::TargetGone,
            Some(RefusalReason::CredentialNotPinned),
        ),
        (
            TrustWrite::LastCredential,
            Some(RefusalReason::LastCredential),
        ),
    ];
    for (write, reason) in rows {
        assert_eq!(trust_write_refusal(write), reason, "{write:?}");
    }
}
