//! Verifies a WebAuthn assertion over a challenge the caller chose. Of the
//! checks under "What the desktop verifies" in `docs/HUB.md` §8 it shares the
//! COSE key, the signature, the `clientDataJSON` type and challenge, the
//! origin, the RP ID hash, and the user-present and user-verified flags. An
//! instance runs those before it changes its pinned set, and [`super`] says
//! why the Hub may not accept an assertion an instance would refuse.
//!
//! An instance's other checks are not here. No field is bounded before it is
//! decoded. Finding the key the assertion claims to be made under is the
//! caller's: [`verify`] is handed that key and never reads
//! `assertion.credential_id`. No counter is compared, where an instance
//! refuses a count that does not advance for its own pinned set.
//!
//! The shared checks run in another order and accept less. This decodes the
//! fields and checks the type and challenge first (`shaped`), then the origin,
//! the RP ID hash and the flags, then the COSE key, and the signature last; an
//! instance verifies the signature before it parses `clientDataJSON`. This
//! accepts an ES256 key only, where an instance also accepts Ed25519.

use aws_lc_rs::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use demeteo_hub_protocol::WebAuthnAssertion;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::challenge::base64url_decode;
use super::{cose, RelyingParty};

pub const FLAG_USER_PRESENT: u8 = 0x01;
pub const FLAG_USER_VERIFIED: u8 = 0x04;

const GET_TYPE: &str = "webauthn.get";

const RP_ID_HASH_LEN: usize = 32;
const COUNTER_LEN: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AssertionError {
    #[error("the assertion is not base64url of well-formed WebAuthn data")]
    Malformed,
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
    #[error("the public key is not an ES256 COSE_Key")]
    UnsupportedKey,
    #[error("the signature does not verify under the public key")]
    BadSignature,
}

/// The members of `clientDataJSON` that are checked. A browser adds others
/// (`crossOrigin`, `topOrigin`), so unknown members are not refused.
#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
}

/// The fixed head of authenticator data, `rpIdHash ‖ flags ‖ signCount`.
/// Attested credential data and extensions may follow the counter; nothing
/// here reads them.
struct AuthenticatorData<'a> {
    rp_id_hash: &'a [u8; RP_ID_HASH_LEN],
    flags: u8,
    sign_count: u32,
}

impl<'a> AuthenticatorData<'a> {
    fn parse(bytes: &'a [u8]) -> Option<Self> {
        let (rp_id_hash, rest) = bytes.split_first_chunk::<RP_ID_HASH_LEN>()?;
        let (flags, rest) = rest.split_first()?;
        let (counter, _) = rest.split_first_chunk::<COUNTER_LEN>()?;
        Some(Self {
            rp_id_hash,
            flags: *flags,
            sign_count: u32::from_be_bytes(*counter),
        })
    }
}

/// The signature counter `assertion` reports: what [`verify`] returns for it,
/// read without verifying anything. It is for a caller that holds an assertion
/// [`verify`] already accepted and kept only the assertion. `None` when the
/// authenticator data is not base64url or is too short to hold a counter.
pub fn sign_count(assertion: &WebAuthnAssertion) -> Option<u32> {
    let authenticator_data = base64url_decode(&assertion.authenticator_data)?;
    Some(AuthenticatorData::parse(&authenticator_data)?.sign_count)
}

/// Returns the signature counter the authenticator reported.
///
/// The counter is never compared with anything, here or by a caller deciding
/// whether to accept: a synced passkey reports zero on every assertion, so a
/// check that it grew would lock those keys out.
pub fn verify(
    rp: &RelyingParty,
    public_key_cose: &[u8],
    challenge: &[u8; 32],
    assertion: &WebAuthnAssertion,
) -> Result<u32, AssertionError> {
    let Shaped {
        authenticator_data,
        client_data_json,
        signature,
        rp_id_hash,
        flags,
        sign_count,
        origin,
    } = shaped(challenge, assertion)?;

    if origin != rp.origin {
        return Err(AssertionError::WrongOrigin);
    }
    if rp_id_hash[..] != Sha256::digest(rp.rp_id.as_bytes())[..] {
        return Err(AssertionError::WrongRpId);
    }
    if flags & FLAG_USER_PRESENT == 0 {
        return Err(AssertionError::UserNotPresent);
    }
    if flags & FLAG_USER_VERIFIED == 0 {
        return Err(AssertionError::UserNotVerified);
    }

    let point = cose::parse(public_key_cose).map_err(|_| AssertionError::UnsupportedKey)?;
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, point.to_sec1_uncompressed())
        .verify(
            &signed_bytes(&authenticator_data, &client_data_json),
            &signature,
        )
        .map_err(|_| AssertionError::BadSignature)?;

    Ok(sign_count)
}

/// An assertion that has passed every step of [`verify`] that needs neither
/// a key nor a relying party.
pub(crate) struct Shaped {
    authenticator_data: Vec<u8>,
    client_data_json: Vec<u8>,
    signature: Vec<u8>,
    rp_id_hash: [u8; RP_ID_HASH_LEN],
    flags: u8,
    sign_count: u32,
    origin: String,
}

