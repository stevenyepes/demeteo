//! The trust decisions over one tenant's passkeys: synchronous, no I/O, each
//! reachable from a test with nothing but a slice. What they protect and the
//! designs they keep out are on [the module root](super).
//!
//! The order of the checks inside [`may_endorse`] and [`may_revoke`] is part of
//! their contract: callers and tests match on the exact [`Refusal`].

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Trusted,
    Revoked,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Trusted => "trusted",
            Self::Revoked => "revoked",
        }
    }

    /// `None` for anything but the three strings of [`Status::as_str`], exact
    /// case: an unrecognised status must fail the read, never pass as some
    /// other status.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "trusted" => Some(Self::Trusted),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

/// The two fields a decision reads. A decision can see only the keys it is
/// handed, so the caller passes every passkey of one tenant and no other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyView<'a> {
    pub credential_id: &'a str,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("no such passkey")]
    UnknownCredential,
    #[error("the passkey is not pending")]
    NotPending,
    #[error("a passkey cannot endorse itself")]
    SelfEndorsement,
    #[error("the endorsing passkey is not trusted")]
    EndorserNotTrusted,
    #[error("the passkey is already revoked")]
    AlreadyRevoked,
    #[error("the last trusted passkey cannot be revoked")]
    LastPasskey,
    #[error("a passkey cannot revoke itself")]
    SelfRevocation,
    #[error("the revoking passkey is not trusted")]
    RevokerNotTrusted,
    #[error("the tenant already holds the most passkeys it may")]
    TooManyPasskeys,
}

/// How many passkeys that are not revoked one tenant may hold. Every ceremony
/// reads the tenant's whole set and the two trust changes lock it, so without
/// a bound a session — a hijacked one included — could grow the cost of each
/// of them, and leave pending rows that are cleared one assertion at a time.
pub const MAX_ACTIVE_PASSKEYS: usize = 16;

/// The status a newly registered passkey is stored with, given the statuses of
/// the keys its tenant already has. Any existing key makes it pending, a
/// revoked one included: once a tenant has had a passkey, only an endorsement
/// makes another trusted.
pub fn admission(existing: &[Status]) -> Status {
    if existing.is_empty() {
        Status::Trusted
    } else {
        Status::Pending
    }
}

/// Whether a tenant holding `existing` may register one more. Pending keys
/// count as trusted ones do: they are the rows an attacker with a session can
/// write. Revoked ones do not, or a tenant that rotated its keys for long
/// enough could never add another.
pub fn may_be_added(existing: &[Status]) -> Result<(), Refusal> {
    let active = existing
        .iter()
        .filter(|status| **status != Status::Revoked)
        .count();
    if active >= MAX_ACTIVE_PASSKEYS {
        return Err(Refusal::TooManyPasskeys);
    }
    Ok(())
}

pub fn may_sign_in(status: Status) -> bool {
    status == Status::Trusted
}

/// The part of [`may_endorse`] that is known before any passkey has asserted.
/// A ceremony is refused on it at its start, so that nobody is asked to sign
/// for a passkey no signature could make trusted.
pub fn may_be_endorsed(keys: &[KeyView], pending: &str) -> Result<(), Refusal> {
    let target = status_of(keys, pending).ok_or(Refusal::UnknownCredential)?;
    if target != Status::Pending {
        return Err(Refusal::NotPending);
    }
    Ok(())
}

pub fn may_endorse(keys: &[KeyView], pending: &str, asserting: &str) -> Result<(), Refusal> {
    may_be_endorsed(keys, pending)?;
    // Looks redundant with the trust check below, since a pending key is not
    // trusted either. It stays, and stays first: the two refusals are distinct
    // answers, and with the order swapped this check could be deleted without
    // any test noticing.
    if asserting == pending {
        return Err(Refusal::SelfEndorsement);
    }
    if status_of(keys, asserting) != Some(Status::Trusted) {
        return Err(Refusal::EndorserNotTrusted);
    }
    Ok(())
}

