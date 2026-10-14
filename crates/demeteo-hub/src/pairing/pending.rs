//! What the Hub holds between the steps of one pairing.
//!
//! A consent screen names the request it is for by a `pairing_id`, and a
//! redemption names what was approved by a code. Neither says anything else:
//! the request, the user and the approved scopes are read from the entry held
//! here, because a value the browser or the desktop echoes back is a value it
//! chose.
//!
//! Beside them is something no step of a pairing owns: the DPoP proofs already
//! presented, by a redemption or a refresh, for as long as
//! [`verify`](super::dpop::verify) would take them again.

use super::authorize::AuthorizeRequest;
use super::dpop::VerifiedProof;
use super::{
    AUTHORIZATION_CODE_TTL_SECS, CONSENT_TTL_SECS, DPOP_MAX_AGE_SECS, MAX_PENDING,
    MAX_PENDING_PER_USER, MAX_REMEMBERED_JTIS,
};
use crate::auth::challenge::{base64url_encode, RandomSourceFailed};
use crate::store::tokens::TokenHash;
use crate::store::UserId;
use demeteo_hub_protocol::Scope;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

const PAIRING_ID_LEN: usize = 32;

/// What a user approved, held until the code it was answered with is
/// redeemed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedPairing {
    pub user: UserId,
    pub request: AuthorizeRequest,
    /// What the user left checked: `request.scopes` is what was asked for.
    pub scopes: Vec<Scope>,
    /// When the user approved (Unix seconds), which is where
    /// [`redeem`](super::grant::redeem) counts the code's lifetime from.
    pub approved_at: u64,
}

/// A request parked for its consent screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedConsent {
    pub pairing_id: String,
    /// The last second (Unix) at which `pairing_id` is still taken.
    pub expires_at: u64,
}

/// A bound was reached and nothing held under it had expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("too many pairings are pending")]
pub struct TooManyPending;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParkError {
    #[error(transparent)]
    Full(#[from] TooManyPending),
    #[error(transparent)]
    RandomSource(#[from] RandomSourceFailed),
}

/// Why a proof that verified is not to be acted on. The two are apart because
/// they are different answers to give: `Replayed` is about the proof, and a
/// caller who makes a new one is served; `Full` is about the Hub, which would
/// have taken no proof at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RememberRefusal {
    #[error("this proof was already presented")]
    Replayed,
    #[error("too many proofs are remembered")]
    Full,
}

struct Held<T> {
    expires_at: u64,
    value: T,
}

impl<T> Held<T> {
    fn is_live(&self, now: u64) -> bool {
        now <= self.expires_at
    }
}

struct Consent {
    user: UserId,
    request: AuthorizeRequest,
}

#[derive(Default)]
struct Pending {
    consents: HashMap<String, Held<Consent>>,
    codes: HashMap<TokenHash, Held<ApprovedPairing>>,
    /// Keyed by `(jkt, jti)`.
    proofs: HashMap<(String, String), Held<()>>,
}

/// Kept in process memory and nowhere else, as
/// [`Ceremonies`](crate::auth::ceremonies::Ceremonies) is and at the costs
/// listed there: an entry is lost on restart, and a second `hub` replica does
/// not see it. A pairing in flight then answers as an unknown one and is
/// started again from the desktop.
///
/// Where this differs from that store:
///
/// - **It shares nothing with it.** A pending consent costs a session and a
///   request anyone can build, so a map shared with the ceremonies would let
///   pairing starts refuse sign-ins. The maps here are apart from each other
///   for the same reason: [`MAX_PENDING`] bounds consents and codes each,
///   [`MAX_REMEMBERED_JTIS`] bounds proofs, and a full one refuses nothing in
///   another.
/// - **A user at the bound is refused, not relieved of the oldest entry.**
///   The ceremony store evicts a holder's own oldest so that the latest start
///   always wins. Here the oldest entry may be the consent screen the user is
///   reading, which a start in another tab would turn into an expired one.
///   The bound is also per tenant, where that store keeps a tenant's passkeys
///   apart: one session holding [`MAX_PENDING_PER_USER`] consents refuses the
///   tenant's other sessions until one is answered or expires.
/// - **Codes have no per-user bound.** One is parked only by an approval,
///   which spends a consent, but nothing limits how fast one session starts
///   and approves. A user who parks [`MAX_PENDING`] codes within
///   [`AUTHORIZATION_CODE_TTL_SECS`] refuses every other approval until they
///   expire.
/// - **A full proof window refuses every proof, whoever made it.** A proof is
///   remembered under the key it carries, and a key costs its maker nothing,
///   so a bound per key would bound no one. [`MAX_REMEMBERED_JTIS`] proofs
///   that verify within one window refuse every redemption and refresh until
///   they leave it. Forgetting one early to make room would be agreeing to
///   take it again.
///
/// Every method is synchronous and returns owned values, so the lock is
/// released before a caller can reach an `.await`.
pub struct Pairings {
    held: Mutex<Pending>,
    max_proofs: usize,
}

impl Default for Pairings {
    fn default() -> Self {
        Self::remembering(MAX_REMEMBERED_JTIS)
    }
}

impl Pairings {
    fn remembering(max_proofs: usize) -> Self {
        Self {
            held: Mutex::default(),
            max_proofs,
        }
    }

