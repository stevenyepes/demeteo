//! What the Hub holds between a ceremony's start and its finish.
//!
//! A finish request names its ceremony by id and says nothing else about it.
//! The user, the credential and the challenge inputs are read from the entry
//! held here: a value the browser echoes back is a value the browser chose.

use super::challenge::{
    base64url_encode, ChallengeBinding, RandomSourceFailed, CHALLENGE_TTL_SECS,
};
use super::webauthn::{RegistrationState, SignInState};
use crate::store::pool::ReservedUser;
use crate::store::requests::RequestId;
use crate::store::{InstanceId, UserId};
use demeteo_hub_protocol::Decision;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Live [`Ceremony::Register`] and [`Ceremony::SignIn`] entries, together.
pub const MAX_LIVE_CEREMONIES: usize = 4096;

/// Live session-bound entries held under one signed-in passkey.
pub const MAX_LIVE_PER_HOLDER: usize = 8;

/// Live session-bound entries, over every holder.
pub const MAX_LIVE_SESSION_BOUND: usize = 16_384;

const CEREMONY_ID_LEN: usize = 16;

#[derive(Debug)]
pub enum Ceremony {
    /// A first passkey, for a tenant that exists only once this finishes.
    Register {
        user: ReservedUser,
        label: String,
        registration: RegistrationState,
    },
    /// For no tenant at all: the passkey its finish presents names one.
    SignIn { sign_in: SignInState },
    /// A further passkey, for the tenant whose session started it.
    Add {
        user: UserId,
        /// The credential id of the passkey that signed that session in. It
        /// says whose budget the entry is held under and nothing about who may
        /// finish it: that is `user`, as it is for the two kinds below.
        holder: Vec<u8>,
        label: String,
        registration: RegistrationState,
    },
    Endorse {
        user: UserId,
        holder: Vec<u8>,
        binding: ChallengeBinding,
        pending_credential_id: String,
    },
    Revoke {
        user: UserId,
        holder: Vec<u8>,
        binding: ChallengeBinding,
        target_credential_id: String,
    },
    /// A decision on one instance's gate, between the challenge that asks a
    /// passkey to sign it and the finish that queues it. Every field the
    /// `GateDecision` request is built from is read back from here, the
    /// request id and expiry included, because the instance recomputes
    /// `challenge` from that request's envelope. Like every kind it is held in
    /// process memory on one replica and lost on restart; [`Ceremonies`] says
    /// what that costs.
    Gate {
        user: UserId,
        holder: Vec<u8>,
        instance: InstanceId,
        feature_id: String,
        step_execution_id: String,
        decision: Decision,
        request_id: RequestId,
        expires_at: u64,
        challenge: [u8; 32],
    },
}

/// Whose allowance an entry is held under.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Budget<'a> {
    /// The one every caller shares, a session or none.
    Anonymous,
    /// That of one signed-in passkey, by its credential id.
    Holder(&'a [u8]),
}