/// The part of [`may_revoke`] that is known before any passkey has asserted,
/// refused at a ceremony's start for the reason [`may_be_endorsed`] gives.
///
/// A pending key may be revoked — it is how one is discarded — and that never
/// trips [`Refusal::LastPasskey`], which counts trusted keys only.
pub fn may_be_revoked(keys: &[KeyView], target: &str) -> Result<(), Refusal> {
    let target_status = status_of(keys, target).ok_or(Refusal::UnknownCredential)?;
    if target_status == Status::Revoked {
        return Err(Refusal::AlreadyRevoked);
    }
    if target_status == Status::Trusted && trusted_count(keys) == 1 {
        return Err(Refusal::LastPasskey);
    }
    Ok(())
}

pub fn may_revoke(keys: &[KeyView], target: &str, asserting: &str) -> Result<(), Refusal> {
    // Before the self check: a sole trusted key has no other key to assert with,
    // so every attempt on it is also a self-revocation, and the answer the
    // caller needs is that no assertion could succeed.
    may_be_revoked(keys, target)?;
    if asserting == target {
        return Err(Refusal::SelfRevocation);
    }
    if status_of(keys, asserting) != Some(Status::Trusted) {
        return Err(Refusal::RevokerNotTrusted);
    }
    Ok(())
}

fn status_of(keys: &[KeyView], credential_id: &str) -> Option<Status> {
    keys.iter()
        .find(|key| key.credential_id == credential_id)
        .map(|key| key.status)
}

