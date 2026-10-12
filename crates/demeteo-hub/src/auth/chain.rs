//! One passkey vouching for another, or removing one: the [`policy`] decision,
//! the challenge and the [`assertion`] check composed into a single answer, and
//! the record that answer leaves behind. Synchronous, no I/O; what the chain
//! protects is on [the module root](super).
//!
//! The order of the checks inside [`endorse`] and [`revoke`] is part of their
//! contract: callers and tests match on the exact [`ChainError`].
//!
//! A record is kept so that someone who does not trust the Hub can check it,
//! which decides two things here. It holds every input of its challenge and
//! the assertion exactly as received, never a parsed or re-encoded form. And
//! [`reverify_endorsement`] and [`reverify_revocation`] read nothing else: not
//! the clock, since a record is read long after its challenge expired and
//! expiry bounds only when an assertion may be *accepted*; and not any
//! [`Status`], which is what the keys were at the time and is not in the
//! record.

use demeteo_hub_protocol::{EncodeError, PasskeyEndorsement, PasskeyRevocation, WebAuthnAssertion};

use super::assertion::{self, AssertionError};
use super::challenge::{
    base64url_encode, endorsement_challenge, revocation_challenge, ChallengeBinding,
};
use super::policy::{self, KeyView, Refusal, Status};
use super::RelyingParty;

/// A passkey as the Hub holds it. `public_key` is its
/// [`COSE_Key`](super::cose) bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredKey {
    pub credential_id: String,
    pub public_key: Vec<u8>,
    pub status: Status,
}

/// The endorser is `body.assertion.credential_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndorsementRecord {
    pub binding: ChallengeBinding,
    pub body: PasskeyEndorsement,
}

/// The revoker is `body.assertion.credential_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevocationRecord {
    pub binding: ChallengeBinding,
    pub body: PasskeyRevocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error("the challenge has expired")]
    Expired,
    #[error(transparent)]
    Policy(#[from] Refusal),
    #[error(transparent)]
    Assertion(#[from] AssertionError),
    #[error("a challenge input is too long to encode")]
    Encode,
}

/// Accepts `assertion` as a trusted passkey's endorsement of
/// `pending_credential_id`, or says why not. `keys` is every passkey of one
/// tenant and `binding` the one the server minted for this ceremony; `now` is
/// Unix seconds, and the binding's last second is still in time.
///
/// The asserting key is the one `assertion.credential_id` names. The challenge
/// is computed from `binding` and the pending key as stored, and nothing a
/// request carries is read into it: a caller that could supply the challenge,
/// or the public key it covers, could have a trusted passkey's signature over
/// one key recorded as its endorsement of another.
pub fn endorse(
    rp: &RelyingParty,
    now: u64,
    keys: &[StoredKey],
    binding: &ChallengeBinding,
    pending_credential_id: &str,
    assertion: &WebAuthnAssertion,
) -> Result<EndorsementRecord, ChainError> {
    if now > binding.expires_at {
        return Err(ChainError::Expired);
    }
    policy::may_endorse(
        &views(keys),
        pending_credential_id,
        &assertion.credential_id,
    )?;
    let pending = stored(keys, pending_credential_id).ok_or(Refusal::UnknownCredential)?;
    let endorser = stored(keys, &assertion.credential_id).ok_or(Refusal::EndorserNotTrusted)?;

    let new_public_key = base64url_encode(&pending.public_key);
    let challenge = endorsement_challenge(binding, &pending.credential_id, &new_public_key)
        .map_err(|_| ChainError::Encode)?;
    assertion::verify(rp, &endorser.public_key, &challenge, assertion)?;

    Ok(EndorsementRecord {
        binding: binding.clone(),
        body: PasskeyEndorsement {
            new_credential_id: pending.credential_id.clone(),
            new_public_key,
            assertion: assertion.clone(),
        },
    })
}

/// Whether `record` holds an assertion by `endorser_public_key` over the
/// challenge its own fields give. The caller finds that key from
/// `record.body.assertion.credential_id`, among keys it already trusts.
pub fn reverify_endorsement(
    rp: &RelyingParty,
    endorser_public_key: &[u8],
    record: &EndorsementRecord,
) -> Result<(), AssertionError> {
    let challenge = endorsement_challenge(
        &record.binding,
        &record.body.new_credential_id,
        &record.body.new_public_key,
    );
    reverify(rp, endorser_public_key, challenge, &record.body.assertion)
}

/// Accepts `assertion` as a trusted passkey's revocation of
/// `target_credential_id`, or says why not. Every argument is as in
/// [`endorse`], and so is the rule that nothing a request carries is read
/// into the challenge.
///
/// The challenge covers the target's id and not its key. What keeps an
/// endorsement of a credential from being replayed as its removal is the label
/// `demeteo_hub_protocol::revocation_challenge` opens its pre-image with, not
/// anything checked here.
pub fn revoke(
    rp: &RelyingParty,
    now: u64,
    keys: &[StoredKey],
    binding: &ChallengeBinding,
    target_credential_id: &str,
    assertion: &WebAuthnAssertion,
) -> Result<RevocationRecord, ChainError> {
    if now > binding.expires_at {
        return Err(ChainError::Expired);
    }
    policy::may_revoke(&views(keys), target_credential_id, &assertion.credential_id)?;
    let revoker = stored(keys, &assertion.credential_id).ok_or(Refusal::RevokerNotTrusted)?;

    let challenge =
        revocation_challenge(binding, target_credential_id).map_err(|_| ChainError::Encode)?;
    assertion::verify(rp, &revoker.public_key, &challenge, assertion)?;

    Ok(RevocationRecord {
        binding: binding.clone(),
        body: PasskeyRevocation {
            credential_id: target_credential_id.to_owned(),
            assertion: assertion.clone(),
        },
    })
}

/// [`reverify_endorsement`] for a revocation: `revoker_public_key` is found
/// the same way.
pub fn reverify_revocation(
    rp: &RelyingParty,
    revoker_public_key: &[u8],
    record: &RevocationRecord,
) -> Result<(), AssertionError> {
    let challenge = revocation_challenge(&record.binding, &record.body.credential_id);
    reverify(rp, revoker_public_key, challenge, &record.body.assertion)
}

fn reverify(
    rp: &RelyingParty,
    public_key: &[u8],
    challenge: Result<[u8; 32], EncodeError>,
    assertion: &WebAuthnAssertion,
) -> Result<(), AssertionError> {
    // Fields that give no challenge cannot hold an assertion over one.
    let challenge = challenge.map_err(|_| AssertionError::WrongChallenge)?;
    assertion::verify(rp, public_key, &challenge, assertion).map(|_| ())
}

fn views(keys: &[StoredKey]) -> Vec<KeyView<'_>> {
    keys.iter()
        .map(|key| KeyView {
            credential_id: &key.credential_id,
            status: key.status,
        })
        .collect()
}

