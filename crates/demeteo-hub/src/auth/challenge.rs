//! What an endorsement, a revocation or a gate decision assertion is made
//! over, and the one text form of every binary value that goes into it.
//!
//! A challenge is `SHA-256` of a pre-image that `demeteo_hub_protocol` builds.
//! The pre-image is never assembled here: an instance rebuilds it with that
//! same function, and a second spelling of the layout is a second thing for
//! the two ends to disagree about.
//!
//! The protocol hashes the UTF-8 bytes of the strings it is given, so the text
//! form of a credential id and of a public key is part of what is signed.
//! Every binary value is base64url without padding ([`base64url_encode`]): a
//! credential id from its raw bytes, a public key from its
//! [`COSE_Key`](super::cose) bytes, a challenge from its 32.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use demeteo_hub_protocol::{Decision, EncodeError, Scope};
use sha2::{Digest, Sha256};

use crate::store::requests::RequestId;
use crate::store::InstanceId;

/// The `instance_id` of a challenge made at the Hub for the Hub's own trust
/// decision: no single instance. The protocol binds an instance id into both
/// passkey challenges and an instance recomputes them from its own, so a
/// record bound to this value is re-verifiable as a signature and is not, as
/// it stands, an assertion addressed to any instance. Whether one may be
/// relayed is undecided (`docs/HUB.md` §11 item 9); nothing here may assume it.
pub const HUB_SCOPE: &str = "*";

pub const CHALLENGE_TTL_SECS: u64 = 300;

const REQUEST_ID_LEN: usize = 16;

/// The three inputs both passkey challenges share. They are stored with the
/// record they produced, because the challenge cannot be recomputed without
/// every one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeBinding {
    pub instance_id: String,
    pub request_id: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the system random source failed")]
pub struct RandomSourceFailed;

impl ChallengeBinding {
    /// A [`HUB_SCOPE`] binding with a fresh `request_id`, expiring
    /// [`CHALLENGE_TTL_SECS`] after `now` (Unix seconds).
    pub fn mint(now: u64) -> Result<Self, RandomSourceFailed> {
        let mut request_id = [0u8; REQUEST_ID_LEN];
        aws_lc_rs::rand::fill(&mut request_id).map_err(|_| RandomSourceFailed)?;
        Ok(Self {
            instance_id: HUB_SCOPE.to_owned(),
            request_id: base64url_encode(&request_id),
            expires_at: now.saturating_add(CHALLENGE_TTL_SECS),
        })
    }
}

/// The challenge a trusted passkey signs to vouch for `new_credential_id`,
/// whose key is `new_public_key`.
pub fn endorsement_challenge(
    binding: &ChallengeBinding,
    new_credential_id: &str,
    new_public_key: &str,
) -> Result<[u8; 32], EncodeError> {
    demeteo_hub_protocol::endorsement_challenge(
        &binding.instance_id,
        &binding.request_id,
        binding.expires_at,
        new_credential_id,
        new_public_key,
    )
    .map(|pre_image| digest(&pre_image))
}

/// The challenge a trusted passkey signs to remove `credential_id`.
pub fn revocation_challenge(
    binding: &ChallengeBinding,
    credential_id: &str,
) -> Result<[u8; 32], EncodeError> {
    demeteo_hub_protocol::revocation_challenge(
        &binding.instance_id,
        &binding.request_id,
        binding.expires_at,
        credential_id,
    )
    .map(|pre_image| digest(&pre_image))
}

/// The pre-image of the challenge a passkey signs to decide a gate. Unlike
/// the two above it is addressed to one instance, which rebuilds it from the
/// envelope of the `GateDecision` request: `instance` and `request_id` are
/// spelled here exactly as `socket::requests::unsigned` spells them there, and
/// a second spelling would be a challenge no instance can recompute.
pub fn gate_challenge_pre_image(
    instance: InstanceId,
    feature_id: &str,
    step_execution_id: &str,
    decision: Decision,
    request_id: &RequestId,
    expires_at: u64,
) -> Result<Vec<u8>, EncodeError> {
    demeteo_hub_protocol::gate_challenge(
        &instance.0.to_string(),
        feature_id,
        step_execution_id,
        decision,
        &base64url_encode(request_id),
        expires_at,
    )
}

/// The WebAuthn challenge over [`gate_challenge_pre_image`].
pub fn gate_challenge_digest(
    instance: InstanceId,
    feature_id: &str,
    step_execution_id: &str,
    decision: Decision,
    request_id: &RequestId,
    expires_at: u64,
) -> Result<[u8; 32], EncodeError> {
    gate_challenge_pre_image(
        instance,
        feature_id,
        step_execution_id,
        decision,
        request_id,
        expires_at,
    )
    .map(|pre_image| digest(&pre_image))
}

/// The challenge a passkey signs to approve a runner's pairing: its key and
/// the scopes the user left checked, under `binding`'s request id and
/// expiry. Its `instance_id` is not read, since no instance exists yet.
pub fn device_approval_challenge(
    binding: &ChallengeBinding,
    dpop_jkt: &str,
    scopes: &[Scope],
) -> Result<[u8; 32], EncodeError> {
    demeteo_hub_protocol::device_approval_challenge(
        dpop_jkt,
        scopes,
        &binding.request_id,
        binding.expires_at,
    )
    .map(|pre_image| digest(&pre_image))
}

