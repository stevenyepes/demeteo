//! What the Hub holds between a runner's `POST /device_authorization` and
//! the token poll that ends it (`docs/HUB.md` §4, *Pairing a runner by device
//! code*).
//!
//! A pairing here is found two ways, and each is a different trust. The
//! `device_code` is the runner's: 32 random bytes held by their hash, which
//! only the runner was sent and only a proof by its key can poll with. The
//! `user_code` is the human's: eight letters short enough to type, which
//! makes it short enough to guess. Every lookup by it that finds nothing is
//! counted against the tenant that made it, and a tenant at
//! [`MAX_DEVICE_LOOKUP_MISSES`] is refused every lookup, a right one
//! included, until its window ends: were the right one still served, the
//! bound would only slow a guesser down.
//!
//! Unlike [`Pending`](super::pending), nothing here belongs to a user until it
//! is approved. Any signed-in tenant can look a code up and the first answer
//! wins, which is the risk decision 64 accepts; the passkey assertion the
//! approve route requires and the fingerprint the page shows are what stand
//! against it.
//!
//! Kept in process memory under the costs [`Pairings`](super::pending::Pairings)
//! lists: a restart loses every pairing in flight, and the runner's next poll
//! is `invalid_grant`.

use super::authorize::PairingAsk;
use super::grant::AuthorizationCode;
use super::pending::{ParkError, TooManyPending};
use super::{
    DEVICE_CODE_TTL_SECS, DEVICE_LOOKUP_WINDOW_SECS, DEVICE_POLL_INTERVAL_SECS,
    MAX_DEVICE_LOOKUP_MISSES, MAX_PENDING, MAX_PENDING_DEVICE_CODES,
};
use crate::auth::challenge::RandomSourceFailed;
use crate::store::tokens::TokenHash;
use crate::store::UserId;
use aws_lc_rs::constant_time::verify_slices_are_equal;
use demeteo_hub_protocol::Scope;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Consonants only, with no vowel to spell a word and none of the letters
/// most often misread for another (RFC 8628 §6.1). Twenty letters, eight of
/// them: 20⁸ codes, about 34.6 bits.
pub const USER_CODE_ALPHABET: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";
const USER_CODE_LEN: usize = 8;
const USER_CODE_GROUP: usize = 4;
/// The largest multiple of the alphabet's size a byte holds. A byte at or
/// above it is drawn again, so that every letter is equally likely.
const UNBIASED_BELOW: u8 = 240;
/// A fresh code colliding with a held one is drawn again, this many times at
/// most. At the bound one draw collides with odds of 1 in six million, so
/// running out is a random source that is not random.
const USER_CODE_DRAWS: usize = 8;

/// A `user_code` in its one canonical form: eight letters of
/// [`USER_CODE_ALPHABET`], no separator.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserCode(String);

impl UserCode {
    fn generate() -> Result<Self, RandomSourceFailed> {
        let mut code = String::with_capacity(USER_CODE_LEN);
        while code.len() < USER_CODE_LEN {
            let mut bytes = [0u8; USER_CODE_LEN];
            aws_lc_rs::rand::fill(&mut bytes).map_err(|_| RandomSourceFailed)?;
            for byte in bytes.into_iter().filter(|byte| *byte < UNBIASED_BELOW) {
                if code.len() < USER_CODE_LEN {
                    let letter = USER_CODE_ALPHABET[usize::from(byte) % USER_CODE_ALPHABET.len()];
                    code.push(char::from(letter));
                }
            }
        }
        Ok(Self(code))
    }

    /// What a human typed, read as RFC 8628 §6.1 recommends: any letter case,
    /// with the separator or without it, and surrounding space ignored.
    /// Anything else is no code, and is not counted as a miss: it could not
    /// have matched one.
    pub fn parse(typed: &str) -> Option<Self> {
        let code: String = typed
            .trim()
            .chars()
            .filter(|c| *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let well_formed = code.len() == USER_CODE_LEN
            && code.bytes().all(|byte| USER_CODE_ALPHABET.contains(&byte));
        well_formed.then_some(Self(code))
    }

    /// Two groups of four joined by `-`: how the runner prints it and the
    /// page shows it.
    pub fn shown(&self) -> String {
        let (first, second) = self.0.split_at(USER_CODE_GROUP);
        format!("{first}-{second}")
    }
}

/// What the human answered, or that they have not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceAnswer {
    Pending,
    Approved { user: UserId, scopes: Vec<Scope> },
    Denied,
}