    /// Holds `request` for `user` until [`CONSENT_TTL_SECS`] after `now`
    /// (Unix seconds) under a fresh `pairing_id`: 32 random bytes as base64url
    /// without padding. Consents that expired before `now` are dropped first
    /// and count toward neither bound.
    pub fn park_consent(
        &self,
        user: UserId,
        request: AuthorizeRequest,
        now: u64,
    ) -> Result<ParkedConsent, ParkError> {
        let mut id = [0u8; PAIRING_ID_LEN];
        aws_lc_rs::rand::fill(&mut id).map_err(|_| RandomSourceFailed)?;
        let pairing_id = base64url_encode(&id);

        let mut pending = self.lock();
        pending.consents.retain(|_, held| held.is_live(now));
        let of_user = pending
            .consents
            .values()
            .filter(|held| held.value.user == user)
            .count();
        if of_user >= MAX_PENDING_PER_USER || pending.consents.len() >= MAX_PENDING {
            return Err(TooManyPending.into());
        }
        let expires_at = now.saturating_add(CONSENT_TTL_SECS);
        let held = Held {
            expires_at,
            value: Consent { user, request },
        };
        pending.consents.insert(pairing_id.clone(), held);
        Ok(ParkedConsent {
            pairing_id,
            expires_at,
        })
    }

    /// The request parked under `pairing_id`, once. `None` for an id that was
    /// never issued, was already taken, expired before `now`, or was parked
    /// by another user — and that user's entry stays where it is, so knowing
    /// an id is not a way to cancel it.
    pub fn take_consent(
        &self,
        user: UserId,
        pairing_id: &str,
        now: u64,
    ) -> Option<AuthorizeRequest> {
        let mut pending = self.lock();
        if pending.consents.get(pairing_id)?.value.user != user {
            return None;
        }
        let held = pending.consents.remove(pairing_id)?;
        held.is_live(now).then_some(held.value.request)
    }

    /// Holds `approved` until [`AUTHORIZATION_CODE_TTL_SECS`] after `now`
    /// under the hash of the code it was answered with. The code itself has
    /// no parameter to arrive in. Codes that expired before `now` are dropped
    /// first.
    pub fn park_code(
        &self,
        code: TokenHash,
        approved: ApprovedPairing,
        now: u64,
    ) -> Result<(), TooManyPending> {
        let mut pending = self.lock();
        pending.codes.retain(|_, held| held.is_live(now));
        if pending.codes.len() >= MAX_PENDING {
            return Err(TooManyPending);
        }
        let held = Held {
            expires_at: now.saturating_add(AUTHORIZATION_CODE_TTL_SECS),
            value: approved,
        };
        pending.codes.insert(code, held);
        Ok(())
    }

    /// What was approved under `code`, once. The entry is removed before
    /// anything about the redemption is checked, so one that goes on to fail
    /// has burned the code: a second attempt with a corrected verifier or
    /// proof finds nothing.
    pub fn take_code(&self, code: &TokenHash, now: u64) -> Option<ApprovedPairing> {
        let held = self.lock().codes.remove(code)?;
        held.is_live(now).then_some(held.value)
    }