fn stored<'a>(keys: &'a [StoredKey], credential_id: &str) -> Option<&'a StoredKey> {
    keys.iter().find(|key| key.credential_id == credential_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::challenge::HUB_SCOPE;
    use crate::auth::testing::SoftAuthenticator;

    const NOW: u64 = 1_700_000_000;

    fn rp() -> RelyingParty {
        RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        }
    }

    fn binding() -> ChallengeBinding {
        ChallengeBinding {
            instance_id: HUB_SCOPE.to_owned(),
            request_id: "AAECAwQFBgcICQoLDA0ODw".to_owned(),
            expires_at: NOW + 300,
        }
    }

    fn challenge_for(pending: &SoftAuthenticator) -> [u8; 32] {
        endorsement_challenge(
            &binding(),
            &pending.credential_id(),
            &base64url_encode(&pending.public_key_cose()),
        )
        .unwrap()
    }

    #[test]
    fn an_endorsement_is_recorded_with_its_challenge_inputs_and_the_assertion_as_received() {
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let keys = [
            trusted.stored(Status::Trusted),
            pending.stored(Status::Pending),
        ];
        let assertion = trusted.assert(&rp(), &challenge_for(&pending));

        let record = endorse(
            &rp(),
            NOW,
            &keys,
            &binding(),
            &pending.credential_id(),
            &assertion,
        );

        assert_eq!(
            record,
            Ok(EndorsementRecord {
                binding: binding(),
                body: PasskeyEndorsement {
                    new_credential_id: pending.credential_id(),
                    new_public_key: base64url_encode(&pending.public_key_cose()),
                    assertion,
                },
            })
        );
    }

    #[test]
    fn a_binding_is_in_time_through_its_last_second_and_expired_after_it() {
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let keys = [
            trusted.stored(Status::Trusted),
            pending.stored(Status::Pending),
        ];
        let assertion = trusted.assert(&rp(), &challenge_for(&pending));
        let at = |now| {
            endorse(
                &rp(),
                now,
                &keys,
                &binding(),
                &pending.credential_id(),
                &assertion,
            )
            .map(|_| ())
        };

        assert_eq!(at(binding().expires_at), Ok(()));
        assert_eq!(at(binding().expires_at + 1), Err(ChainError::Expired));
    }

    /// Each request is wrong in two ways, and the earlier check answers.
    #[test]
    fn expiry_is_decided_before_policy_and_policy_before_the_assertion() {
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let keys = [
            trusted.stored(Status::Trusted),
            pending.stored(Status::Pending),
        ];
        let mut other_challenge = challenge_for(&pending);
        other_challenge[0] ^= 0x01;
        let self_signed_over_another_challenge = pending.assert(&rp(), &other_challenge);
        let at = |now| {
            endorse(
                &rp(),
                now,
                &keys,
                &binding(),
                &pending.credential_id(),
                &self_signed_over_another_challenge,
            )
        };

        assert_eq!(at(binding().expires_at + 1), Err(ChainError::Expired));
        assert_eq!(at(NOW), Err(ChainError::Policy(Refusal::SelfEndorsement)));
    }

    /// The key the stored row holds decides the challenge, whatever key the
    /// endorser was shown when it signed.
    #[test]
    fn the_challenge_covers_the_pending_key_as_stored() {
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let substitute = SoftAuthenticator::new();
        let keys = [
            trusted.stored(Status::Trusted),
            StoredKey {
                public_key: substitute.public_key_cose(),
                ..pending.stored(Status::Pending)
            },
        ];
        let assertion = trusted.assert(&rp(), &challenge_for(&pending));

        assert_eq!(
            endorse(
                &rp(),
                NOW,
                &keys,
                &binding(),
                &pending.credential_id(),
                &assertion,
            ),
            Err(ChainError::Assertion(AssertionError::WrongChallenge))
        );
    }

    #[test]
    fn a_record_reverifies_under_its_endorser_and_under_no_other_key() {
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        let record = EndorsementRecord {
            binding: binding(),
            body: PasskeyEndorsement {
                new_credential_id: pending.credential_id(),
                new_public_key: base64url_encode(&pending.public_key_cose()),
                assertion: trusted.assert(&rp(), &challenge_for(&pending)),
            },
        };

        assert_eq!(
            reverify_endorsement(&rp(), &trusted.public_key_cose(), &record),
            Ok(())
        );
        assert_eq!(
            reverify_endorsement(&rp(), &pending.public_key_cose(), &record),
            Err(AssertionError::BadSignature)
        );
    }

    #[test]
    fn a_revocation_is_recorded_with_its_challenge_inputs_and_the_assertion_as_received() {
        let revoker = SoftAuthenticator::new();
        let target = SoftAuthenticator::new();
        let keys = [
            revoker.stored(Status::Trusted),
            target.stored(Status::Trusted),
        ];
        let challenge = revocation_challenge(&binding(), &target.credential_id()).unwrap();
        let assertion = revoker.assert(&rp(), &challenge);

        let record = revoke(
            &rp(),
            NOW,
            &keys,
            &binding(),
            &target.credential_id(),
            &assertion,
        );

        assert_eq!(
            record,
            Ok(RevocationRecord {
                binding: binding(),
                body: PasskeyRevocation {
                    credential_id: target.credential_id(),
                    assertion,
                },
            })
        );
    }

    /// Each request is wrong in two ways, and the earlier check answers.
    #[test]
    fn a_revocation_is_decided_in_the_order_an_endorsement_is() {
        let other = SoftAuthenticator::new();
        let target = SoftAuthenticator::new();
        let keys = [
            other.stored(Status::Trusted),
            target.stored(Status::Trusted),
        ];
        let mut other_challenge =
            revocation_challenge(&binding(), &target.credential_id()).unwrap();
        other_challenge[0] ^= 0x01;
        let self_signed_over_another_challenge = target.assert(&rp(), &other_challenge);
        let at = |now| {
            revoke(
                &rp(),
                now,
                &keys,
                &binding(),
                &target.credential_id(),
                &self_signed_over_another_challenge,
            )
        };

        assert_eq!(
            at(binding().expires_at),
            Err(ChainError::Policy(Refusal::SelfRevocation))
        );
        assert_eq!(at(binding().expires_at + 1), Err(ChainError::Expired));
    }

    #[test]
    fn a_revocation_record_reverifies_under_its_revoker_and_under_no_other_key() {
        let revoker = SoftAuthenticator::new();
        let target = SoftAuthenticator::new();
        let challenge = revocation_challenge(&binding(), &target.credential_id()).unwrap();
        let record = RevocationRecord {
            binding: binding(),
            body: PasskeyRevocation {
                credential_id: target.credential_id(),
                assertion: revoker.assert(&rp(), &challenge),
            },
        };

        assert_eq!(
            reverify_revocation(&rp(), &revoker.public_key_cose(), &record),
            Ok(())
        );
        assert_eq!(
            reverify_revocation(&rp(), &target.public_key_cose(), &record),
            Err(AssertionError::BadSignature)
        );
    }
}