impl Ceremony {
    /// No wildcard arm, so that a new kind does not compile until it is given
    /// a budget here. One that defaulted to [`Budget::Anonymous`] would be
    /// refused by a flood of sign-in starts, and one that defaulted to none
    /// would be held without a bound.
    fn budget(&self) -> Budget<'_> {
        match self {
            Ceremony::Register { .. } | Ceremony::SignIn { .. } => Budget::Anonymous,
            Ceremony::Add { holder, .. }
            | Ceremony::Endorse { holder, .. }
            | Ceremony::Revoke { holder, .. }
            | Ceremony::Gate { holder, .. } => Budget::Holder(holder),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InsertError {
    #[error("too many ceremonies are in flight")]
    Full,
    #[error(transparent)]
    RandomSource(#[from] RandomSourceFailed),
}

struct Held {
    expires_at: u64,
    /// [`Live::inserted`] when this was held: smaller is older.
    order: u64,
    ceremony: Ceremony,
}

impl Held {
    fn is_live(&self, now: u64) -> bool {
        now <= self.expires_at
    }
}

#[derive(Default)]
struct Live {
    entries: HashMap<String, Held>,
    /// How many entries were ever held. `expires_at` is in whole seconds and
    /// cannot say which of two entries of one second is the older.
    inserted: u64,
}

impl Live {
    /// Leaves a place for one more entry under `budget`, or says there is
    /// none. See [`Ceremonies`] for why the two budgets differ.
    fn make_room(&mut self, budget: Budget<'_>) -> Result<(), InsertError> {
        let under_it = || {
            self.entries
                .iter()
                .filter(|(_, entry)| entry.ceremony.budget() == budget)
        };
        match budget {
            Budget::Anonymous if under_it().count() >= MAX_LIVE_CEREMONIES => {
                Err(InsertError::Full)
            }
            Budget::Anonymous => Ok(()),
            Budget::Holder(_) if under_it().count() >= MAX_LIVE_PER_HOLDER => {
                let oldest = under_it()
                    .min_by_key(|(_, entry)| entry.order)
                    .map(|(id, _)| id.clone());
                if let Some(id) = oldest {
                    self.entries.remove(&id);
                }
                Ok(())
            }
            Budget::Holder(_) if self.session_bound() >= MAX_LIVE_SESSION_BOUND => {
                Err(InsertError::Full)
            }
            Budget::Holder(_) => Ok(()),
        }
    }

    fn session_bound(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.ceremony.budget() != Budget::Anonymous)
            .count()
    }
}

/// Kept in process memory and nowhere else. A first registration and a sign-in
/// have no user yet, and a row for either would mean creating a tenant for
/// every ceremony that is started and abandoned.
///
/// What that costs, none of which the code below can say:
///
/// - **What a caller can hold is bounded by who it is.** Anyone can start a
///   registration or a sign-in, so those two kinds share
///   [`MAX_LIVE_CEREMONIES`] and are refused beyond it: that many entries for
///   [`CHALLENGE_TTL_SECS`] is all an anonymous caller can hold. The kinds a
///   session starts are held under the passkey that signed it in,
///   [`MAX_LIVE_PER_HOLDER`] each, and count toward neither the anonymous
///   budget nor another passkey's: a cap shared with the anonymous kinds
///   would let a flood of sign-in starts refuse every revocation for as long
///   as it lasted. Registration is open, so the number of holders is itself
///   something a caller can grow, at one stored tenant each.
///   [`MAX_LIVE_SESSION_BOUND`] is therefore the hard bound over all of them,
///   and a passkey below its own budget is refused once it is reached.
///   Raising the TTL, the anonymous budget or that backstop, or adding a
///   ceremony kind with a large entry, raises the memory bound; raising the
///   per-holder budget lowers the number of passkeys it takes to reach the
///   backstop.
/// - **Eviction is per holder and never global.** A passkey at its budget
///   loses its own oldest entry to the one it starts, so its latest start is
///   never refused and only its own sessions can displace its ceremonies.
///   Evicting the oldest entry of the map, or of a budget that passkeys share,
///   would let whoever starts fastest cancel everyone else's ceremonies in
///   flight, which is worse than the refusal it replaces. The budget is per
///   passkey and not per tenant for the same reason: the caller a revocation
///   is aimed at holds a session of that very tenant.
/// - **It is lost on restart.** A ceremony in flight then finishes as an
///   unknown one, and the user starts it again.
/// - **It is wrong for more than one `hub` replica.** A finish that reaches a
///   replica other than the one that served the start finds nothing. Compose
///   runs one; a second needs this state shared first.
///
/// Every method is synchronous and returns owned values, so the lock is
/// released before a caller can reach an `.await`.
#[derive(Default)]
pub struct Ceremonies {
    held: Mutex<Live>,
}

impl Ceremonies {
    /// Holds `ceremony` until [`CHALLENGE_TTL_SECS`] after `now` (Unix
    /// seconds) and returns the id that takes it back: 16 random bytes as
    /// base64url without padding. Entries that expired before `now` are
    /// dropped first and count toward no budget.
    pub fn insert(&self, now: u64, ceremony: Ceremony) -> Result<String, InsertError> {
        let mut id = [0u8; CEREMONY_ID_LEN];
        aws_lc_rs::rand::fill(&mut id).map_err(|_| RandomSourceFailed)?;
        let id = base64url_encode(&id);

        let mut live = self.lock();
        live.entries.retain(|_, entry| entry.is_live(now));
        live.make_room(ceremony.budget())?;
        live.inserted += 1;
        let held = Held {
            expires_at: now.saturating_add(CHALLENGE_TTL_SECS),
            order: live.inserted,
            ceremony,
        };
        live.entries.insert(id.clone(), held);
        Ok(id)
    }

    /// The ceremony held under `id`, once. `None` for an id that was never
    /// issued, was already taken, or whose entry expired before `now` — and an
    /// expired entry is removed all the same.
    pub fn take(&self, now: u64, id: &str) -> Option<Ceremony> {
        let entry = self.lock().entries.remove(id)?;
        entry.is_live(now).then_some(entry.ceremony)
    }

    /// How many entries are held under the passkey whose credential id is
    /// `holder`, expired ones included until an `insert` drops them. What a
    /// test reads to show a refusal held nothing.
    #[cfg(any(test, feature = "test-support"))]
    pub fn held_under(&self, holder: &[u8]) -> usize {
        self.lock()
            .entries
            .values()
            .filter(|entry| entry.ceremony.budget() == Budget::Holder(holder))
            .count()
    }

    /// A poisoned lock still guards a whole map: nothing runs under it but
    /// `HashMap` calls and the counting between them, so a panic in another
    /// thread cannot have left an entry half-written. Refusing every ceremony
    /// until restart would be the worse answer.
    fn lock(&self) -> MutexGuard<'_, Live> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::challenge::base64url_decode;
    use crate::auth::webauthn::Verifier;
    use crate::auth::RelyingParty;
    use std::collections::VecDeque;

    const NOW: u64 = 1_700_000_000;
    const MINE: &[u8] = b"my passkey";
    const THEIRS: &[u8] = b"their passkey";

    fn verifier() -> Verifier {
        Verifier::new(&RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        })
        .unwrap()
    }

    fn binding() -> ChallengeBinding {
        ChallengeBinding {
            instance_id: "*".to_owned(),
            request_id: "AAECAwQFBgcICQoLDA0ODw".to_owned(),
            expires_at: NOW + CHALLENGE_TTL_SECS,
        }
    }

    fn sign_in(verifier: &Verifier) -> Ceremony {
        let (_, sign_in) = verifier.start_sign_in().unwrap();
        Ceremony::SignIn { sign_in }
    }

    fn revoke_by(user: UserId, holder: &[u8], target: &str) -> Ceremony {
        Ceremony::Revoke {
            user,
            holder: holder.to_vec(),
            binding: binding(),
            target_credential_id: target.to_owned(),
        }
    }

    fn revoke(target: &str) -> Ceremony {
        revoke_by(UserId::generate(), MINE, target)
    }

    fn endorse_by(holder: &[u8]) -> Ceremony {
        Ceremony::Endorse {
            user: UserId::generate(),
            holder: holder.to_vec(),
            binding: binding(),
            pending_credential_id: "pending".to_owned(),
        }
    }

    fn add_by(verifier: &Verifier, holder: &[u8]) -> Ceremony {
        let user = UserId::generate();
        let (_, registration) = verifier
            .start_registration(user.handle(), "phone", &[])
            .unwrap();
        Ceremony::Add {
            user,
            holder: holder.to_vec(),
            label: "phone".to_owned(),
            registration,
        }
    }

    fn gate_by(holder: &[u8], step_execution_id: &str) -> Ceremony {
        Ceremony::Gate {
            user: UserId::generate(),
            holder: holder.to_vec(),
            instance: InstanceId(uuid::Uuid::new_v4()),
            feature_id: "feature".to_owned(),
            step_execution_id: step_execution_id.to_owned(),
            decision: Decision::Approve,
            request_id: [7; 16],
            expires_at: NOW + CHALLENGE_TTL_SECS,
            challenge: [9; 32],
        }
    }

    fn gated_step(taken: Option<Ceremony>) -> Option<String> {
        match taken? {
            Ceremony::Gate {
                step_execution_id, ..
            } => Some(step_execution_id),
            Ceremony::Revoke { .. }
            | Ceremony::Endorse { .. }
            | Ceremony::Register { .. }
            | Ceremony::SignIn { .. }
            | Ceremony::Add { .. } => None,
        }
    }

    fn is_a_sign_in(taken: Option<Ceremony>) -> bool {
        matches!(taken, Some(Ceremony::SignIn { .. }))
    }

    fn target(taken: Option<Ceremony>) -> Option<String> {
        match taken? {
            Ceremony::Revoke {
                target_credential_id,
                ..
            } => Some(target_credential_id),
            Ceremony::Endorse { .. }
            | Ceremony::Register { .. }
            | Ceremony::SignIn { .. }
            | Ceremony::Add { .. }
            | Ceremony::Gate { .. } => None,
        }
    }

    /// Fills the anonymous budget, and returns the ids it is held under.
    fn fill(ceremonies: &Ceremonies, verifier: &Verifier, now: u64) -> Vec<String> {
        (0..MAX_LIVE_CEREMONIES)
            .map(|_| ceremonies.insert(now, sign_in(verifier)).unwrap())
            .collect()
    }

    #[test]
    fn an_entry_is_returned_to_the_id_it_was_held_under() {
        let ceremonies = Ceremonies::default();
        let first = ceremonies.insert(NOW, revoke("first")).unwrap();
        let second = ceremonies.insert(NOW, revoke("second")).unwrap();

        assert_eq!(
            target(ceremonies.take(NOW, &second)),
            Some("second".to_owned())
        );
        assert_eq!(
            target(ceremonies.take(NOW, &first)),
            Some("first".to_owned())
        );
    }

    #[test]
    fn an_entry_is_taken_once() {
        let ceremonies = Ceremonies::default();
        let id = ceremonies.insert(NOW, revoke("target")).unwrap();

        assert!(ceremonies.take(NOW, &id).is_some());
        assert!(ceremonies.take(NOW, &id).is_none());
    }

    #[test]
    fn an_id_that_was_never_issued_takes_nothing() {
        let ceremonies = Ceremonies::default();
        ceremonies.insert(NOW, revoke("target")).unwrap();

        assert!(ceremonies.take(NOW, "AAAAAAAAAAAAAAAAAAAAAA").is_none());
        assert!(ceremonies.take(NOW, "").is_none());
    }

    #[test]
    fn an_entry_lives_through_the_ttl_and_no_longer() {
        let ceremonies = Ceremonies::default();
        let kept = ceremonies.insert(NOW, revoke("kept")).unwrap();
        let late = ceremonies.insert(NOW, revoke("late")).unwrap();

        assert!(ceremonies.take(NOW + CHALLENGE_TTL_SECS, &kept).is_some());
        assert!(ceremonies
            .take(NOW + CHALLENGE_TTL_SECS + 1, &late)
            .is_none());
    }

    #[test]
    fn taking_an_expired_entry_removes_it() {
        let ceremonies = Ceremonies::default();
        let id = ceremonies.insert(NOW, revoke("target")).unwrap();

        assert!(ceremonies.take(NOW + CHALLENGE_TTL_SECS + 1, &id).is_none());
        assert!(ceremonies.take(NOW, &id).is_none());
    }

    #[test]
    fn the_4097th_live_anonymous_entry_is_refused() {
        let (ceremonies, verifier) = (Ceremonies::default(), verifier());
        fill(&ceremonies, &verifier, NOW);

        assert_eq!(
            ceremonies
                .insert(NOW + CHALLENGE_TTL_SECS, sign_in(&verifier))
                .unwrap_err(),
            InsertError::Full
        );
    }

    #[test]
    fn taking_an_entry_frees_its_place() {
        let (ceremonies, verifier) = (Ceremonies::default(), verifier());
        let ids = fill(&ceremonies, &verifier, NOW);
        assert!(ceremonies.insert(NOW, sign_in(&verifier)).is_err());

        assert!(ceremonies.take(NOW, &ids[0]).is_some());
        assert!(ceremonies.insert(NOW, sign_in(&verifier)).is_ok());
    }

    #[test]
    fn expired_entries_free_capacity() {
        let (ceremonies, verifier) = (Ceremonies::default(), verifier());
        fill(&ceremonies, &verifier, NOW);
        let later = NOW + CHALLENGE_TTL_SECS + 1;

        let id = ceremonies.insert(later, sign_in(&verifier)).unwrap();

        assert!(is_a_sign_in(ceremonies.take(later, &id)));
        assert_eq!(ceremonies.lock().entries.len(), 0);
    }

    #[test]
    fn a_full_anonymous_budget_refuses_no_session_bound_ceremony() {
        let (ceremonies, verifier) = (Ceremonies::default(), verifier());
        let before = ceremonies.insert(NOW, revoke("before")).unwrap();
        fill(&ceremonies, &verifier, NOW);

        let revoking = ceremonies.insert(NOW, revoke("after")).unwrap();
        let endorsing = ceremonies.insert(NOW, endorse_by(MINE)).unwrap();
        let adding = ceremonies.insert(NOW, add_by(&verifier, MINE)).unwrap();

        assert_eq!(
            ceremonies.insert(NOW, sign_in(&verifier)).unwrap_err(),
            InsertError::Full
        );
        assert_eq!(
            target(ceremonies.take(NOW, &before)),
            Some("before".to_owned())
        );
        assert_eq!(
            target(ceremonies.take(NOW, &revoking)),
            Some("after".to_owned())
        );
        assert!(matches!(
            ceremonies.take(NOW, &endorsing),
            Some(Ceremony::Endorse { .. })
        ));
        assert!(matches!(
            ceremonies.take(NOW, &adding),
            Some(Ceremony::Add { .. })
        ));
    }

    /// The anonymous entries are the oldest in the map and the other
    /// passkey's are older than any of `MINE`, so an eviction that looked past
    /// the holder would take one of those first.
    #[test]
    fn a_holder_at_its_budget_loses_its_own_oldest_entry_and_nothing_else() {
        let (ceremonies, verifier) = (Ceremonies::default(), verifier());
        let user = UserId::generate();
        let anonymous: Vec<String> = (0..3)
            .map(|_| ceremonies.insert(NOW, sign_in(&verifier)).unwrap())
            .collect();
        let theirs: Vec<String> = (0..MAX_LIVE_PER_HOLDER)
            .map(|_| ceremonies.insert(NOW, endorse_by(THEIRS)).unwrap())
            .collect();
        let held_as = |target: String| {
            let ceremony = revoke_by(user, MINE, &target);
            (ceremonies.insert(NOW, ceremony).unwrap(), target)
        };
        let mut mine: VecDeque<(String, String)> = (0..MAX_LIVE_PER_HOLDER)
            .map(|place| held_as(format!("within budget {place}")))
            .collect();

        let mut evicted = Vec::new();
        for place in 0..2 * MAX_LIVE_PER_HOLDER {
            mine.push_back(held_as(format!("beyond budget {place}")));
            evicted.extend(mine.pop_front());
        }

        for (id, target) in evicted {
            assert!(ceremonies.take(NOW, &id).is_none(), "{target}");
        }
        assert_eq!(mine.len(), MAX_LIVE_PER_HOLDER);
        for (id, held_target) in mine {
            assert_eq!(target(ceremonies.take(NOW, &id)), Some(held_target));
        }
        for id in theirs {
            assert!(matches!(
                ceremonies.take(NOW, &id),
                Some(Ceremony::Endorse { .. })
            ));
        }
        for id in anonymous {
            assert!(is_a_sign_in(ceremonies.take(NOW, &id)));
        }
    }

    /// Both passkeys are one tenant's: the one a revocation is aimed at may
    /// be signed in to the tenant it is being revoked from.
    #[test]
    fn a_passkey_at_its_budget_takes_no_place_from_another_of_the_same_tenant() {
        let ceremonies = Ceremonies::default();
        let user = UserId::generate();
        let in_flight = ceremonies
            .insert(NOW, revoke_by(user, MINE, "in flight"))
            .unwrap();

        for _ in 0..3 * MAX_LIVE_PER_HOLDER {
            let flooding = revoke_by(user, THEIRS, "flood");
            assert!(ceremonies.insert(NOW, flooding).is_ok());
        }
        let rest: Vec<String> = (1..MAX_LIVE_PER_HOLDER)
            .map(|_| {
                ceremonies
                    .insert(NOW, revoke_by(user, MINE, "mine"))
                    .unwrap()
            })
            .collect();

        assert_eq!(
            target(ceremonies.take(NOW, &in_flight)),
            Some("in flight".to_owned())
        );
        for id in rest {
            assert_eq!(target(ceremonies.take(NOW, &id)), Some("mine".to_owned()));
        }
        assert_eq!(ceremonies.lock().entries.len(), MAX_LIVE_PER_HOLDER);
    }

    /// `kept` is the holder's oldest entry by insertion and its only live one
    /// by the time the last is held, so counting the expired ones evicts it.
    #[test]
    fn a_holders_expired_entries_do_not_count_toward_its_budget() {
        let ceremonies = Ceremonies::default();
        let kept = ceremonies.insert(NOW + 100, revoke("kept")).unwrap();
        for _ in 1..MAX_LIVE_PER_HOLDER {
            ceremonies.insert(NOW, revoke("expires first")).unwrap();
        }
        let later = NOW + CHALLENGE_TTL_SECS + 1;

        let last = ceremonies.insert(later, revoke("last")).unwrap();

        assert_eq!(
            target(ceremonies.take(later, &kept)),
            Some("kept".to_owned())
        );
        assert_eq!(
            target(ceremonies.take(later, &last)),
            Some("last".to_owned())
        );
    }

    /// The backstop is filled in the map itself. Through `insert` every entry
    /// costs a scan of the ones before it, which for this many is seconds.
    #[test]
    fn the_backstop_refuses_a_passkey_below_its_budget_and_no_other_ceremony() {
        let verifier = verifier();
        let holder = |place: usize| (place / MAX_LIVE_PER_HOLDER).to_be_bytes();
        let mut live = Live::default();
        for place in 0..MAX_LIVE_SESSION_BOUND {
            live.inserted += 1;
            let held = Held {
                expires_at: NOW + CHALLENGE_TTL_SECS,
                order: live.inserted,
                ceremony: endorse_by(&holder(place)),
            };
            live.entries.insert(place.to_string(), held);
        }
        let ceremonies = Ceremonies {
            held: Mutex::new(live),
        };

        assert_eq!(
            ceremonies.insert(NOW, endorse_by(MINE)).unwrap_err(),
            InsertError::Full
        );
        assert!(ceremonies.insert(NOW, endorse_by(&holder(0))).is_ok());
        assert!(ceremonies.insert(NOW, sign_in(&verifier)).is_ok());
        assert_eq!(ceremonies.lock().session_bound(), MAX_LIVE_SESSION_BOUND);
    }

    #[test]
    fn a_gate_is_held_under_its_holders_budget() {
        assert!(gate_by(MINE, "step").budget() == Budget::Holder(MINE));
    }

    /// A revocation of the same passkey shares the budget, so a gate held
    /// under the anonymous one or a budget of its own would leave it alive.
    #[test]
    fn gates_beyond_the_holders_budget_evict_its_oldest_entry() {
        let ceremonies = Ceremonies::default();
        let revoking = ceremonies.insert(NOW, revoke("oldest")).unwrap();
        let gates: Vec<String> = (0..MAX_LIVE_PER_HOLDER)
            .map(|place| {
                let gate = gate_by(MINE, &format!("step {place}"));
                ceremonies.insert(NOW, gate).unwrap()
            })
            .collect();
        let theirs = ceremonies.insert(NOW, gate_by(THEIRS, "theirs")).unwrap();

        assert!(ceremonies.take(NOW, &revoking).is_none());
        assert_eq!(ceremonies.lock().entries.len(), MAX_LIVE_PER_HOLDER + 1);
        for (place, id) in gates.iter().enumerate() {
            assert_eq!(
                gated_step(ceremonies.take(NOW, id)),
                Some(format!("step {place}"))
            );
        }
        assert_eq!(
            gated_step(ceremonies.take(NOW, &theirs)),
            Some("theirs".to_owned())
        );
    }

    #[test]
    fn an_id_is_16_fresh_bytes() {
        let ceremonies = Ceremonies::default();
        let first = ceremonies.insert(NOW, revoke("first")).unwrap();
        let second = ceremonies.insert(NOW, revoke("second")).unwrap();

        let decoded = base64url_decode(&first).unwrap();
        assert_eq!(decoded.len(), 16);
        assert_ne!(decoded, [0u8; 16]);
        assert_ne!(first, second);
    }

    #[test]
    fn a_poisoned_lock_still_serves_ceremonies() {
        let ceremonies = Ceremonies::default();
        let id = ceremonies.insert(NOW, revoke("target")).unwrap();

        let poisoned = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = ceremonies.held.lock();
                    panic!("poison the lock");
                })
                .join()
        });
        assert!(poisoned.is_err());
        assert!(ceremonies.held.is_poisoned());

        assert!(ceremonies.take(NOW, &id).is_some());
        assert!(ceremonies.insert(NOW, revoke("after")).is_ok());
    }
}