    /// Remembers a proof [`verify`](super::dpop::verify) accepted at `now`,
    /// or refuses one it was already shown. A `jti` names a proof only under
    /// the key that signed it, so the same one under another thumbprint is
    /// another proof.
    ///
    /// The pair is held through `iat + DPOP_MAX_AGE_SECS`, the last second at
    /// which `verify` takes the proof. That is counted from `iat` and not
    /// from `now`: a proof dated the whole bound ahead of `now` verifies for
    /// twice as long as one dated at it, and an entry counted from `now`
    /// would be gone for the second half.
    ///
    /// Proofs past their window are dropped first. A replay is answered
    /// before the bound is, so a full window still calls one a replay.
    pub fn remember_proof(&self, proof: &VerifiedProof, now: u64) -> Result<(), RememberRefusal> {
        let mut pending = self.lock();
        pending.proofs.retain(|_, held| held.is_live(now));
        let full = pending.proofs.len() >= self.max_proofs;
        match pending.proofs.entry((proof.jkt.clone(), proof.jti.clone())) {
            Entry::Occupied(_) => Err(RememberRefusal::Replayed),
            Entry::Vacant(_) if full => Err(RememberRefusal::Full),
            Entry::Vacant(place) => {
                place.insert(Held {
                    expires_at: proof.iat.saturating_add(DPOP_MAX_AGE_SECS),
                    value: (),
                });
                Ok(())
            }
        }
    }

    /// Poisoning is answered as `Ceremonies` answers it, for the reason given
    /// on its `lock`: nothing runs under this one but `HashMap` calls either.
    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::challenge::base64url_decode;
    use crate::auth::cose::P256Point;
    use crate::pairing::jwk::DeviceJwk;

    const NOW: u64 = 1_700_000_000;
    const KEY: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";
    const OTHER_KEY: &str = "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs";

    fn request(state: &str) -> AuthorizeRequest {
        AuthorizeRequest {
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_owned(),
            redirect_uri: "http://127.0.0.1:49152/callback".to_owned(),
            dpop_jkt: "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I".to_owned(),
            scopes: vec![Scope::Read, Scope::Spend],
            state: state.to_owned(),
            instance_name: "Workstation".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            app_version: "1.4.0".to_owned(),
        }
    }

    fn approved(state: &str) -> ApprovedPairing {
        ApprovedPairing {
            user: UserId::generate(),
            request: request(state),
            scopes: vec![Scope::Read],
            approved_at: NOW,
        }
    }

    fn code(name: &str) -> TokenHash {
        TokenHash::digest(name.as_bytes())
    }

    /// What `dpop::verify` hands back, without the signature it would take to
    /// get one: nothing here reads `jwk`, and no curve point is behind it.
    fn proof(jkt: &str, jti: &str, iat: u64) -> VerifiedProof {
        let point = P256Point {
            x: [0; 32],
            y: [0; 32],
        };
        VerifiedProof {
            jwk: DeviceJwk { point },
            jkt: jkt.to_owned(),
            jti: jti.to_owned(),
            iat,
            ath: None,
        }
    }

    fn park(pairings: &Pairings, user: UserId, state: &str, now: u64) -> String {
        pairings
            .park_consent(user, request(state), now)
            .unwrap()
            .pairing_id
    }

    fn state_of(taken: Option<AuthorizeRequest>) -> Option<String> {
        taken.map(|request| request.state)
    }

    /// Both stores at [`MAX_PENDING`], every entry expiring at `expires_at`
    /// and no user holding [`MAX_PENDING_PER_USER`] consents twice over. They
    /// are filled in the maps themselves: through `park_consent` every entry
    /// costs a scan of the ones before it, which for this many is seconds.
    fn full(expires_at: u64) -> Pairings {
        let mut pending = Pending::default();
        let mut user = UserId::generate();
        for place in 0..MAX_PENDING {
            if place % MAX_PENDING_PER_USER == 0 {
                user = UserId::generate();
            }
            let consent = Consent {
                user,
                request: request("filler"),
            };
            pending.consents.insert(
                place.to_string(),
                Held {
                    expires_at,
                    value: consent,
                },
            );
            pending.codes.insert(
                code(&place.to_string()),
                Held {
                    expires_at,
                    value: approved("filler"),
                },
            );
        }
        Pairings {
            held: Mutex::new(pending),
            ..Pairings::default()
        }
    }

