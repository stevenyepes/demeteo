//! A software passkey, ES256 or Ed25519, signing WebAuthn assertions with
//! `aws_lc_rs` keys, for the vectors of `domain::hub_request::{assertion,
//! passkeys}`.
//!
//! `src-tauri/tests/hub_gate_assertion.rs` runs the same vectors against the
//! Hub's own `SoftAuthenticator`, but that comes from `demeteo-hub`, a
//! Linux-only dev-dependency of `src-tauri`. This one needs nothing
//! `demeteo-core` does not already depend on, so the `cross-os-test` job runs
//! these vectors on macOS and Windows too. Every byte string an authenticator
//! would produce is built by hand here, the `COSE_Key` included, so a vector
//! does not lean on the parser it is checking.
//!
//! Reached as `crate::support::soft_passkey`, for the reason the header of
//! `preflight_strategy.rs` gives.

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{EcdsaKeyPair, Ed25519KeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use demeteo_hub_protocol::{Decision, WebAuthnAssertion};
use sha2::{Digest, Sha256};

use crate::domain::hub_request::assertion::{FLAG_USER_PRESENT, FLAG_USER_VERIFIED};
use crate::domain::hub_request::passkeys::{gate_challenge_digest, TrustedPasskey};

pub(crate) const RP_ID: &str = "hub.example";
pub(crate) const ORIGIN: &str = "https://hub.example";
pub(crate) const UP_AND_UV: u8 = FLAG_USER_PRESENT | FLAG_USER_VERIFIED;

/// The six fields `gate_challenge` binds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gate {
    pub instance_id: &'static str,
    pub feature_id: &'static str,
    pub step_execution_id: &'static str,
    pub decision: Decision,
    pub request_id: &'static str,
    pub expires_at: u64,
}

/// The docs/HUB.md §8 example, which every assertion is checked against.
pub(crate) const GATE: Gate = Gate {
    instance_id: "i-7f3a9c",
    feature_id: "f-1791004167577",
    step_execution_id: "se-0042",
    decision: Decision::Approve,
    request_id: "Zk3m0R8pQnVx2tL5aYw9bA",
    expires_at: 1_791_020_400,
};

pub(crate) fn digest(gate: Gate) -> [u8; 32] {
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

pub(crate) fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn decode(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(text).expect("unpadded base64url")
}

/// `b64url` with the low bit of its last decoded byte flipped.
pub(crate) fn flip_last(b64url: &str) -> String {
    let mut bytes = decode(b64url);
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    encode(&bytes)
}

/// The canonical ES256 `COSE_Key` of a SEC1 uncompressed point `04 ‖ x ‖ y`.
pub(crate) fn es256_cose(sec1: &[u8]) -> Vec<u8> {
    assert_eq!(sec1.len(), 65, "a SEC1 uncompressed P-256 point");
    assert_eq!(sec1[0], 0x04, "a SEC1 uncompressed P-256 point");
    [
        &[0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20][..],
        &sec1[1..33],
        &[0x22, 0x58, 0x20],
        &sec1[33..],
    ]
    .concat()
}

/// The canonical OKP Ed25519 `COSE_Key` of a raw public key.
pub(crate) fn ed25519_cose(x: &[u8]) -> Vec<u8> {
    assert_eq!(x.len(), 32, "a raw Ed25519 public key");
    [
        &[0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20][..],
        x,
    ]
    .concat()
}

/// `rpIdHash ‖ flags ‖ signCount`, the counter big-endian.
pub(crate) fn authenticator_data(rp_id: &str, flags: u8, sign_count: u32) -> Vec<u8> {
    [
        &Sha256::digest(rp_id.as_bytes())[..],
        &[flags],
        &sign_count.to_be_bytes(),
    ]
    .concat()
}

pub(crate) fn client_data(kind: &str, origin: &str, challenge: &[u8]) -> Vec<u8> {
    serde_json::json!({
        "type": kind,
        "challenge": encode(challenge),
        "origin": origin,
    })
    .to_string()
    .into_bytes()
}

enum Key {
    Es256(EcdsaKeyPair),
    Ed25519(Ed25519KeyPair),
}

pub(crate) struct SoftPasskey {
    key: Key,
    credential_id: Vec<u8>,
    sign_count: u32,
}

impl SoftPasskey {
    pub(crate) fn es256() -> Self {
        let pair =
            EcdsaKeyPair::generate(&ECDSA_P256_SHA256_ASN1_SIGNING).expect("generate a P-256 key");
        Self::with_key(Key::Es256(pair))
    }

    pub(crate) fn ed25519() -> Self {
        let pair = Ed25519KeyPair::generate().expect("generate an Ed25519 key");
        Self::with_key(Key::Ed25519(pair))
    }

    /// The credential id is derived from the public key, so two passkeys
    /// never share one.
    fn with_key(key: Key) -> Self {
        let public_key = match &key {
            Key::Es256(pair) => pair.public_key().as_ref().to_vec(),
            Key::Ed25519(pair) => pair.public_key().as_ref().to_vec(),
        };
        Self {
            key,
            credential_id: Sha256::digest(public_key)[..16].to_vec(),
            sign_count: 0,
        }
    }

    /// The counter every assertion of this passkey reports.
    pub(crate) fn with_sign_count(mut self, sign_count: u32) -> Self {
        self.sign_count = sign_count;
        self
    }

    pub(crate) fn credential_id(&self) -> String {
        encode(&self.credential_id)
    }

    pub(crate) fn public_key_cose(&self) -> Vec<u8> {
        match &self.key {
            Key::Es256(pair) => es256_cose(pair.public_key().as_ref()),
            Key::Ed25519(pair) => ed25519_cose(pair.public_key().as_ref()),
        }
    }

    /// This passkey as the trusted set holds it, with `sign_count` stored.
    pub(crate) fn trusted(&self, sign_count: u32) -> TrustedPasskey {
        TrustedPasskey {
            credential_id: self.credential_id(),
            public_key: encode(&self.public_key_cose()),
            sign_count,
        }
    }

    /// A valid assertion over `challenge` at [`RP_ID`] and [`ORIGIN`].
    pub(crate) fn assert(&self, challenge: &[u8; 32]) -> WebAuthnAssertion {
        self.assert_with(RP_ID, ORIGIN, UP_AND_UV, challenge)
    }

    pub(crate) fn assert_with(
        &self,
        rp_id: &str,
        origin: &str,
        flags: u8,
        challenge: &[u8; 32],
    ) -> WebAuthnAssertion {
        self.sign(
            &authenticator_data(rp_id, flags, self.sign_count),
            &client_data("webauthn.get", origin, challenge),
        )
    }

    /// Signs `authenticatorData ‖ SHA-256(clientDataJSON)` exactly as given,
    /// under this passkey's credential id.
    pub(crate) fn sign(
        &self,
        authenticator_data: &[u8],
        client_data_json: &[u8],
    ) -> WebAuthnAssertion {
        let signed = [authenticator_data, &Sha256::digest(client_data_json)].concat();
        let signature = match &self.key {
            Key::Es256(pair) => pair
                .sign(&SystemRandom::new(), &signed)
                .expect("sign the assertion")
                .as_ref()
                .to_vec(),
            Key::Ed25519(pair) => pair.sign(&signed).as_ref().to_vec(),
        };
        WebAuthnAssertion {
            credential_id: self.credential_id(),
            authenticator_data: encode(authenticator_data),
            client_data_json: encode(client_data_json),
            signature: encode(&signature),
        }
    }
}
