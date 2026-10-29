// The WebAuthn assertion verifier's own vocabulary. `super` =
// `domain::hub_request::assertion`.
//
// `src-tauri/tests/hub_gate_assertion.rs` holds the twin of each vector here,
// signed by the Hub's `SoftAuthenticator`, and runs on Linux only. Keep both:
// these are the ones `cross-os-test` runs on macOS and Windows.

use super::*;
use crate::domain::hub_request::passkeys::assertion_refusal;
use crate::support::soft_passkey::{
    authenticator_data, client_data, decode, digest, ed25519_cose, encode, es256_cose, flip_last,
    Gate, SoftPasskey, GATE, ORIGIN, RP_ID, UP_AND_UV,
};
use demeteo_hub_protocol::{Decision, RefusalReason};

const ES256_ALG_OFFSET: usize = 4;
const CRV_OFFSET: usize = 6;

fn pins() -> RelyingPartyPins<'static> {
    RelyingPartyPins {
        rp_id: RP_ID,
        origin: ORIGIN,
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
    let credential_id = decode(&assertion.credential_id);
    let authenticator_data = decode(&assertion.authenticator_data);
    let client_data_json = decode(&assertion.client_data_json);
    let signature = decode(&assertion.signature);
    let bytes = AssertionBytes {
        credential_id: &credential_id,
        authenticator_data: &authenticator_data,
        client_data_json: &client_data_json,
        signature: &signature,
    };
    verify_assertion_bytes(bytes, trusted, pins(), &digest(GATE))
        .map(|verified| verified.sign_count)
}

fn replaced(bytes: &[u8], offset: usize, value: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[offset] = value;
    out
}