    #[test]
    fn a_consent_is_returned_to_the_user_and_id_it_was_parked_under() {
        let pairings = Pairings::default();
        let (mine, theirs) = (UserId::generate(), UserId::generate());
        let first = park(&pairings, mine, "first", NOW);
        let second = park(&pairings, theirs, "second", NOW);

        assert_eq!(
            pairings.take_consent(theirs, &second, NOW),
            Some(request("second"))
        );
        assert_eq!(
            pairings.take_consent(mine, &first, NOW),
            Some(request("first"))
        );
    }

    #[test]
    fn a_parked_consent_expires_one_consent_ttl_after_it_was_parked() {
        let pairings = Pairings::default();

        let parked = pairings
            .park_consent(UserId::generate(), request("state"), NOW)
            .unwrap();

        assert_eq!(parked.expires_at, NOW + CONSENT_TTL_SECS);
    }

    #[test]
    fn a_consent_lives_through_the_ttl_and_no_longer() {
        let pairings = Pairings::default();
        let user = UserId::generate();
        let kept = park(&pairings, user, "kept", NOW);
        let late = park(&pairings, user, "late", NOW);

        assert_eq!(
            state_of(pairings.take_consent(user, &kept, NOW + CONSENT_TTL_SECS)),
            Some("kept".to_owned())
        );
        assert!(pairings
            .take_consent(user, &late, NOW + CONSENT_TTL_SECS + 1)
            .is_none());
    }

    #[test]
    fn a_consent_is_taken_once() {
        let pairings = Pairings::default();
        let user = UserId::generate();
        let id = park(&pairings, user, "state", NOW);

        assert!(pairings.take_consent(user, &id, NOW).is_some());
        assert!(pairings.take_consent(user, &id, NOW).is_none());
    }

    #[test]
    fn taking_an_expired_consent_removes_it() {
        let pairings = Pairings::default();
        let user = UserId::generate();
        let id = park(&pairings, user, "state", NOW);

        assert!(pairings
            .take_consent(user, &id, NOW + CONSENT_TTL_SECS + 1)
            .is_none());
        assert!(pairings.take_consent(user, &id, NOW).is_none());
    }

    #[test]
    fn a_pairing_id_that_was_never_issued_takes_nothing() {
        let pairings = Pairings::default();
        let user = UserId::generate();
        park(&pairings, user, "state", NOW);

        let never_issued = base64url_encode(&[0u8; PAIRING_ID_LEN]);
        assert!(pairings.take_consent(user, &never_issued, NOW).is_none());
        assert!(pairings.take_consent(user, "", NOW).is_none());
    }

    #[test]
    fn a_consent_is_nothing_to_another_user_and_still_there_for_its_owner() {
        let pairings = Pairings::default();
        let (mine, theirs) = (UserId::generate(), UserId::generate());
        let id = park(&pairings, mine, "mine", NOW);

        assert!(pairings.take_consent(theirs, &id, NOW).is_none());

        assert_eq!(
            state_of(pairings.take_consent(mine, &id, NOW)),
            Some("mine".to_owned())
        );
    }

    #[test]
    fn the_ninth_pending_consent_of_a_user_is_refused_and_no_other_users() {
        let pairings = Pairings::default();
        let (mine, theirs) = (UserId::generate(), UserId::generate());
        let ids: Vec<String> = (0..MAX_PENDING_PER_USER)
            .map(|place| park(&pairings, mine, &place.to_string(), NOW))
            .collect();

        assert_eq!(
            pairings
                .park_consent(mine, request("ninth"), NOW + CONSENT_TTL_SECS)
                .unwrap_err(),
            ParkError::Full(TooManyPending)
        );
        assert!(pairings
            .park_consent(theirs, request("theirs"), NOW)
            .is_ok());
        for (place, id) in ids.iter().enumerate() {
            assert_eq!(
                state_of(pairings.take_consent(mine, id, NOW)),
                Some(place.to_string())
            );
        }
    }