fn digest(pre_image: &[u8]) -> [u8; 32] {
    Sha256::digest(pre_image).into()
}

pub fn base64url_encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// `None` for padding, the standard alphabet's `+` and `/`, or trailing bits
/// that are not zero: one byte string has one text form, and only that form
/// decodes.
pub fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::cose;

    const REQUEST_ID: &str = "AAECAwQFBgcICQoLDA0ODw";
    const EXPIRES_AT: u64 = 1_700_000_300;
    const CREDENTIAL_ID: &str = "bmV3LWNyZWRlbnRpYWw";
    const PUBLIC_KEY: &str = "pQECAyYgASFYIAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gIlggQUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVpbXF1eX2A";

    fn binding() -> ChallengeBinding {
        ChallengeBinding {
            instance_id: HUB_SCOPE.to_owned(),
            request_id: REQUEST_ID.to_owned(),
            expires_at: EXPIRES_AT,
        }
    }

    /// The literal was computed outside this crate, with Python's `hashlib`
    /// over a pre-image assembled by hand from the layout in the rustdoc of
    /// `demeteo_hub_protocol::endorsement_challenge`:
    ///
    /// ```text
    /// 00000016 "demeteo-hub/v1/endorse"   label
    /// 00000001                            version 1
    /// 00000001 "*"                        instance_id
    /// 00000016 REQUEST_ID                 request_id
    /// 000000006553f22c                    expires_at
    /// 00000013 CREDENTIAL_ID              new_credential_id
    /// 00000067 PUBLIC_KEY                 new_public_key
    /// ```
    #[test]
    fn the_endorsement_challenge_is_sha256_of_the_protocol_pre_image() {
        let expected: [u8; 32] = [
            0xfa, 0xb6, 0xf4, 0x30, 0x75, 0x25, 0x1f, 0x85, 0x16, 0xb4, 0x90, 0xd2, 0x6b, 0x9d,
            0x2e, 0x15, 0x4f, 0xc7, 0x69, 0xda, 0x8b, 0xde, 0xc7, 0xe1, 0xcb, 0xfa, 0xdd, 0x10,
            0x83, 0xba, 0xf3, 0xc6,
        ];
        assert_eq!(
            endorsement_challenge(&binding(), CREDENTIAL_ID, PUBLIC_KEY),
            Ok(expected)
        );
    }

    /// Computed the same way, over label `demeteo-hub/v1/revoke` and the
    /// layout that ends at `credential_id`.
    #[test]
    fn the_revocation_challenge_is_sha256_of_the_protocol_pre_image() {
        let expected: [u8; 32] = [
            0x9c, 0x85, 0x67, 0x58, 0x66, 0x23, 0x8a, 0x45, 0x9f, 0xd7, 0x9b, 0xc0, 0x81, 0x04,
            0x75, 0xc2, 0x5e, 0x6b, 0xd0, 0x8c, 0x40, 0x95, 0xc7, 0xb4, 0xbd, 0xa2, 0xad, 0xbf,
            0x57, 0x00, 0x78, 0x52,
        ];
        assert_eq!(
            revocation_challenge(&binding(), CREDENTIAL_ID),
            Ok(expected)
        );
    }

    #[test]
    fn each_endorsement_input_changes_the_digest() {
        let base = endorsement_challenge(&binding(), CREDENTIAL_ID, PUBLIC_KEY);
        let rebound = |change: fn(&mut ChallengeBinding)| {
            let mut b = binding();
            change(&mut b);
            endorsement_challenge(&b, CREDENTIAL_ID, PUBLIC_KEY)
        };
        let variants = [
            ("instance_id", rebound(|b| b.instance_id.push('x'))),
            ("request_id", rebound(|b| b.request_id.push('x'))),
            ("expires_at", rebound(|b| b.expires_at += 1)),
            (
                "new_credential_id",
                endorsement_challenge(&binding(), "bmV3LWNyZWRlbnRpYWx4", PUBLIC_KEY),
            ),
            (
                "new_public_key",
                endorsement_challenge(&binding(), CREDENTIAL_ID, CREDENTIAL_ID),
            ),
        ];
        assert!(base.is_ok());
        for (input, digest) in variants {
            assert!(digest.is_ok(), "{input} variant must encode");
            assert_ne!(digest, base, "changing {input} must change the digest");
        }
    }

    #[test]
    fn each_revocation_input_changes_the_digest() {
        let base = revocation_challenge(&binding(), CREDENTIAL_ID);
        let rebound = |change: fn(&mut ChallengeBinding)| {
            let mut b = binding();
            change(&mut b);
            revocation_challenge(&b, CREDENTIAL_ID)
        };
        let variants = [
            ("instance_id", rebound(|b| b.instance_id.push('x'))),
            ("request_id", rebound(|b| b.request_id.push('x'))),
            ("expires_at", rebound(|b| b.expires_at += 1)),
            (
                "credential_id",
                revocation_challenge(&binding(), "bmV3LWNyZWRlbnRpYWx4"),
            ),
        ];
        assert!(base.is_ok());
        for (input, digest) in variants {
            assert!(digest.is_ok(), "{input} variant must encode");
            assert_ne!(digest, base, "changing {input} must change the digest");
        }
    }

    #[test]
    fn endorsing_and_revoking_one_credential_are_different_challenges() {
        let endorse = endorsement_challenge(&binding(), CREDENTIAL_ID, PUBLIC_KEY);
        let revoke = revocation_challenge(&binding(), CREDENTIAL_ID);
        assert!(endorse.is_ok() && revoke.is_ok());
        assert_ne!(endorse, revoke);
    }

    const INSTANCE: &str = "67e55044-10b1-426f-9247-bb680e5fe0c8";
    const GATE_REQUEST_ID: RequestId = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

    fn instance() -> InstanceId {
        InstanceId(INSTANCE.parse().unwrap())
    }

    /// The protocol is called with literal spellings, so a digest over the
    /// instance's simple form or the request id's padded one does not pass.
    #[test]
    fn the_gate_challenge_is_sha256_of_the_protocol_pre_image() {
        let pre_image = demeteo_hub_protocol::gate_challenge(
            INSTANCE,
            "feature",
            "step-execution",
            Decision::Approve,
            REQUEST_ID,
            EXPIRES_AT,
        )
        .unwrap();
        let expected: [u8; 32] = Sha256::digest(&pre_image).into();

        assert_eq!(
            gate_challenge_digest(
                instance(),
                "feature",
                "step-execution",
                Decision::Approve,
                &GATE_REQUEST_ID,
                EXPIRES_AT,
            ),
            Ok(expected)
        );
        assert_eq!(
            gate_challenge_pre_image(
                instance(),
                "feature",
                "step-execution",
                Decision::Approve,
                &GATE_REQUEST_ID,
                EXPIRES_AT,
            ),
            Ok(pre_image)
        );
    }

    /// What an instance does with a `GateDecision` it is delivered: rebuild
    /// the challenge from the envelope's own fields.
    #[test]
    fn the_gate_challenge_is_recomputable_from_the_envelope() {
        use crate::socket::requests::{unsigned, Minted};
        use demeteo_hub_protocol::{CancelFeature, RequestPayload};

        let minted = Minted {
            id: GATE_REQUEST_ID,
            expires_at: EXPIRES_AT,
        };
        let payload = RequestPayload::CancelFeature(CancelFeature {
            feature_id: "feature".to_owned(),
        });
        let envelope = unsigned(minted, instance(), payload, EXPIRES_AT - 1);

        assert_eq!(
            demeteo_hub_protocol::gate_challenge(
                &envelope.instance_id,
                "feature",
                "step-execution",
                Decision::Cancel,
                &envelope.request_id,
                envelope.expires_at,
            ),
            gate_challenge_pre_image(
                instance(),
                "feature",
                "step-execution",
                Decision::Cancel,
                &minted.id,
                minted.expires_at,
            )
        );
    }

    #[test]
    fn a_minted_binding_is_hub_scoped_and_expires_after_the_ttl() {
        let minted = ChallengeBinding::mint(1_700_000_000).unwrap();
        assert_eq!(minted.instance_id, "*");
        assert_eq!(minted.expires_at, 1_700_000_300);
    }

    #[test]
    fn a_minted_request_id_is_16_fresh_bytes() {
        let first = ChallengeBinding::mint(0).unwrap();
        let second = ChallengeBinding::mint(0).unwrap();
        let decoded = base64url_decode(&first.request_id).unwrap();
        assert_eq!(decoded.len(), 16);
        assert_ne!(decoded, [0u8; 16]);
        assert_ne!(first.request_id, second.request_id);
    }

    #[test]
    fn base64url_is_the_url_alphabet_without_padding() {
        assert_eq!(base64url_encode(&[0xfb, 0xff]), "-_8");
        assert_eq!(base64url_decode("-_8"), Some(vec![0xfb, 0xff]));
        assert_eq!(base64url_encode(&[]), "");
        assert_eq!(base64url_decode(""), Some(Vec::new()));
    }

    #[test]
    fn base64url_decodes_only_the_one_text_form_of_a_value() {
        assert_eq!(base64url_decode("-_8="), None, "padding");
        assert_eq!(base64url_decode("+/8"), None, "standard alphabet");
        assert_eq!(base64url_decode("-_9"), None, "non-zero trailing bits");
    }

    #[test]
    fn a_public_key_as_a_string_is_base64url_of_its_cose_bytes() {
        let key = cose::encode(&cose::P256Point {
            x: std::array::from_fn(|i| 0x01 + i as u8),
            y: std::array::from_fn(|i| 0x41 + i as u8),
        });
        assert_eq!(base64url_encode(&key), PUBLIC_KEY);
        assert_eq!(base64url_decode(PUBLIC_KEY), Some(key.to_vec()));
    }
}