#[test]
fn cose_reads_the_point_of_a_hand_built_es256_key() {
    let cose = SoftPasskey::es256().public_key_cose();
    assert_eq!(cose.len(), 77);
    let Some(PasskeyKey::Es256(sec1)) = parse_cose_key(&cose) else {
        panic!("a canonical ES256 COSE_Key must parse as Es256");
    };
    assert_eq!(es256_cose(&sec1), cose);
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
    let es256 = SoftPasskey::es256().public_key_cose();
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
fn accepts_a_valid_es256_assertion() {
    let other = SoftPasskey::es256();
    let passkey = SoftPasskey::es256().with_sign_count(7);
    let set = [other.trusted(0), passkey.trusted(0)];

    assert_eq!(
        verify_assertion(&passkey.assert(&digest(GATE)), &set, pins(), &digest(GATE)),
        Ok(VerifiedAssertion {
            passkey: &set[1],
            sign_count: 7,
        })
    );
}

#[test]
fn accepts_a_valid_ed25519_assertion() {
    let other = SoftPasskey::es256();
    let passkey = SoftPasskey::ed25519().with_sign_count(9);
    let set = [other.trusted(0), passkey.trusted(0)];

    assert_eq!(
        verify_assertion(&passkey.assert(&digest(GATE)), &set, pins(), &digest(GATE)),
        Ok(VerifiedAssertion {
            passkey: &set[1],
            sign_count: 9,
        })
    );
}

#[test]
fn rejects_a_challenge_over_another_value_of_each_gate_field() {
    let cases = [
        (
            "instance_id",
            Gate {
                instance_id: "i-0000ff",
                ..GATE
            },
        ),
        (
            "feature_id",
            Gate {
                feature_id: "f-1791004167578",
                ..GATE
            },
        ),
        (
            "step_execution_id",
            Gate {
                step_execution_id: "se-0043",
                ..GATE
            },
        ),
        (
            "decision",
            Gate {
                decision: Decision::Cancel,
                ..GATE
            },
        ),
        (
            "request_id",
            Gate {
                request_id: "Zk3m0R8pQnVx2tL5aYw9bB",
                ..GATE
            },
        ),
        (
            "expires_at",
            Gate {
                expires_at: GATE.expires_at + 1,
                ..GATE
            },
        ),
    ];
    let passkey = SoftPasskey::es256();
    let set = [passkey.trusted(0)];
    for (field, gate) in cases {
        assert_eq!(
            checked(&passkey.assert(&digest(gate)), &set),
            Err(AssertionFault::WrongChallenge),
            "another {field}"
        );
    }
}

#[test]
fn rejects_wrong_origin() {
    let passkey = SoftPasskey::es256();
    let set = [passkey.trusted(0)];
    for origin in [
        "https://hub.example.evil.example",
        "https://hub.example/",
        "https://HUB.example",
        "http://hub.example",
        "https://hub.example:443",
    ] {
        let assertion = passkey.assert_with(RP_ID, origin, UP_AND_UV, &digest(GATE));
        assert_eq!(
            checked(&assertion, &set),
            Err(AssertionFault::WrongOrigin),
            "{origin}"
        );
    }
}

#[test]
fn rejects_wrong_rp_id_hash() {
    let passkey = SoftPasskey::es256();
    let assertion = passkey.assert_with("other.example", ORIGIN, UP_AND_UV, &digest(GATE));
    assert_eq!(
        checked(&assertion, &[passkey.trusted(0)]),
        Err(AssertionFault::WrongRpId)
    );
}

#[test]
fn rejects_uv_unset() {
    let passkey = SoftPasskey::es256();
    let assertion = passkey.assert_with(RP_ID, ORIGIN, FLAG_USER_PRESENT, &digest(GATE));
    assert_eq!(
        checked(&assertion, &[passkey.trusted(0)]),
        Err(AssertionFault::UserNotVerified)
    );
}

#[test]
fn rejects_up_unset() {
    let passkey = SoftPasskey::es256();
    let assertion = passkey.assert_with(RP_ID, ORIGIN, FLAG_USER_VERIFIED, &digest(GATE));
    assert_eq!(
        checked(&assertion, &[passkey.trusted(0)]),
        Err(AssertionFault::UserNotPresent)
    );
}

/// A registration's client data replayed as an assertion, signed by the
/// trusted key, so the type is the only thing wrong with it.
#[test]
fn rejects_wrong_type() {
    let passkey = SoftPasskey::es256();
    let assertion = passkey.sign(
        &authenticator_data(RP_ID, UP_AND_UV, 0),
        &client_data("webauthn.create", ORIGIN, &digest(GATE)),
    );
    assert_eq!(
        checked(&assertion, &[passkey.trusted(0)]),
        Err(AssertionFault::WrongType)
    );
}

#[test]
fn rejects_an_untrusted_credential() {
    let passkey = SoftPasskey::es256();
    let assertion = passkey.assert(&digest(GATE));
    assert_eq!(
        checked(&assertion, &[]),
        Err(AssertionFault::CredentialNotTrusted)
    );
    assert_eq!(
        checked(&assertion, &[SoftPasskey::es256().trusted(0)]),
        Err(AssertionFault::CredentialNotTrusted)
    );
}

/// The passkey's own credential id, trusted under the 76-byte ES256 key of
/// `cose_rejects_non_canonical_keys`.
#[test]
fn rejects_a_trusted_key_that_is_not_canonical_cose() {
    let passkey = SoftPasskey::es256();
    let set = [TrustedPasskey {
        public_key: encode(&passkey.public_key_cose()[..76]),
        ..passkey.trusted(0)
    }];
    assert_eq!(
        checked(&passkey.assert(&digest(GATE)), &set),
        Err(AssertionFault::UnsupportedKey)
    );
    assert_eq!(
        assertion_refusal(AssertionFault::UnsupportedKey),
        RefusalReason::AssertionInvalid
    );
}

#[test]
fn rejects_a_signature_by_another_key() {
    for (trusted_key, impostor) in [
        (SoftPasskey::es256(), SoftPasskey::es256()),
        (SoftPasskey::ed25519(), SoftPasskey::ed25519()),
    ] {
        let mut assertion = impostor.assert(&digest(GATE));
        assertion.credential_id = trusted_key.credential_id();
        assert_eq!(
            checked(&assertion, &[trusted_key.trusted(0)]),
            Err(AssertionFault::BadSignature)
        );
    }
}

#[test]
fn rejects_a_flipped_signature_byte() {
    for passkey in [SoftPasskey::es256(), SoftPasskey::ed25519()] {
        let mut assertion = passkey.assert(&digest(GATE));
        assertion.signature = flip_last(&assertion.signature);
        assert_eq!(
            checked(&assertion, &[passkey.trusted(0)]),
            Err(AssertionFault::BadSignature)
        );
    }
}

/// Unsigned client data that is not even JSON: `BadSignature`, not
/// `Malformed`, because the signature is checked before the parser runs.
#[test]
fn checks_the_signature_before_reading_client_data() {
    let passkey = SoftPasskey::es256();
    let mut assertion = passkey.assert(&digest(GATE));
    assertion.client_data_json = encode(b"webauthn.get");
    assert_eq!(
        checked(&assertion, &[passkey.trusted(0)]),
        Err(AssertionFault::BadSignature)
    );
}

/// Re-signed, so the parser does run: `Malformed`, where the unsigned copy in
/// `checks_the_signature_before_reading_client_data` is `BadSignature`.
#[test]
fn rejects_re_signed_client_data_that_is_not_json() {
    let passkey = SoftPasskey::es256();
    let assertion = passkey.sign(&authenticator_data(RP_ID, UP_AND_UV, 0), b"webauthn.get");
    assert_eq!(
        checked(&assertion, &[passkey.trusted(0)]),
        Err(AssertionFault::Malformed)
    );
}

/// One byte short of the counter, re-signed, and stored at 0: a short counter
/// read as 0 would be accepted. `accepts_a_valid_es256_assertion` is the
/// 37-byte case, so the floor is pinned from both sides.
#[test]
fn rejects_authenticator_data_shorter_than_37_bytes() {
    let passkey = SoftPasskey::es256();
    let auth_data = authenticator_data(RP_ID, UP_AND_UV, 0);
    assert_eq!(auth_data.len(), 37);
    let assertion = passkey.sign(
        &auth_data[..36],
        &client_data("webauthn.get", ORIGIN, &digest(GATE)),
    );
    assert_eq!(
        checked(&assertion, &[passkey.trusted(0)]),
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
        let passkey = SoftPasskey::es256().with_sign_count(reported);
        assert_eq!(
            checked(&passkey.assert(&digest(GATE)), &[passkey.trusted(stored)]),
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

/// One byte past its bound, each field is `Malformed` even where every later
/// check would pass: re-signed, and under a credential id the trusted set
/// holds. `verify_assertion` measures the text before decoding it and
/// `verify_assertion_bytes` the bytes it is handed, so each case goes through
/// both. The bounds are spelled here rather than read from `super`, so that
/// moving one moves this test's expectation away from it.
///
/// No ES256 or Ed25519 signature is 128 bytes long, so a signature at its
/// bound is let through to the signature check and refused there.
#[test]
fn oversized_fields_are_malformed_before_decode() {
    const MAX_CREDENTIAL_ID: usize = 1023;
    const MAX_AUTHENTICATOR_DATA: usize = 1024;
    const MAX_CLIENT_DATA_JSON: usize = 4096;
    const MAX_SIGNATURE: usize = 128;

    let passkey = SoftPasskey::es256();
    let auth_data = authenticator_data(RP_ID, UP_AND_UV, 0);
    let client_json = client_data("webauthn.get", ORIGIN, &digest(GATE));
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
        let id = encode(&vec![0x5a; len]);
        let mut assertion = passkey.sign(&auth_data, &client_json);
        assertion.credential_id = id.clone();
        let set = [TrustedPasskey {
            credential_id: id,
            ..passkey.trusted(0)
        }];
        both(
            &format!("credential id of {len}"),
            &assertion,
            &set,
            expected,
        );
    }

    let set = [passkey.trusted(0)];
    for (len, expected) in [
        (MAX_AUTHENTICATOR_DATA, Ok(0)),
        (MAX_AUTHENTICATOR_DATA + 1, Err(AssertionFault::Malformed)),
    ] {
        let assertion = passkey.sign(&padded(&auth_data, len, 0x00), &client_json);
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
        let assertion = passkey.sign(&auth_data, &padded(&client_json, len, b' '));
        both(&format!("client data of {len}"), &assertion, &set, expected);
    }

    for (len, expected) in [
        (MAX_SIGNATURE, Err(AssertionFault::BadSignature)),
        (MAX_SIGNATURE + 1, Err(AssertionFault::Malformed)),
    ] {
        let mut assertion = passkey.sign(&auth_data, &client_json);
        assertion.signature = encode(&padded(&decode(&assertion.signature), len, 0x00));
        both(&format!("signature of {len}"), &assertion, &set, expected);
    }
}