    #[test]
    fn taking_a_consent_frees_its_place_under_the_user() {
        let pairings = Pairings::default();
        let user = UserId::generate();
        let ids: Vec<String> = (0..MAX_PENDING_PER_USER)
            .map(|_| park(&pairings, user, "state", NOW))
            .collect();
        assert!(pairings.park_consent(user, request("ninth"), NOW).is_err());

        assert!(pairings.take_consent(user, &ids[0], NOW).is_some());
        assert!(pairings.park_consent(user, request("ninth"), NOW).is_ok());
    }

    #[test]
    fn a_users_expired_consents_do_not_count_toward_its_bound() {
        let pairings = Pairings::default();
        let user = UserId::generate();
        for _ in 0..MAX_PENDING_PER_USER {
            park(&pairings, user, "expires", NOW);
        }
        let later = NOW + CONSENT_TTL_SECS + 1;

        let id = park(&pairings, user, "after", later);

        assert_eq!(
            state_of(pairings.take_consent(user, &id, later)),
            Some("after".to_owned())
        );
        assert_eq!(pairings.lock().consents.len(), 0);
    }

    #[test]
    fn a_full_consent_store_refuses_a_user_below_its_bound_until_entries_expire() {
        let pairings = full(NOW);
        let user = UserId::generate();

        assert_eq!(
            pairings
                .park_consent(user, request("refused"), NOW)
                .unwrap_err(),
            ParkError::Full(TooManyPending)
        );
        assert_eq!(pairings.lock().consents.len(), MAX_PENDING);

        let id = park(&pairings, user, "admitted", NOW + 1);
        assert_eq!(
            state_of(pairings.take_consent(user, &id, NOW + 1)),
            Some("admitted".to_owned())
        );
        assert_eq!(pairings.lock().consents.len(), 0);
    }

    #[test]
    fn a_code_is_returned_to_the_hash_it_was_parked_under() {
        let pairings = Pairings::default();
        let (first, second) = (approved("first"), approved("second"));
        pairings
            .park_code(code("first"), first.clone(), NOW)
            .unwrap();
        pairings
            .park_code(code("second"), second.clone(), NOW)
            .unwrap();

        assert_eq!(pairings.take_code(&code("second"), NOW), Some(second));
        assert_eq!(pairings.take_code(&code("first"), NOW), Some(first));
    }

    #[test]
    fn a_code_lives_through_the_ttl_and_no_longer() {
        let pairings = Pairings::default();
        pairings
            .park_code(code("kept"), approved("kept"), NOW)
            .unwrap();
        pairings
            .park_code(code("late"), approved("late"), NOW)
            .unwrap();

        assert!(pairings
            .take_code(&code("kept"), NOW + AUTHORIZATION_CODE_TTL_SECS)
            .is_some());
        assert!(pairings
            .take_code(&code("late"), NOW + AUTHORIZATION_CODE_TTL_SECS + 1)
            .is_none());
    }

    #[test]
    fn a_code_is_taken_once() {
        let pairings = Pairings::default();
        pairings
            .park_code(code("code"), approved("state"), NOW)
            .unwrap();

        assert!(pairings.take_code(&code("code"), NOW).is_some());
        assert!(pairings.take_code(&code("code"), NOW).is_none());
    }

    #[test]
    fn taking_an_expired_code_removes_it() {
        let pairings = Pairings::default();
        pairings
            .park_code(code("code"), approved("state"), NOW)
            .unwrap();

        assert!(pairings
            .take_code(&code("code"), NOW + AUTHORIZATION_CODE_TTL_SECS + 1)
            .is_none());
        assert!(pairings.take_code(&code("code"), NOW).is_none());
    }

    #[test]
    fn a_code_that_was_never_parked_takes_nothing() {
        let pairings = Pairings::default();
        pairings
            .park_code(code("code"), approved("state"), NOW)
            .unwrap();

        assert!(pairings.take_code(&code("another"), NOW).is_none());
    }

    #[test]
    fn a_full_code_store_refuses_until_entries_expire() {
        let pairings = full(NOW);

        assert_eq!(
            pairings.park_code(code("refused"), approved("refused"), NOW),
            Err(TooManyPending)
        );
        assert_eq!(pairings.lock().codes.len(), MAX_PENDING);

        let admitted = approved("admitted");
        pairings
            .park_code(code("admitted"), admitted.clone(), NOW + 1)
            .unwrap();
        assert_eq!(
            pairings.take_code(&code("admitted"), NOW + 1),
            Some(admitted)
        );
        assert_eq!(pairings.lock().codes.len(), 0);
    }

