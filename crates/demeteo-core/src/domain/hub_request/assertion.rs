//! The desktop's check of a WebAuthn assertion by one of its trusted passkeys
//! (docs/HUB.md §8, DECISIONS 66). It mirrors the Hub's
//! `crates/demeteo-hub/src/auth/assertion.rs`, which this crate must not
//! depend on. Where the desktop is the authority it differs from it: it finds
//! the key from the credential id itself, and it refuses a counter that does
//! not advance. It also bounds each field before decoding it, and accepts an
//! Ed25519 key as well as ES256, where the Hub accepts ES256 only.
//!
//! The order differs too. The signature is checked before `clientDataJSON` is
//! parsed, where the Hub checks it last, so nothing the trusted key did not
//! sign reaches the JSON parser. A negative test vector therefore has to be
//! re-signed, or it is refused as a bad signature before the check it is meant
//! for runs.

use aws_lc_rs::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1, ED25519};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use demeteo_hub_protocol::WebAuthnAssertion;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::cose::{parse_cose_key, PasskeyKey};
use super::passkeys::TrustedPasskey;

pub const FLAG_USER_PRESENT: u8 = 0x01;
pub const FLAG_USER_VERIFIED: u8 = 0x04;

const GET_TYPE: &str = "webauthn.get";

const RP_ID_HASH_LEN: usize = 32;
const COUNTER_LEN: usize = 4;

/// WebAuthn L2 caps a credential id at 1023 bytes. The other bounds are well
/// above what an authenticator or browser produces; they exist so that a
/// hostile Hub cannot make the desktop decode, hash or parse more than this.
pub(super) const MAX_CREDENTIAL_ID_LEN: usize = 1023;
const MAX_AUTHENTICATOR_DATA_LEN: usize = 1024;
const MAX_CLIENT_DATA_JSON_LEN: usize = 4096;
const MAX_SIGNATURE_LEN: usize = 128;

/// What the desktop pinned at pairing. `origin` is the pinned `hub_origin`,
/// not `https://` followed by `rp_id`: the two differ when the RP ID is a
/// parent domain, and the Hub compares against its own configured origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelyingPartyPins<'a> {
    pub rp_id: &'a str,
    pub origin: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AssertionFault {
    #[error("the assertion is not bounded base64url of well-formed WebAuthn data")]
    Malformed,
    #[error("the credential is not in the trusted passkey set")]
    CredentialNotTrusted,
    #[error("the trusted public key is not a canonical ES256 or Ed25519 COSE_Key")]
    UnsupportedKey,
    #[error("the signature does not verify under the trusted public key")]
    BadSignature,
    #[error("the client data is not of type {GET_TYPE}")]
    WrongType,
    #[error("the assertion is over another challenge")]
    WrongChallenge,
    #[error("the assertion was made at another origin")]
    WrongOrigin,
    #[error("the assertion was made for another relying party")]
    WrongRpId,
    #[error("the authenticator did not report user presence")]
    UserNotPresent,
    #[error("the authenticator did not verify the user")]
    UserNotVerified,
    #[error("the signature counter did not advance")]
    CounterRegressed,
}

/// An assertion every check accepted. `sign_count` is what the authenticator
/// reported, the value the caller stores for `passkey` once it commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedAssertion<'t> {
    pub passkey: &'t TrustedPasskey,
    pub sign_count: u32,
}

/// [`WebAuthnAssertion`] decoded from base64url.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssertionBytes<'a> {
    pub credential_id: &'a [u8],
    pub authenticator_data: &'a [u8],
    pub client_data_json: &'a [u8],
    pub signature: &'a [u8],
}

/// Only these members are checked. A browser sends others (`crossOrigin`,
/// `topOrigin`), which are ignored rather than refused.
#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
}

/// WebAuthn L2 §7.2 step 21. A synced passkey reports zero on every
/// assertion, so zero after zero is accepted; any other count has to grow,
/// and an equal non-zero count is refused as a possible clone.
pub fn counter_advances(stored: u32, reported: u32) -> bool {
    (stored == 0 && reported == 0) || reported > stored
}

/// [`verify_assertion_bytes`] over the base64url fields, each measured
/// against its bound before it is decoded.
pub fn verify_assertion<'t>(
    assertion: &WebAuthnAssertion,
    trusted: &'t [TrustedPasskey],
    pins: RelyingPartyPins<'_>,
    challenge: &[u8; 32],
) -> Result<VerifiedAssertion<'t>, AssertionFault> {
    let credential_id = bounded_decode(&assertion.credential_id, MAX_CREDENTIAL_ID_LEN)?;
    let authenticator_data =
        bounded_decode(&assertion.authenticator_data, MAX_AUTHENTICATOR_DATA_LEN)?;
    let client_data_json = bounded_decode(&assertion.client_data_json, MAX_CLIENT_DATA_JSON_LEN)?;
    let signature = bounded_decode(&assertion.signature, MAX_SIGNATURE_LEN)?;
    verify_bounded(
        AssertionBytes {
            credential_id: &credential_id,
            authenticator_data: &authenticator_data,
            client_data_json: &client_data_json,
            signature: &signature,
        },
        trusted,
        pins,
        challenge,
    )
}