fn trusted_count(keys: &[KeyView]) -> usize {
    keys.iter()
        .filter(|key| key.status == Status::Trusted)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(credential_id: &str, status: Status) -> KeyView<'_> {
        KeyView {
            credential_id,
            status,
        }
    }

    #[test]
    fn status_round_trips_its_three_strings_and_refuses_any_other() {
        for (status, s) in [
            (Status::Pending, "pending"),
            (Status::Trusted, "trusted"),
            (Status::Revoked, "revoked"),
        ] {
            assert_eq!(status.as_str(), s);
            assert_eq!(Status::parse(s), Some(status));
        }
        for other in ["", "Trusted", "TRUSTED", " trusted", "trusted ", "active"] {
            assert_eq!(Status::parse(other), None, "{other:?}");
        }
    }

    #[test]
    fn only_the_first_passkey_of_a_tenant_is_admitted_trusted() {
        assert_eq!(admission(&[]), Status::Trusted);
        assert_eq!(admission(&[Status::Trusted]), Status::Pending);
        assert_eq!(admission(&[Status::Pending]), Status::Pending);
        assert_eq!(admission(&[Status::Revoked]), Status::Pending);
    }

    fn holding(active: &[Status], revoked: usize) -> Vec<Status> {
        let mut existing = active.to_vec();
        existing.extend(std::iter::repeat_n(Status::Revoked, revoked));
        existing
    }

    #[test]
    fn a_passkey_may_be_added_until_the_tenant_holds_the_most_it_may() {
        let below = [Status::Trusted; MAX_ACTIVE_PASSKEYS - 1];
        let at = [Status::Trusted; MAX_ACTIVE_PASSKEYS];
        assert_eq!(MAX_ACTIVE_PASSKEYS, 16);
        assert_eq!(may_be_added(&[]), Ok(()));
        assert_eq!(may_be_added(&below), Ok(()));
        assert_eq!(may_be_added(&at), Err(Refusal::TooManyPasskeys));
        assert_eq!(
            may_be_added(&[Status::Trusted; MAX_ACTIVE_PASSKEYS + 1]),
            Err(Refusal::TooManyPasskeys)
        );
    }

    #[test]
    fn revoked_passkeys_do_not_count_toward_the_most_a_tenant_may_hold() {
        let below = [Status::Trusted; MAX_ACTIVE_PASSKEYS - 1];
        let at = [Status::Trusted; MAX_ACTIVE_PASSKEYS];
        assert_eq!(may_be_added(&holding(&below, 100)), Ok(()));
        assert_eq!(may_be_added(&holding(&[], 100)), Ok(()));
        for revoked in [1, 100] {
            assert_eq!(
                may_be_added(&holding(&at, revoked)),
                Err(Refusal::TooManyPasskeys),
                "{revoked}"
            );
        }
    }

    #[test]
    fn pending_and_trusted_passkeys_count_alike_toward_the_most_a_tenant_may_hold() {
        for trusted in 0..=MAX_ACTIVE_PASSKEYS {
            let mut at = vec![Status::Trusted; trusted];
            at.resize(MAX_ACTIVE_PASSKEYS, Status::Pending);
            assert_eq!(
                may_be_added(&at),
                Err(Refusal::TooManyPasskeys),
                "{trusted}"
            );
            at.pop();
            assert_eq!(may_be_added(&at), Ok(()), "{trusted}");
        }
    }

    #[test]
    fn only_a_trusted_passkey_may_sign_in() {
        assert!(may_sign_in(Status::Trusted));
        assert!(!may_sign_in(Status::Pending));
        assert!(!may_sign_in(Status::Revoked));
    }

    #[test]
    fn a_trusted_passkey_endorses_a_pending_one() {
        let keys = [key("a", Status::Trusted), key("b", Status::Pending)];
        assert_eq!(may_endorse(&keys, "b", "a"), Ok(()));
    }

    #[test]
    fn only_a_pending_passkey_of_the_tenant_may_be_endorsed() {
        let keys = [
            key("a", Status::Trusted),
            key("b", Status::Pending),
            key("c", Status::Revoked),
        ];
        assert_eq!(may_be_endorsed(&keys, "b"), Ok(()));
        assert_eq!(may_be_endorsed(&keys, "a"), Err(Refusal::NotPending));
        assert_eq!(may_be_endorsed(&keys, "c"), Err(Refusal::NotPending));
        assert_eq!(
            may_be_endorsed(&keys, "missing"),
            Err(Refusal::UnknownCredential)
        );
    }

    #[test]
    fn endorsing_an_unknown_passkey_is_refused() {
        let keys = [key("a", Status::Trusted)];
        for asserting in ["a", "missing"] {
            assert_eq!(
                may_endorse(&keys, "missing", asserting),
                Err(Refusal::UnknownCredential),
                "{asserting}"
            );
        }
    }

    #[test]
    fn endorsing_a_passkey_that_is_not_pending_is_refused() {
        let keys = [
            key("a", Status::Trusted),
            key("b", Status::Trusted),
            key("c", Status::Revoked),
        ];
        assert_eq!(may_endorse(&keys, "b", "a"), Err(Refusal::NotPending));
        assert_eq!(may_endorse(&keys, "c", "a"), Err(Refusal::NotPending));
        assert_eq!(may_endorse(&keys, "b", "b"), Err(Refusal::NotPending));
    }

    #[test]
    fn a_pending_passkey_endorsing_itself_is_a_self_endorsement() {
        let keys = [key("a", Status::Trusted), key("b", Status::Pending)];
        assert_eq!(may_endorse(&keys, "b", "b"), Err(Refusal::SelfEndorsement));
    }

    #[test]
    fn an_endorser_that_is_not_trusted_is_refused() {
        let keys = [
            key("a", Status::Trusted),
            key("b", Status::Pending),
            key("c", Status::Pending),
        ];
        assert_eq!(
            may_endorse(&keys, "b", "c"),
            Err(Refusal::EndorserNotTrusted)
        );
        assert_eq!(
            may_endorse(&keys, "b", "missing"),
            Err(Refusal::EndorserNotTrusted)
        );
    }

    #[test]
    fn a_revoked_passkey_cannot_endorse() {
        let keys = [
            key("a", Status::Trusted),
            key("b", Status::Pending),
            key("c", Status::Revoked),
        ];
        assert_eq!(
            may_endorse(&keys, "b", "c"),
            Err(Refusal::EndorserNotTrusted)
        );
    }

    #[test]
    fn a_trusted_passkey_revokes_another_trusted_one() {
        let keys = [key("a", Status::Trusted), key("b", Status::Trusted)];
        assert_eq!(may_revoke(&keys, "b", "a"), Ok(()));
    }

    #[test]
    fn a_pending_passkey_is_revoked_by_the_only_trusted_one() {
        let keys = [key("a", Status::Trusted), key("b", Status::Pending)];
        assert_eq!(may_revoke(&keys, "b", "a"), Ok(()));
    }

    #[test]
    fn a_passkey_may_be_revoked_unless_it_is_unknown_revoked_or_the_only_trusted_one() {
        let keys = [
            key("a", Status::Trusted),
            key("b", Status::Pending),
            key("c", Status::Revoked),
        ];
        assert_eq!(may_be_revoked(&keys, "b"), Ok(()));
        assert_eq!(
            may_be_revoked(&keys, "missing"),
            Err(Refusal::UnknownCredential)
        );
        assert_eq!(may_be_revoked(&keys, "c"), Err(Refusal::AlreadyRevoked));
        assert_eq!(may_be_revoked(&keys, "a"), Err(Refusal::LastPasskey));

        let keys = [key("a", Status::Trusted), key("d", Status::Trusted)];
        assert_eq!(may_be_revoked(&keys, "a"), Ok(()));
        assert_eq!(may_be_revoked(&keys, "d"), Ok(()));
    }

    #[test]
    fn revoking_an_unknown_passkey_is_refused() {
        let keys = [key("a", Status::Trusted), key("b", Status::Trusted)];
        for asserting in ["a", "missing"] {
            assert_eq!(
                may_revoke(&keys, "missing", asserting),
                Err(Refusal::UnknownCredential),
                "{asserting}"
            );
        }
    }

    #[test]
    fn revoking_a_revoked_passkey_is_refused() {
        let keys = [
            key("a", Status::Trusted),
            key("b", Status::Trusted),
            key("c", Status::Revoked),
        ];
        assert_eq!(may_revoke(&keys, "c", "a"), Err(Refusal::AlreadyRevoked));
        assert_eq!(may_revoke(&keys, "c", "c"), Err(Refusal::AlreadyRevoked));
    }

    #[test]
    fn the_only_trusted_passkey_cannot_be_revoked() {
        let keys = [
            key("a", Status::Trusted),
            key("b", Status::Pending),
            key("c", Status::Revoked),
        ];
        assert_eq!(may_revoke(&keys, "a", "a"), Err(Refusal::LastPasskey));
        assert_eq!(may_revoke(&keys, "a", "b"), Err(Refusal::LastPasskey));
    }

    #[test]
    fn with_two_trusted_passkeys_revoking_oneself_is_a_self_revocation() {
        let keys = [key("a", Status::Trusted), key("b", Status::Trusted)];
        assert_eq!(may_revoke(&keys, "a", "a"), Err(Refusal::SelfRevocation));
    }

    #[test]
    fn a_pending_passkey_revoking_itself_is_a_self_revocation() {
        let keys = [key("a", Status::Trusted), key("b", Status::Pending)];
        assert_eq!(may_revoke(&keys, "b", "b"), Err(Refusal::SelfRevocation));
    }

    #[test]
    fn a_revoker_that_is_not_trusted_is_refused() {
        let keys = [
            key("a", Status::Trusted),
            key("b", Status::Trusted),
            key("c", Status::Pending),
            key("d", Status::Revoked),
        ];
        for asserting in ["c", "d", "missing"] {
            assert_eq!(
                may_revoke(&keys, "a", asserting),
                Err(Refusal::RevokerNotTrusted),
                "{asserting}"
            );
        }
    }
}