    #[test]
    fn taking_a_code_frees_its_place() {
        let pairings = full(NOW);
        assert!(pairings
            .park_code(code("refused"), approved("refused"), NOW)
            .is_err());

        assert!(pairings.take_code(&code("0"), NOW).is_some());
        assert!(pairings
            .park_code(code("admitted"), approved("admitted"), NOW)
            .is_ok());
    }

    #[test]
    fn a_full_store_of_one_kind_refuses_nothing_of_the_other() {
        let consents_full = full(NOW);
        consents_full.lock().codes.clear();
        assert!(consents_full
            .park_code(code("code"), approved("state"), NOW)
            .is_ok());

        let codes_full = full(NOW);
        codes_full.lock().consents.clear();
        assert!(codes_full
            .park_consent(UserId::generate(), request("state"), NOW)
            .is_ok());
    }

    #[test]
    fn the_same_proof_a_second_time_is_a_replay() {
        let pairings = Pairings::default();
        let presented = proof(KEY, "jti-0", NOW);

        assert_eq!(pairings.remember_proof(&presented, NOW), Ok(()));
        assert_eq!(
            pairings.remember_proof(&presented, NOW),
            Err(RememberRefusal::Replayed)
        );
        assert_eq!(
            pairings.remember_proof(&presented, NOW + 1),
            Err(RememberRefusal::Replayed)
        );
    }

    #[test]
    fn the_same_jti_under_another_key_is_another_proof() {
        let pairings = Pairings::default();
        assert_eq!(
            pairings.remember_proof(&proof(KEY, "jti-0", NOW), NOW),
            Ok(())
        );

        assert_eq!(
            pairings.remember_proof(&proof(OTHER_KEY, "jti-0", NOW), NOW),
            Ok(())
        );
        assert_eq!(
            pairings.remember_proof(&proof(KEY, "jti-1", NOW), NOW),
            Ok(())
        );
        assert_eq!(
            pairings.remember_proof(&proof(KEY, "jti-0", NOW), NOW),
            Err(RememberRefusal::Replayed)
        );
    }

    #[test]
    fn a_proof_is_remembered_through_the_last_second_it_verifies_and_no_longer() {
        let pairings = Pairings::default();
        let presented = proof(KEY, "jti-0", NOW);
        assert_eq!(pairings.remember_proof(&presented, NOW), Ok(()));

        assert_eq!(
            pairings.remember_proof(&presented, NOW + DPOP_MAX_AGE_SECS),
            Err(RememberRefusal::Replayed)
        );
        assert_eq!(pairings.lock().proofs.len(), 1);

        assert_eq!(
            pairings.remember_proof(&proof(KEY, "jti-1", NOW + 1), NOW + DPOP_MAX_AGE_SECS + 1),
            Ok(())
        );
        let pending = pairings.lock();
        let held: Vec<&(String, String)> = pending.proofs.keys().collect();
        assert_eq!(held, [&(KEY.to_owned(), "jti-1".to_owned())]);
    }

    #[test]
    fn a_proof_is_remembered_from_its_iat_and_not_from_when_it_was_shown() {
        let pairings = Pairings::default();
        let ahead = proof(KEY, "ahead", NOW + DPOP_MAX_AGE_SECS);
        let behind = proof(KEY, "behind", NOW - DPOP_MAX_AGE_SECS);
        assert_eq!(pairings.remember_proof(&ahead, NOW), Ok(()));
        assert_eq!(pairings.remember_proof(&behind, NOW), Ok(()));

        assert_eq!(
            pairings.remember_proof(&behind, NOW),
            Err(RememberRefusal::Replayed)
        );
        assert_eq!(
            pairings.remember_proof(&ahead, NOW + 2 * DPOP_MAX_AGE_SECS),
            Err(RememberRefusal::Replayed)
        );
        let held = |pairings: &Pairings| -> Vec<String> {
            let pending = pairings.lock();
            pending.proofs.keys().map(|(_, jti)| jti.clone()).collect()
        };
        assert_eq!(held(&pairings), ["ahead"]);

        let later = NOW + 2 * DPOP_MAX_AGE_SECS + 1;
        assert_eq!(
            pairings.remember_proof(&proof(OTHER_KEY, "later", later), later),
            Ok(())
        );
        assert_eq!(held(&pairings), ["later"]);
    }