/// Decodes `assertion` and checks that its client data is a `webauthn.get`
/// over `challenge`: [`verify`] up to the origin, refusing what it refuses
/// there and in the same order. Nothing it accepts is evidence that anyone
/// signed anything.
pub(crate) fn shaped(
    challenge: &[u8; 32],
    assertion: &WebAuthnAssertion,
) -> Result<Shaped, AssertionError> {
    let decode = |text: &str| base64url_decode(text).ok_or(AssertionError::Malformed);
    let authenticator_data = decode(&assertion.authenticator_data)?;
    let client_data_json = decode(&assertion.client_data_json)?;
    let signature = decode(&assertion.signature)?;

    let AuthenticatorData {
        rp_id_hash,
        flags,
        sign_count,
    } = AuthenticatorData::parse(&authenticator_data).ok_or(AssertionError::Malformed)?;
    let rp_id_hash = *rp_id_hash;
    let client_data: ClientData =
        serde_json::from_slice(&client_data_json).map_err(|_| AssertionError::Malformed)?;

    if client_data.kind != GET_TYPE {
        return Err(AssertionError::WrongType);
    }
    if base64url_decode(&client_data.challenge).as_deref() != Some(challenge.as_slice()) {
        return Err(AssertionError::WrongChallenge);
    }
    Ok(Shaped {
        authenticator_data,
        client_data_json,
        signature,
        rp_id_hash,
        flags,
        sign_count,
        origin: client_data.origin,
    })
}