/// Each check refuses with its own [`AssertionFault`]. `challenge` is the
/// digest the caller recomputed from the request, never one the Hub supplied.
pub fn verify_assertion_bytes<'t>(
    assertion: AssertionBytes<'_>,
    trusted: &'t [TrustedPasskey],
    pins: RelyingPartyPins<'_>,
    challenge: &[u8; 32],
) -> Result<VerifiedAssertion<'t>, AssertionFault> {
    if assertion.credential_id.len() > MAX_CREDENTIAL_ID_LEN
        || assertion.authenticator_data.len() > MAX_AUTHENTICATOR_DATA_LEN
        || assertion.client_data_json.len() > MAX_CLIENT_DATA_JSON_LEN
        || assertion.signature.len() > MAX_SIGNATURE_LEN
    {
        return Err(AssertionFault::Malformed);
    }
    verify_bounded(assertion, trusted, pins, challenge)
}

/// Every check after the size bounds, which each entry point applies in its
/// own units so that neither relies on the other's: text before it is
/// decoded, bytes as handed over.
fn verify_bounded<'t>(
    assertion: AssertionBytes<'_>,
    trusted: &'t [TrustedPasskey],
    pins: RelyingPartyPins<'_>,
    challenge: &[u8; 32],
) -> Result<VerifiedAssertion<'t>, AssertionFault> {
    let AssertionBytes {
        credential_id,
        authenticator_data,
        client_data_json,
        signature,
    } = assertion;
    let (rp_id_hash, rest) = authenticator_data
        .split_first_chunk::<RP_ID_HASH_LEN>()
        .ok_or(AssertionFault::Malformed)?;
    let (&flags, rest) = rest.split_first().ok_or(AssertionFault::Malformed)?;
    let (counter, _) = rest
        .split_first_chunk::<COUNTER_LEN>()
        .ok_or(AssertionFault::Malformed)?;
    let reported = u32::from_be_bytes(*counter);

    let passkey = trusted
        .iter()
        .find(|passkey| {
            URL_SAFE_NO_PAD
                .decode(&passkey.credential_id)
                .is_ok_and(|id| id == credential_id)
        })
        .ok_or(AssertionFault::CredentialNotTrusted)?;

    let key = URL_SAFE_NO_PAD
        .decode(&passkey.public_key)
        .ok()
        .and_then(|cose| parse_cose_key(&cose))
        .ok_or(AssertionFault::UnsupportedKey)?;

    let signed = [authenticator_data, &Sha256::digest(client_data_json)].concat();
    let verified = match &key {
        PasskeyKey::Es256(sec1) => {
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, sec1).verify(&signed, signature)
        }
        PasskeyKey::Ed25519(raw) => {
            UnparsedPublicKey::new(&ED25519, raw).verify(&signed, signature)
        }
    };
    verified.map_err(|_| AssertionFault::BadSignature)?;

    let client_data: ClientData =
        serde_json::from_slice(client_data_json).map_err(|_| AssertionFault::Malformed)?;
    if client_data.kind != GET_TYPE {
        return Err(AssertionFault::WrongType);
    }
    if URL_SAFE_NO_PAD
        .decode(&client_data.challenge)
        .ok()
        .as_deref()
        != Some(challenge.as_slice())
    {
        return Err(AssertionFault::WrongChallenge);
    }
    if client_data.origin != pins.origin {
        return Err(AssertionFault::WrongOrigin);
    }
    if rp_id_hash[..] != Sha256::digest(pins.rp_id.as_bytes())[..] {
        return Err(AssertionFault::WrongRpId);
    }
    if flags & FLAG_USER_PRESENT == 0 {
        return Err(AssertionFault::UserNotPresent);
    }
    if flags & FLAG_USER_VERIFIED == 0 {
        return Err(AssertionFault::UserNotVerified);
    }
    if !counter_advances(passkey.sign_count, reported) {
        return Err(AssertionFault::CounterRegressed);
    }

    Ok(VerifiedAssertion {
        passkey,
        sign_count: reported,
    })
}

/// The text is measured before it is decoded: unpadded base64url of `max`
/// bytes is `ceil(4 * max / 3)` characters, and anything longer cannot decode
/// to `max` bytes or fewer.
pub(super) fn bounded_decode(text: &str, max: usize) -> Result<Vec<u8>, AssertionFault> {
    if text.len() > (4 * max).div_ceil(3) {
        return Err(AssertionFault::Malformed);
    }
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| AssertionFault::Malformed)
}

#[cfg(test)]
#[path = "../../../tests/domain/hub_request/assertion.rs"]
mod assertion_tests;