    #[test]
    fn a_full_window_refuses_a_new_proof_until_its_entries_expire() {
        let pairings = Pairings::remembering(3);
        for place in 0..3 {
            let jti = place.to_string();
            assert_eq!(pairings.remember_proof(&proof(KEY, &jti, NOW), NOW), Ok(()));
        }
        let fourth = proof(KEY, "fourth", NOW);

        assert_eq!(
            pairings.remember_proof(&fourth, NOW + DPOP_MAX_AGE_SECS),
            Err(RememberRefusal::Full)
        );
        assert_eq!(pairings.lock().proofs.len(), 3);

        let later = NOW + DPOP_MAX_AGE_SECS + 1;
        assert_eq!(
            pairings.remember_proof(&proof(KEY, "fourth", later), later),
            Ok(())
        );
        assert_eq!(pairings.lock().proofs.len(), 1);
    }

    #[test]
    fn a_proof_refused_by_a_full_window_was_not_remembered() {
        let pairings = Pairings::remembering(1);
        assert_eq!(
            pairings.remember_proof(&proof(KEY, "held", NOW), NOW),
            Ok(())
        );
        let refused = proof(KEY, "refused", NOW + 1);

        assert_eq!(
            pairings.remember_proof(&refused, NOW),
            Err(RememberRefusal::Full)
        );
        assert_eq!(
            pairings.remember_proof(&refused, NOW + DPOP_MAX_AGE_SECS + 1),
            Ok(())
        );
    }

    #[test]
    fn a_replay_into_a_full_window_is_still_called_a_replay() {
        let pairings = Pairings::remembering(1);
        let presented = proof(KEY, "jti-0", NOW);
        assert_eq!(pairings.remember_proof(&presented, NOW), Ok(()));

        assert_eq!(
            pairings.remember_proof(&presented, NOW),
            Err(RememberRefusal::Replayed)
        );
        assert_eq!(
            pairings.remember_proof(&proof(OTHER_KEY, "jti-0", NOW), NOW),
            Err(RememberRefusal::Full)
        );
    }

    #[test]
    fn the_window_holds_max_remembered_jtis_proofs_where_no_other_bound_is_given() {
        assert_eq!(Pairings::default().max_proofs, MAX_REMEMBERED_JTIS);
    }

    #[test]
    fn a_full_proof_window_refuses_no_consent_and_no_code() {
        let pairings = Pairings::remembering(0);
        assert_eq!(
            pairings.remember_proof(&proof(KEY, "jti-0", NOW), NOW),
            Err(RememberRefusal::Full)
        );

        assert!(pairings
            .park_consent(UserId::generate(), request("state"), NOW)
            .is_ok());
        assert!(pairings
            .park_code(code("code"), approved("state"), NOW)
            .is_ok());
    }

    #[test]
    fn a_pairing_id_is_32_fresh_bytes() {
        let pairings = Pairings::default();
        let user = UserId::generate();
        let first = park(&pairings, user, "first", NOW);
        let second = park(&pairings, user, "second", NOW);

        let decoded = base64url_decode(&first).unwrap();
        assert_eq!(decoded.len(), 32);
        assert_ne!(decoded, [0u8; 32]);
        assert_ne!(first, second);
    }

    #[test]
    fn a_poisoned_lock_still_serves_pairings() {
        let pairings = Pairings::default();
        let user = UserId::generate();
        let id = park(&pairings, user, "state", NOW);

        let poisoned = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = pairings.held.lock();
                    panic!("poison the lock");
                })
                .join()
        });
        assert!(poisoned.is_err());
        assert!(pairings.held.is_poisoned());

        assert!(pairings.take_consent(user, &id, NOW).is_some());
        assert!(pairings
            .park_code(code("code"), approved("state"), NOW)
            .is_ok());
    }
}