/// `authenticatorData ‖ SHA-256(clientDataJSON)`: what an authenticator signs.
fn signed_bytes(authenticator_data: &[u8], client_data_json: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(authenticator_data.len() + Sha256::output_size());
    out.extend_from_slice(authenticator_data);
    out.extend_from_slice(&Sha256::digest(client_data_json));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::challenge::base64url_encode;
    use crate::auth::testing::SoftAuthenticator;

    const CHALLENGE: [u8; 32] = [0x5a; 32];
    const UP_AND_UV: u8 = FLAG_USER_PRESENT | FLAG_USER_VERIFIED;
    const AUTHENTICATOR_DATA_LEN: usize = RP_ID_HASH_LEN + 1 + COUNTER_LEN;

    fn rp() -> RelyingParty {
        RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        }
    }

    fn verified(
        authenticator: &SoftAuthenticator,
        assertion: &WebAuthnAssertion,
    ) -> Result<u32, AssertionError> {
        verify(
            &rp(),
            &authenticator.public_key_cose(),
            &CHALLENGE,
            assertion,
        )
    }

    fn reencoded(text: &str, change: impl FnOnce(&mut Vec<u8>)) -> String {
        let mut bytes = base64url_decode(text).unwrap();
        change(&mut bytes);
        base64url_encode(&bytes)
    }

    #[test]
    fn an_assertion_verifies_and_returns_the_counter_it_reported() {
        let authenticator = SoftAuthenticator::new().with_sign_count(0x0102_0304);
        let assertion = authenticator.assert(&rp(), &CHALLENGE);
        assert_eq!(verified(&authenticator, &assertion), Ok(0x0102_0304));
    }

    #[test]
    fn a_zero_counter_is_accepted() {
        let authenticator = SoftAuthenticator::new();
        let first = authenticator.assert(&rp(), &CHALLENGE);
        let second = authenticator.assert(&rp(), &CHALLENGE);
        assert_eq!(verified(&authenticator, &first), Ok(0));
        assert_eq!(verified(&authenticator, &second), Ok(0));
    }

    #[test]
    fn the_counter_is_read_from_an_assertion_without_verifying_it() {
        let authenticator = SoftAuthenticator::new().with_sign_count(0x0102_0304);
        let assertion = authenticator.assert(&rp(), &CHALLENGE);
        assert_eq!(sign_count(&assertion), Some(0x0102_0304));
        assert_eq!(
            sign_count(&SoftAuthenticator::new().assert(&rp(), &CHALLENGE)),
            Some(0)
        );
    }

    #[test]
    fn authenticator_data_that_holds_no_counter_reports_none() {
        let authenticator = SoftAuthenticator::new().with_sign_count(7);
        let get = authenticator.assert(&rp(), &CHALLENGE);
        let authenticator_data = base64url_decode(&get.authenticator_data).unwrap();
        let client_data_json = base64url_decode(&get.client_data_json).unwrap();
        for len in [0, RP_ID_HASH_LEN, AUTHENTICATOR_DATA_LEN - 1] {
            let short = authenticator.sign(&authenticator_data[..len], &client_data_json);
            assert_eq!(sign_count(&short), None, "{len} bytes");
        }
        let mut padded = get;
        padded.authenticator_data.push('=');
        assert_eq!(sign_count(&padded), None);
    }

    #[test]
    fn an_assertion_over_another_challenge_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let mut other = CHALLENGE;
        other[31] ^= 0x01;
        let assertion = authenticator.assert(&rp(), &other);
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::WrongChallenge)
        );
    }

    #[test]
    fn a_challenge_that_is_a_prefix_of_the_expected_one_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let rp = rp();
        let assertion =
            authenticator.assert_with(&rp.rp_id, &rp.origin, UP_AND_UV, &CHALLENGE[..31]);
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::WrongChallenge)
        );
    }

    #[test]
    fn an_assertion_made_at_another_origin_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let assertion = authenticator.assert_with(
            &rp().rp_id,
            "https://hub.example.evil.example",
            UP_AND_UV,
            &CHALLENGE,
        );
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::WrongOrigin)
        );
    }

    #[test]
    fn an_assertion_for_another_rp_id_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let assertion =
            authenticator.assert_with("other.example", &rp().origin, UP_AND_UV, &CHALLENGE);
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::WrongRpId)
        );
    }

    #[test]
    fn an_assertion_without_user_presence_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let rp = rp();
        let assertion =
            authenticator.assert_with(&rp.rp_id, &rp.origin, FLAG_USER_VERIFIED, &CHALLENGE);
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::UserNotPresent)
        );
    }

    #[test]
    fn an_assertion_without_user_verification_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let rp = rp();
        let assertion =
            authenticator.assert_with(&rp.rp_id, &rp.origin, FLAG_USER_PRESENT, &CHALLENGE);
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::UserNotVerified)
        );
    }

    #[test]
    fn one_altered_signature_byte_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let mut assertion = authenticator.assert(&rp(), &CHALLENGE);
        assertion.signature = reencoded(&assertion.signature, |der| {
            let last = der.len() - 1;
            der[last] ^= 0x01;
        });
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::BadSignature)
        );
    }

    #[test]
    fn a_signature_by_another_authenticator_is_refused() {
        let signer = SoftAuthenticator::new();
        let claimed = SoftAuthenticator::new();
        let assertion = signer.assert(&rp(), &CHALLENGE);
        assert_eq!(
            verified(&claimed, &assertion),
            Err(AssertionError::BadSignature)
        );
    }

    /// The counter is inside what was signed, so a relay cannot rewrite it.
    #[test]
    fn an_altered_counter_breaks_the_signature() {
        let authenticator = SoftAuthenticator::new();
        let mut assertion = authenticator.assert(&rp(), &CHALLENGE);
        assertion.authenticator_data = reencoded(&assertion.authenticator_data, |data| {
            data[AUTHENTICATOR_DATA_LEN - 1] ^= 0x01;
        });
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::BadSignature)
        );
    }

    /// Signed by the authenticator, so the type is the only thing wrong with
    /// it: this is a registration's client data replayed as an assertion.
    #[test]
    fn client_data_of_type_create_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let get = authenticator.assert(&rp(), &CHALLENGE);
        let client_data = serde_json::json!({
            "type": "webauthn.create",
            "challenge": base64url_encode(&CHALLENGE),
            "origin": rp().origin,
        });
        let assertion = authenticator.sign(
            &base64url_decode(&get.authenticator_data).unwrap(),
            client_data.to_string().as_bytes(),
        );
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::WrongType)
        );
    }

    #[test]
    fn a_field_that_is_not_unpadded_base64url_is_refused() {
        type Field = fn(&mut WebAuthnAssertion) -> &mut String;
        let authenticator = SoftAuthenticator::new();
        let fields: [(&str, Field); 3] = [
            ("authenticator_data", |a| &mut a.authenticator_data),
            ("client_data_json", |a| &mut a.client_data_json),
            ("signature", |a| &mut a.signature),
        ];
        for (name, field) in fields {
            let mut assertion = authenticator.assert(&rp(), &CHALLENGE);
            field(&mut assertion).push('=');
            assert_eq!(
                verified(&authenticator, &assertion),
                Err(AssertionError::Malformed),
                "{name} must be decoded"
            );
        }
    }

    #[test]
    fn authenticator_data_too_short_to_hold_a_counter_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let get = authenticator.assert(&rp(), &CHALLENGE);
        let authenticator_data = base64url_decode(&get.authenticator_data).unwrap();
        let assertion = authenticator.sign(
            &authenticator_data[..AUTHENTICATOR_DATA_LEN - 1],
            &base64url_decode(&get.client_data_json).unwrap(),
        );
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::Malformed)
        );
    }

    #[test]
    fn client_data_that_is_not_json_is_refused() {
        let authenticator = SoftAuthenticator::new();
        let get = authenticator.assert(&rp(), &CHALLENGE);
        let assertion = authenticator.sign(
            &base64url_decode(&get.authenticator_data).unwrap(),
            b"webauthn.get",
        );
        assert_eq!(
            verified(&authenticator, &assertion),
            Err(AssertionError::Malformed)
        );
    }

    #[test]
    fn a_key_that_is_not_es256_is_refused() {
        const ALG_OFFSET: usize = 4;
        const EDDSA: u8 = 0x27;
        let authenticator = SoftAuthenticator::new();
        let assertion = authenticator.assert(&rp(), &CHALLENGE);
        let mut key = authenticator.public_key_cose();
        key[ALG_OFFSET] = EDDSA;
        assert_eq!(
            verify(&rp(), &key, &CHALLENGE, &assertion),
            Err(AssertionError::UnsupportedKey)
        );
    }
}