/// A started pairing, as the runner is told it. Deliberately not `Debug`:
/// `device_code` is what the tokens are polled for with.
pub struct StartedDevice {
    pub device_code: AuthorizationCode,
    pub user_code: UserCode,
    pub expires_at: u64,
}

/// A pending pairing as the approval page is shown it. `device` is how the
/// approve route names it again without a second lookup by `user_code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePrompt {
    pub device: TokenHash,
    pub user_code: UserCode,
    pub ask: PairingAsk,
    pub expires_at: u64,
}

/// What an approved pairing hands the token grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedDevice {
    pub user: UserId,
    pub ask: PairingAsk,
    pub scopes: Vec<Scope>,
}

/// The verdict on one poll, in the terms of RFC 8628 §3.5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    Pending,
    SlowDown,
    Denied,
    Expired,
    /// No such code, one already spent, or a proof by another key.
    Unknown,
    Approved(ApprovedDevice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LookupRefusal {
    #[error("no pending pairing has this user code")]
    Unknown,
    #[error("too many lookups found nothing")]
    TooManyMisses,
}

struct DevicePairing {
    ask: PairingAsk,
    user_code: UserCode,
    expires_at: u64,
    last_poll: Option<u64>,
    answer: DeviceAnswer,
}

impl DevicePairing {
    fn is_live(&self, now: u64) -> bool {
        now <= self.expires_at
    }

    fn is_open(&self, now: u64) -> bool {
        self.is_live(now) && self.answer == DeviceAnswer::Pending
    }
}

struct Misses {
    count: u32,
    window_ends: u64,
}

#[derive(Default)]
struct Held {
    pairings: HashMap<TokenHash, DevicePairing>,
    by_user_code: HashMap<UserCode, TokenHash>,
    misses: HashMap<UserId, Misses>,
}

impl Held {
    /// An expired pairing is kept for one lifetime more, so a runner that
    /// polls late is told `expired_token` and not that its code never
    /// existed. It counts toward no bound while it is kept.
    fn sweep(&mut self, now: u64) {
        let forgotten =
            |pairing: &DevicePairing| now > pairing.expires_at.saturating_add(DEVICE_CODE_TTL_SECS);
        self.pairings.retain(|_, pairing| !forgotten(pairing));
        let pairings = &self.pairings;
        self.by_user_code
            .retain(|_, device| pairings.contains_key(device));
        self.misses.retain(|_, misses| now <= misses.window_ends);
    }

    fn remove(&mut self, device: &TokenHash) -> Option<DevicePairing> {
        let pairing = self.pairings.remove(device)?;
        self.by_user_code.remove(&pairing.user_code);
        Some(pairing)
    }

    fn fresh_user_code(&self) -> Result<UserCode, ParkError> {
        for _ in 0..USER_CODE_DRAWS {
            let code = UserCode::generate()?;
            if !self.by_user_code.contains_key(&code) {
                return Ok(code);
            }
        }
        Err(RandomSourceFailed.into())
    }
}

/// Every method is synchronous and returns owned values, so the lock is
/// released before a caller can reach an `.await`.
pub struct DeviceCodes {
    held: Mutex<Held>,
    max_pending: usize,
}

impl Default for DeviceCodes {
    fn default() -> Self {
        Self::bounded(MAX_PENDING_DEVICE_CODES)
    }
}

impl DeviceCodes {
    fn bounded(max_pending: usize) -> Self {
        Self {
            held: Mutex::default(),
            max_pending,
        }
    }

    /// Holds `ask` until [`DEVICE_CODE_TTL_SECS`] after `now` under a fresh
    /// device code and a fresh user code.
    pub fn start(&self, ask: PairingAsk, now: u64) -> Result<StartedDevice, ParkError> {
        let device_code = AuthorizationCode::generate()?;
        let mut held = self.lock();
        held.sweep(now);
        let live = held.pairings.values().filter(|p| p.is_live(now)).count();
        if live >= self.max_pending {
            return Err(TooManyPending.into());
        }
        let user_code = held.fresh_user_code()?;
        let expires_at = now.saturating_add(DEVICE_CODE_TTL_SECS);
        let device = device_code.hash();
        held.by_user_code.insert(user_code.clone(), device.clone());
        let pairing = DevicePairing {
            ask,
            user_code: user_code.clone(),
            expires_at,
            last_poll: None,
            answer: DeviceAnswer::Pending,
        };
        held.pairings.insert(device, pairing);
        Ok(StartedDevice {
            device_code,
            user_code,
            expires_at,
        })
    }

    /// One poll of the token endpoint by a proof made with the key whose
    /// thumbprint is `proof_jkt`.
    ///
    /// A poll by another key is answered as an unknown code and changes
    /// nothing, its poll time included: a device code read off the runner is
    /// worth nothing without its key, and must not be a way to make the
    /// runner's own polls too early. An answer the runner was told, approved,
    /// denied or expired, spends the code.
    pub fn poll(&self, device: &TokenHash, proof_jkt: &str, now: u64) -> Poll {
        let mut held = self.lock();
        let Some(pairing) = held.pairings.get_mut(device) else {
            return Poll::Unknown;
        };
        if !is_bound_to(&pairing.ask.dpop_jkt, proof_jkt) {
            return Poll::Unknown;
        }
        if !pairing.is_live(now) {
            held.remove(device);
            return Poll::Expired;
        }
        let too_soon = pairing
            .last_poll
            .is_some_and(|last| now < last.saturating_add(DEVICE_POLL_INTERVAL_SECS));
        pairing.last_poll = Some(now);
        if too_soon {
            return Poll::SlowDown;
        }
        match pairing.answer {
            DeviceAnswer::Pending => Poll::Pending,
            DeviceAnswer::Denied | DeviceAnswer::Approved { .. } => {
                let Some(answered) = held.remove(device) else {
                    return Poll::Unknown;
                };
                match answered.answer {
                    DeviceAnswer::Approved { user, scopes } => Poll::Approved(ApprovedDevice {
                        user,
                        ask: answered.ask,
                        scopes,
                    }),
                    DeviceAnswer::Denied | DeviceAnswer::Pending => Poll::Denied,
                }
            }
        }
    }

    /// The pending pairing `user_code` names, for `user` to answer. A code
    /// that names none — never issued, expired, or already answered — is a
    /// miss, counted against `user`.
    pub fn lookup(
        &self,
        user: UserId,
        user_code: &UserCode,
        now: u64,
    ) -> Result<DevicePrompt, LookupRefusal> {
        let mut held = self.lock();
        held.sweep(now);
        let misses = held.misses.get(&user).map_or(0, |misses| misses.count);
        if misses >= MAX_DEVICE_LOOKUP_MISSES {
            return Err(LookupRefusal::TooManyMisses);
        }
        let found = held.by_user_code.get(user_code).and_then(|device| {
            let pairing = held.pairings.get(device)?;
            pairing.is_open(now).then(|| DevicePrompt {
                device: device.clone(),
                user_code: pairing.user_code.clone(),
                ask: pairing.ask.clone(),
                expires_at: pairing.expires_at,
            })
        });
        if let Some(prompt) = found {
            return Ok(prompt);
        }
        if !held.misses.contains_key(&user) && held.misses.len() >= MAX_PENDING {
            return Err(LookupRefusal::TooManyMisses);
        }
        let window_ends = now.saturating_add(DEVICE_LOOKUP_WINDOW_SECS);
        held.misses
            .entry(user)
            .or_insert(Misses {
                count: 0,
                window_ends,
            })
            .count += 1;
        Err(LookupRefusal::Unknown)
    }

    /// Records `answer` on the pairing `device` names if it is still open and
    /// still the key `dpop_jkt` names, and says whether it did. The first
    /// answer wins; a second, from any tenant, finds nothing open.
    pub fn answer(
        &self,
        device: &TokenHash,
        dpop_jkt: &str,
        answer: DeviceAnswer,
        now: u64,
    ) -> bool {
        let mut held = self.lock();
        match held.pairings.get_mut(device) {
            Some(pairing)
                if pairing.is_open(now) && is_bound_to(&pairing.ask.dpop_jkt, dpop_jkt) =>
            {
                pairing.answer = answer;
                true
            }
            _ => false,
        }
    }

    /// Poisoning is answered as `Pairings` answers it.
    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn is_bound_to(bound_jkt: &str, proof_jkt: &str) -> bool {
    verify_slices_are_equal(bound_jkt.as_bytes(), proof_jkt.as_bytes()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000;
    const KEY: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";
    const OTHER_KEY: &str = "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs";

    fn ask() -> PairingAsk {
        PairingAsk {
            dpop_jkt: KEY.to_owned(),
            scopes: vec![Scope::Read, Scope::Gates],
            instance_name: "build-01".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            app_version: "1.4.0".to_owned(),
        }
    }

    fn started(devices: &DeviceCodes, now: u64) -> (TokenHash, UserCode) {
        let started = devices.start(ask(), now).unwrap();
        (started.device_code.hash(), started.user_code)
    }

    fn approved(user: UserId) -> DeviceAnswer {
        DeviceAnswer::Approved {
            user,
            scopes: vec![Scope::Read],
        }
    }

    #[test]
    fn a_user_code_is_eight_letters_of_the_alphabet_shown_as_two_groups_of_four() {
        for _ in 0..256 {
            let code = UserCode::generate().unwrap();
            assert_eq!(code.0.len(), 8);
            assert!(code
                .0
                .bytes()
                .all(|byte| USER_CODE_ALPHABET.contains(&byte)));
            let shown = code.shown();
            assert_eq!(shown.len(), 9);
            assert_eq!(&shown[4..5], "-");
            assert_eq!(UserCode::parse(&shown), Some(code));
        }
    }

    #[test]
    fn a_typed_user_code_is_read_in_any_case_with_or_without_its_separator() {
        let code = UserCode("BCDFGHJK".to_owned());
        for typed in [
            "BCDF-GHJK",
            "bcdf-ghjk",
            "bcdfghjk",
            "  BcDf-GhJk \n",
            "BC-DF-GH-JK",
        ] {
            assert_eq!(UserCode::parse(typed), Some(code.clone()), "{typed:?}");
        }
        for typed in [
            "",
            "BCDF-GHJ",
            "BCDF-GHJKL",
            "BCDF GHJK",
            "ABCD-GHJK",
            "BCDF-GHJ1",
            "BCDF_GHJK",
        ] {
            assert_eq!(UserCode::parse(typed), None, "{typed:?}");
        }
    }

    #[test]
    fn a_pairing_is_pending_until_answered_and_an_approval_is_handed_over_once() {
        let devices = DeviceCodes::default();
        let (device, _) = started(&devices, NOW);
        let user = UserId::generate();

        assert_eq!(devices.poll(&device, KEY, NOW), Poll::Pending);
        assert!(devices.answer(&device, KEY, approved(user), NOW + 1));
        let polled = devices.poll(&device, KEY, NOW + DEVICE_POLL_INTERVAL_SECS);

        assert_eq!(
            polled,
            Poll::Approved(ApprovedDevice {
                user,
                ask: ask(),
                scopes: vec![Scope::Read],
            })
        );
        assert_eq!(
            devices.poll(&device, KEY, NOW + 2 * DEVICE_POLL_INTERVAL_SECS),
            Poll::Unknown
        );
    }

    #[test]
    fn a_poll_sooner_than_the_interval_after_the_last_is_slow_down() {
        let devices = DeviceCodes::default();
        let (device, _) = started(&devices, NOW);
        assert_eq!(devices.poll(&device, KEY, NOW), Poll::Pending);

        assert_eq!(
            devices.poll(&device, KEY, NOW + DEVICE_POLL_INTERVAL_SECS - 1),
            Poll::SlowDown
        );
        assert_eq!(
            devices.poll(&device, KEY, NOW + 2 * DEVICE_POLL_INTERVAL_SECS - 2),
            Poll::SlowDown
        );
        assert_eq!(
            devices.poll(&device, KEY, NOW + 3 * DEVICE_POLL_INTERVAL_SECS - 2),
            Poll::Pending
        );
    }

    #[test]
    fn a_poll_by_another_key_is_unknown_and_moves_nothing() {
        let devices = DeviceCodes::default();
        let (device, _) = started(&devices, NOW);
        assert_eq!(devices.poll(&device, KEY, NOW), Poll::Pending);
        assert!(devices.answer(&device, KEY, approved(UserId::generate()), NOW));

        assert_eq!(devices.poll(&device, OTHER_KEY, NOW + 10), Poll::Unknown);
        assert_eq!(devices.poll(&device, OTHER_KEY, NOW + 11), Poll::Unknown);

        assert!(matches!(
            devices.poll(&device, KEY, NOW + DEVICE_POLL_INTERVAL_SECS),
            Poll::Approved(_)
        ));
    }

    #[test]
    fn a_denial_is_told_once_and_spends_the_code() {
        let devices = DeviceCodes::default();
        let (device, _) = started(&devices, NOW);
        assert!(devices.answer(&device, KEY, DeviceAnswer::Denied, NOW));

        assert_eq!(devices.poll(&device, KEY, NOW + 1), Poll::Denied);
        assert_eq!(devices.poll(&device, KEY, NOW + 10), Poll::Unknown);
    }

    #[test]
    fn a_pairing_expires_after_its_ttl_answered_or_not_and_is_told_so_once() {
        let devices = DeviceCodes::default();
        let (pending, _) = started(&devices, NOW);
        let (approved_late, _) = started(&devices, NOW);
        assert!(devices.answer(&approved_late, KEY, approved(UserId::generate()), NOW));
        let last = NOW + DEVICE_CODE_TTL_SECS;

        assert_eq!(devices.poll(&pending, KEY, last), Poll::Pending);
        assert_eq!(
            devices.poll(&pending, KEY, last + DEVICE_POLL_INTERVAL_SECS),
            Poll::Expired
        );
        assert_eq!(devices.poll(&pending, KEY, last + 20), Poll::Unknown);
        assert_eq!(devices.poll(&approved_late, KEY, last + 1), Poll::Expired);
    }

    #[test]
    fn an_expired_pairing_is_remembered_one_ttl_more_and_then_forgotten() {
        let devices = DeviceCodes::default();
        let (late, _) = started(&devices, NOW);
        let (forgotten, _) = started(&devices, NOW);
        let expired_for_a_ttl = NOW + 2 * DEVICE_CODE_TTL_SECS;

        started(&devices, expired_for_a_ttl);
        assert_eq!(devices.poll(&late, KEY, expired_for_a_ttl), Poll::Expired);
        started(&devices, expired_for_a_ttl + 1);
        assert_eq!(
            devices.poll(&forgotten, KEY, expired_for_a_ttl + 1),
            Poll::Unknown
        );
    }

    #[test]
    fn an_answer_lands_only_on_an_open_pairing_for_the_key_it_names() {
        let devices = DeviceCodes::default();
        let (device, _) = started(&devices, NOW);

        assert!(!devices.answer(&device, OTHER_KEY, DeviceAnswer::Denied, NOW));
        assert!(!devices.answer(
            &device,
            KEY,
            DeviceAnswer::Denied,
            NOW + DEVICE_CODE_TTL_SECS + 1
        ));
        assert!(devices.answer(&device, KEY, approved(UserId::generate()), NOW));
        assert!(!devices.answer(&device, KEY, DeviceAnswer::Denied, NOW));
        assert!(matches!(devices.poll(&device, KEY, NOW), Poll::Approved(_)));
    }

    #[test]
    fn a_lookup_finds_an_open_pairing_by_its_user_code_for_any_tenant() {
        let devices = DeviceCodes::default();
        let (device, user_code) = started(&devices, NOW);

        for user in [UserId::generate(), UserId::generate()] {
            let prompt = devices.lookup(user, &user_code, NOW).unwrap();
            assert_eq!(
                prompt,
                DevicePrompt {
                    device: device.clone(),
                    user_code: user_code.clone(),
                    ask: ask(),
                    expires_at: NOW + DEVICE_CODE_TTL_SECS,
                }
            );
        }

        assert!(devices.answer(&device, KEY, DeviceAnswer::Denied, NOW));
        let answered = devices.lookup(UserId::generate(), &user_code, NOW);
        assert_eq!(answered, Err(LookupRefusal::Unknown));
    }

    #[test]
    fn a_tenant_at_the_miss_bound_is_refused_every_lookup_until_its_window_ends() {
        let devices = DeviceCodes::default();
        let (_, user_code) = started(&devices, NOW);
        let (guesser, other) = (UserId::generate(), UserId::generate());
        let wrong = UserCode("ZZZZZZZZ".to_owned());
        for _ in 0..MAX_DEVICE_LOOKUP_MISSES {
            assert_eq!(
                devices.lookup(guesser, &wrong, NOW),
                Err(LookupRefusal::Unknown)
            );
        }

        assert_eq!(
            devices.lookup(guesser, &user_code, NOW),
            Err(LookupRefusal::TooManyMisses)
        );
        assert!(devices.lookup(other, &user_code, NOW).is_ok());
        let window_over = NOW + DEVICE_LOOKUP_WINDOW_SECS + 1;
        let (_, later_code) = started(&devices, window_over);
        assert!(devices.lookup(guesser, &later_code, window_over).is_ok());
    }

    #[test]
    fn a_full_store_refuses_a_start_until_a_pairing_expires() {
        let devices = DeviceCodes::bounded(2);
        started(&devices, NOW);
        started(&devices, NOW);

        assert_eq!(
            devices.start(ask(), NOW + DEVICE_CODE_TTL_SECS).err(),
            Some(ParkError::Full(TooManyPending))
        );
        assert!(devices.start(ask(), NOW + DEVICE_CODE_TTL_SECS + 1).is_ok());
    }

    #[test]
    fn every_started_pairing_has_its_own_codes() {
        let devices = DeviceCodes::default();
        let (first_device, first_code) = started(&devices, NOW);
        let (second_device, second_code) = started(&devices, NOW);

        assert_ne!(first_device, second_device);
        assert_ne!(first_code, second_code);
    }
}
