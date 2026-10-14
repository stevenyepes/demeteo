#![cfg(target_os = "linux")]
//! The Hub's passkey trust rules, run from the desktop package.
//!
//! The harness's Rust gate is `cd src-tauri && cargo test`, which scopes cargo
//! to the `demeteo` package: `demeteo-hub` is built only as a Linux
//! dev-dependency of this one, and none of its own tests run. A test inside
//! the Hub crate can therefore never be seen passing by a Demeteo run, so the
//! behaviour a run has to prove is asserted here. `hub_layout.rs` beside this
//! file does the same for the crate's shape.
//!
//! Every signature is a real one, made by the Hub's software authenticator
//! over the bytes a browser would sign: a refusal asserted here is of an
//! assertion that verifies, never of a hand-set flag. Nothing touches
//! Postgres.

use demeteo_hub::auth::assertion::{self, AssertionError, FLAG_USER_PRESENT, FLAG_USER_VERIFIED};
use demeteo_hub::auth::ceremonies::Ceremony;
use demeteo_hub::auth::chain::{self, ChainError, EndorsementRecord, RevocationRecord, StoredKey};
use demeteo_hub::auth::challenge::{
    base64url_decode, base64url_encode, endorsement_challenge, revocation_challenge,
    ChallengeBinding,
};
use demeteo_hub::auth::handlers::{stored_addition, verified_sign_in, ApiError};
use demeteo_hub::auth::policy::{self, Refusal, Status, MAX_ACTIVE_PASSKEYS};
use demeteo_hub::auth::session::{self, SessionSecret};
use demeteo_hub::auth::testing::{answered, lazy_state, serve, SoftAuthenticator};
use demeteo_hub::auth::webauthn::SignedIn;
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::store::passkeys::{NewPasskey, Passkey, TrustChangeError};
use demeteo_hub::store::{StoreError, UserId};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::future::{ready, Ready};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const NOW: u64 = 1_700_000_000;
const UP_AND_UV: u8 = FLAG_USER_PRESENT | FLAG_USER_VERIFIED;

fn rp() -> RelyingParty {
    RelyingParty {
        rp_id: "hub.example".to_owned(),
        origin: "https://hub.example".to_owned(),
    }
}

/// Origins that are not the relying party's and that only comparing the whole
/// string refuses: each begins with the real one, ends with it, is its
/// beginning, or differs by the scheme alone. They are derived from [`rp`] so
/// that they stay near misses of whatever it names.
fn near_miss_origins() -> [String; 6] {
    let origin = rp().origin;
    let host = origin
        .strip_prefix("https://")
        .expect("the relying party's origin is https");
    [
        format!("{origin}.evil.example"),
        format!("{origin}:8443"),
        format!("{origin}/"),
        format!("http://{host}"),
        format!("https://evil.example/{origin}"),
        origin[..origin.len() - 1].to_owned(),
    ]
}

/// The database URL is parsed and never dialled: a query against this state
/// fails instead of returning rows.
fn hub_state() -> AppState {
    lazy_state("postgres://hub@db.invalid/hub", rp()).expect("a parsable database URL")
}

/// The Hub's real router over `state` on a loopback port, and the base URL it
/// answers at. The web directory does not exist: no route this file drives
/// needs it.
async fn serve_state(state: AppState) -> String {
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a free loopback port");
    let addr = listener.local_addr().expect("the bound address");
    serve(listener, routes::router(state, &web_dir));
    format!("http://{addr}")
}

async fn serve_hub() -> String {
    serve_state(hub_state()).await
}

fn public_key_text(authenticator: &SoftAuthenticator) -> String {
    base64url_encode(&authenticator.public_key_cose())
}

/// Registers `authenticator` the way a tenant's next passkey is: with the
/// status the keys already there give it.
fn admit(keys: &mut Vec<StoredKey>, authenticator: &SoftAuthenticator) -> Status {
    let existing: Vec<Status> = keys.iter().map(|key| key.status).collect();
    let status = policy::admission(&existing);
    keys.push(authenticator.stored(status));
    status
}

/// One tenant: a first passkey, a second registered after it, and the binding
/// the server minted for the one ceremony a test runs.
struct Tenant {
    first: SoftAuthenticator,
    second: SoftAuthenticator,
    keys: Vec<StoredKey>,
    binding: ChallengeBinding,
}

impl Tenant {
    fn new() -> Self {
        let first = SoftAuthenticator::new();
        let second = SoftAuthenticator::new();
        let mut keys = Vec::new();
        admit(&mut keys, &first);
        admit(&mut keys, &second);
        Self {
            first,
            second,
            keys,
            binding: ChallengeBinding::mint(NOW).expect("the system random source works"),
        }
    }

    /// A tenant whose second passkey became trusted the only way one can: the
    /// first endorsed it.
    fn with_both_trusted() -> Self {
        let mut tenant = Self::new();
        tenant
            .endorse_over(&tenant.first, &tenant.challenge())
            .expect("a trusted passkey endorses a pending one");
        for key in &mut tenant.keys {
            key.status = Status::Trusted;
        }
        tenant
    }

    /// The challenge an endorsement of the second passkey is made over.
    fn challenge(&self) -> [u8; 32] {
        endorsement_challenge(
            &self.binding,
            &self.second.credential_id(),
            &public_key_text(&self.second),
        )
        .expect("the endorsement challenge encodes")
    }

    /// Submits `signer`'s user-verified assertion over `challenge` as an
    /// endorsement of the second passkey.
    fn endorse_over(
        &self,
        signer: &SoftAuthenticator,
        challenge: &[u8; 32],
    ) -> Result<EndorsementRecord, ChainError> {
        chain::endorse(
            &rp(),
            NOW,
            &self.keys,
            &self.binding,
            &self.second.credential_id(),
            &signer.assert(&rp(), challenge),
        )
    }

    /// Submits the first passkey's assertion over the right challenge, made
    /// claiming this relying party, origin and flags.
    fn endorse_claiming(
        &self,
        rp_id: &str,
        origin: &str,
        flags: u8,
    ) -> Result<EndorsementRecord, ChainError> {
        chain::endorse(
            &rp(),
            NOW,
            &self.keys,
            &self.binding,
            &self.second.credential_id(),
            &self
                .first
                .assert_with(rp_id, origin, flags, &self.challenge()),
        )
    }

    /// The challenge a revocation of `target` is made over.
    fn revocation_of(&self, target: &SoftAuthenticator) -> [u8; 32] {
        revocation_challenge(&self.binding, &target.credential_id())
            .expect("the revocation challenge encodes")
    }

    /// Submits `signer`'s user-verified assertion over `challenge` as a
    /// revocation of `target`.
    fn revoke_over(
        &self,
        target: &SoftAuthenticator,
        signer: &SoftAuthenticator,
        challenge: &[u8; 32],
    ) -> Result<RevocationRecord, ChainError> {
        chain::revoke(
            &rp(),
            NOW,
            &self.keys,
            &self.binding,
            &target.credential_id(),
            &signer.assert(&rp(), challenge),
        )
    }

    /// Submits the first passkey's assertion over the right challenge as a
    /// revocation of `target`, made claiming this relying party, origin and
    /// flags.
    fn revoke_claiming(
        &self,
        target: &SoftAuthenticator,
        rp_id: &str,
        origin: &str,
        flags: u8,
    ) -> Result<RevocationRecord, ChainError> {
        chain::revoke(
            &rp(),
            NOW,
            &self.keys,
            &self.binding,
            &target.credential_id(),
            &self
                .first
                .assert_with(rp_id, origin, flags, &self.revocation_of(target)),
        )
    }
}

/// Replaces base64url `text` with the encoding of the bytes it holds after
/// `change`.
fn alter_bytes(text: &mut String, change: fn(&mut Vec<u8>)) {
    let mut bytes = base64url_decode(text).expect("an assertion field is base64url");
    change(&mut bytes);
    *text = base64url_encode(&bytes);
}

fn flip_last_bit(bytes: &mut [u8]) {
    *bytes.last_mut().expect("an assertion field is not empty") ^= 0x01;
}

/// One field of a record changed, and what re-verifying it must then answer.
type Alteration<R> = (&'static str, fn(&mut R), AssertionError);

/// The alterations of `record` that re-verifying does not answer as it must,
/// each with what it answered instead. All of them and not the first: a
/// verifier that stops making one check lets through every alteration only
/// that check caught, and the failure should name them all.
fn unnoticed_alterations<R: Clone>(
    kind: &str,
    record: &R,
    reverify: impl Fn(&R) -> Result<(), AssertionError>,
    alterations: &[Alteration<R>],
) -> Vec<String> {
    assert_eq!(reverify(record), Ok(()), "the {kind} as it was made");
    alterations
        .iter()
        .filter_map(|(field, alter, refusal)| {
            let mut altered = record.clone();
            alter(&mut altered);
            let answer = reverify(&altered);
            (answer != Err(*refusal)).then(|| format!("{kind} {field}: {answer:?}"))
        })
        .collect()
}

/// The decision the store is handed with a passkey to add, and what it
/// answers: the row as written, or the decision's refusal.
type Admission = fn(&[Status]) -> Result<Status, Refusal>;
type Added = Ready<Result<Passkey, TrustChangeError<Refusal>>>;

/// The passkeys of one tenant as the one store call [`stored_addition`] makes
/// sees them. Like the store's transaction it decides nothing: it shows the
/// decision it is handed the statuses in `existing` and writes what that
/// answers, so `written` is the status the handler's own decision gave, or
/// `None` when it refused.
struct PasskeyRows {
    user: UserId,
    existing: Vec<Status>,
    written: Cell<Option<Status>>,
}

impl PasskeyRows {
    fn of(user: UserId, existing: &[Status]) -> Self {
        Self {
            user,
            existing: existing.to_vec(),
            written: Cell::new(None),
        }
    }

    /// The store call, for an addition of `key` under `label`. Handed any
    /// other passkey it panics: a row written for a credential the ceremony
    /// did not verify must not pass as the addition asserted below.
    fn adding<'a>(
        &'a self,
        key: &'a SoftAuthenticator,
        label: &'a str,
    ) -> impl FnOnce(NewPasskey, Admission) -> Added + 'a {
        move |new, decide| {
            assert_eq!(
                (
                    base64url_encode(&new.credential_id),
                    &new.public_key,
                    new.label.as_str()
                ),
                (key.credential_id(), &key.public_key_cose(), label),
                "an addition of a passkey this table was never told to expect"
            );
            ready(match decide(&self.existing) {
                Ok(status) => {
                    self.written.set(Some(status));
                    Ok(Passkey {
                        user_id: self.user,
                        credential_id: new.credential_id,
                        public_key: new.public_key,
                        label: new.label,
                        sign_count: new.sign_count,
                        status,
                        credential: new.credential,
                        created_at: SystemTime::now().into(),
                    })
                }
                Err(refusal) => Err(TrustChangeError::Refused(refusal)),
            })
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock set after 1970")
        .as_secs()
}

/// One whole addition of `key` by a tenant whose passkeys have the statuses
/// in `existing`: the ceremony `/v1/passkeys/add/start` holds, finished with
/// `key`'s real registration by the function the `/v1/passkeys/add/finish`
/// handler calls. Returns the status the Hub answered the passkey has, or its
/// refusal as a client is sent it, and the status the row was written with.
///
/// The start route reads the tenant's passkeys from Postgres, so its ceremony
/// is held here directly.
async fn add_while_holding(
    state: &AppState,
    existing: &[Status],
    key: &SoftAuthenticator,
) -> (Result<Status, (u16, serde_json::Value)>, Option<Status>) {
    let user = SoftAuthenticator::new()
        .enrolled(state, Status::Trusted)
        .user_id;
    let label = "a further passkey";
    let (options, registration) = state
        .verifier()
        .start_registration(user.handle(), label, &[])
        .expect("an addition's registration starts");
    let ceremony = Ceremony::Add {
        user,
        holder: b"the session's passkey".to_vec(),
        label: label.to_owned(),
        registration,
    };
    let ceremony_id = state
        .ceremonies()
        .insert(unix_now(), ceremony)
        .expect("the ceremony map has room");
    let finish = serde_json::from_value(serde_json::json!({
        "ceremony_id": ceremony_id,
        "credential": key.create(&rp().origin, &options),
    }))
    .expect("the body of an addition's finish");

    let rows = PasskeyRows::of(user, existing);
    let added = stored_addition(state, user, finish, rows.adding(key, label)).await;
    let outcome = match added {
        Ok(passkey) => Ok(passkey.status),
        Err(refusal) => Err(answered(refusal).await),
    };
    (outcome, rows.written.get())
}

/// The finish route writes to Postgres, so it cannot be driven here.
/// [`stored_addition`] is everything that route does, with the one store
/// call handed in.
#[tokio::test]
async fn a_first_passkey_is_admitted_trusted_and_a_later_one_pending() {
    let state = hub_state();
    let first = SoftAuthenticator::new();
    let second = SoftAuthenticator::new();
    let third = SoftAuthenticator::new();
    let mut keys = Vec::new();

    assert_eq!(admit(&mut keys, &first), Status::Trusted);
    for later in [&second, &third] {
        let existing: Vec<Status> = keys.iter().map(|key| key.status).collect();
        assert_eq!(
            add_while_holding(&state, &existing, later).await,
            (Ok(Status::Pending), Some(Status::Pending)),
            "{existing:?}"
        );
        keys.push(later.stored(Status::Pending));
    }

    // What those statuses mean to the chain, with a genuine signature each
    // way: the first passkey's endorsement of the second is accepted, and the
    // second's of the first is refused because the first was never pending.
    let binding = ChallengeBinding::mint(NOW).expect("the system random source works");
    let over = |subject: &SoftAuthenticator| {
        endorsement_challenge(
            &binding,
            &subject.credential_id(),
            &public_key_text(subject),
        )
        .expect("the endorsement challenge encodes")
    };
    let endorse = |subject: &SoftAuthenticator, signer: &SoftAuthenticator| {
        chain::endorse(
            &rp(),
            NOW,
            &keys,
            &binding,
            &subject.credential_id(),
            &signer.assert(&rp(), &over(subject)),
        )
        .map(|_| ())
    };
    assert_eq!(endorse(&second, &first), Ok(()));
    assert_eq!(
        endorse(&first, &second),
        Err(ChainError::Policy(Refusal::NotPending))
    );
}

#[tokio::test]
async fn a_tenant_holding_sixteen_active_passkeys_is_refused_another_and_none_is_written() {
    let state = hub_state();
    let too_many = serde_json::json!({ "error": "too_many_passkeys" });
    let revoked = [Status::Revoked; 40];
    let mut below = vec![Status::Trusted];
    below.resize(MAX_ACTIVE_PASSKEYS - 1, Status::Pending);
    let at = [below.as_slice(), &[Status::Pending]].concat();
    assert_eq!(at.len(), 16);

    for existing in [below.clone(), [below.as_slice(), &revoked].concat()] {
        assert_eq!(
            add_while_holding(&state, &existing, &SoftAuthenticator::new()).await,
            (Ok(Status::Pending), Some(Status::Pending)),
            "{existing:?}"
        );
    }
    for existing in [
        at.clone(),
        [at.as_slice(), &revoked].concat(),
        vec![Status::Trusted; MAX_ACTIVE_PASSKEYS],
    ] {
        assert_eq!(
            add_while_holding(&state, &existing, &SoftAuthenticator::new()).await,
            (Err((409, too_many.clone())), None),
            "{existing:?}"
        );
    }
}

/// A passkeys table holding `row` and nothing else, as the lookup
/// [`verified_sign_in`] makes of one. Asked for any other credential id it
/// panics: answering `None` there would make a sign-in that looked up the
/// wrong passkey indistinguishable from the refusals asserted below.
fn table_of(row: Passkey) -> impl FnOnce(Vec<u8>) -> Ready<Result<Option<Passkey>, StoreError>> {
    move |credential_id| {
        assert_eq!(
            credential_id, row.credential_id,
            "a lookup of a credential this table was never given"
        );
        ready(Ok(Some(row)))
    }
}

/// One whole sign-in by `key` while its row says `status`: a ceremony started
/// over HTTP as the Hub's page starts one, then finished with `key`'s valid
/// assertion by the function the `/v1/session/finish` handler calls.
async fn sign_in_while(
    state: &AppState,
    base: &str,
    key: &SoftAuthenticator,
    status: Status,
) -> Result<(Passkey, SignedIn), ApiError> {
    let (answered, started) = post_json(base, "/v1/session/start", &serde_json::json!({})).await;
    assert_eq!(answered, 200, "{started}");
    let finish = serde_json::from_value(serde_json::json!({
        "ceremony_id": started["ceremony_id"],
        "credential": key.get(&rp().origin, &started["options"], None),
    }))
    .expect("the body of a sign-in finish");
    verified_sign_in(state, finish, table_of(key.enrolled(state, status))).await
}

/// The finish route reads its passkey from Postgres, so it cannot be driven
/// here. [`verified_sign_in`] is everything that route does before it writes,
/// with the lookup handed in.
#[tokio::test]
async fn a_pending_passkey_may_not_sign_in() {
    let state = hub_state();
    let base = serve_state(state.clone()).await;
    let key = SoftAuthenticator::new();

    // The control: the refusals below are of this same key, whose assertion
    // is as valid each time, so the row's status is all that tells them from
    // it.
    let admitted = sign_in_while(&state, &base, &key, Status::Trusted).await;
    assert_eq!(
        admitted.map(|(passkey, signed_in)| (
            base64url_encode(&passkey.credential_id),
            passkey.status,
            signed_in
        )),
        Ok((
            key.credential_id(),
            Status::Trusted,
            SignedIn {
                sign_count: 0,
                counter_regressed: false
            }
        ))
    );

    for status in [Status::Pending, Status::Revoked] {
        assert_eq!(
            sign_in_while(&state, &base, &key, status).await.err(),
            Some(ApiError::UNAUTHENTICATED),
            "{status:?}"
        );
    }
}

#[test]
fn an_endorsement_signed_by_the_pending_key_itself_is_rejected() {
    let tenant = Tenant::new();
    let challenge = tenant.challenge();
    let assertion = tenant.second.assert(&rp(), &challenge);

    // Nothing is wrong with the assertion: it is the pending key's own valid
    // signature over the very challenge an endorsement of it must be made
    // over. The refusal below is of who signed, not of what was signed.
    assert_eq!(
        assertion::verify(
            &rp(),
            &tenant.second.public_key_cose(),
            &challenge,
            &assertion
        ),
        Ok(0)
    );

    assert_eq!(
        chain::endorse(
            &rp(),
            NOW,
            &tenant.keys,
            &tenant.binding,
            &tenant.second.credential_id(),
            &assertion,
        ),
        Err(ChainError::Policy(Refusal::SelfEndorsement))
    );
}

#[test]
fn a_trusted_passkey_endorses_a_pending_one_over_the_endorsement_challenge() {
    let tenant = Tenant::new();
    let assertion = tenant.first.assert(&rp(), &tenant.challenge());

    let record = chain::endorse(
        &rp(),
        NOW,
        &tenant.keys,
        &tenant.binding,
        &tenant.second.credential_id(),
        &assertion,
    )
    .expect("a trusted passkey endorses a pending one");

    assert_eq!(record.binding, tenant.binding);
    assert_eq!(record.body.new_credential_id, tenant.second.credential_id());
    assert_eq!(record.body.new_public_key, public_key_text(&tenant.second));
    assert_eq!(record.body.assertion, assertion);
    assert_eq!(
        record.body.assertion.credential_id,
        tenant.first.credential_id()
    );
}

#[test]
fn an_assertion_over_another_challenge_does_not_endorse() {
    let tenant = Tenant::new();
    let stranger = SoftAuthenticator::new();
    let pending_id = tenant.second.credential_id();
    let pending_key = public_key_text(&tenant.second);
    let rebound = |change: fn(&mut ChallengeBinding)| {
        let mut binding = tenant.binding.clone();
        change(&mut binding);
        endorsement_challenge(&binding, &pending_id, &pending_key)
    };
    let others = [
        (
            "a different new_public_key",
            endorsement_challenge(&tenant.binding, &pending_id, &public_key_text(&stranger)),
        ),
        (
            "a different new_credential_id",
            endorsement_challenge(&tenant.binding, &stranger.credential_id(), &pending_key),
        ),
        (
            "a different request_id",
            rebound(|binding| binding.request_id.push('A')),
        ),
        (
            "a different expires_at",
            rebound(|binding| binding.expires_at += 1),
        ),
        (
            "a different instance_id",
            rebound(|binding| binding.instance_id.push('x')),
        ),
        (
            "the revocation challenge for the same credential",
            revocation_challenge(&tenant.binding, &pending_id),
        ),
    ];

    for (case, challenge) in others {
        let challenge = challenge.expect("the challenge encodes");
        assert_ne!(challenge, tenant.challenge(), "{case}");
        assert_eq!(
            tenant.endorse_over(&tenant.first, &challenge),
            Err(ChainError::Assertion(AssertionError::WrongChallenge)),
            "{case}"
        );
    }
    assert!(tenant
        .endorse_over(&tenant.first, &tenant.challenge())
        .is_ok());
}

#[test]
fn an_endorsement_record_reverifies_from_its_own_fields() {
    let (record, endorser_public_key, other_public_key) = {
        let tenant = Tenant::new();
        let record = tenant
            .endorse_over(&tenant.first, &tenant.challenge())
            .expect("a trusted passkey endorses a pending one");
        (
            record,
            tenant.first.public_key_cose(),
            tenant.second.public_key_cose(),
        )
    };

    // The tenant's rows, the minted binding and the clock are gone: the
    // record, its signer's key and the relying party are all that is left.
    assert_eq!(
        chain::reverify_endorsement(&rp(), &endorser_public_key, &record),
        Ok(())
    );
    assert_eq!(
        chain::reverify_endorsement(&rp(), &other_public_key, &record),
        Err(AssertionError::BadSignature)
    );
}

#[test]
fn an_endorsement_over_an_expired_binding_is_rejected() {
    let tenant = Tenant::new();
    let assertion = tenant.first.assert(&rp(), &tenant.challenge());
    let at = |now| {
        chain::endorse(
            &rp(),
            now,
            &tenant.keys,
            &tenant.binding,
            &tenant.second.credential_id(),
            &assertion,
        )
        .map(|_| ())
    };

    assert_eq!(at(tenant.binding.expires_at), Ok(()));
    assert_eq!(at(tenant.binding.expires_at + 1), Err(ChainError::Expired));
}

#[test]
fn an_endorsement_without_user_verification_is_rejected() {
    let tenant = Tenant::new();
    assert_eq!(
        tenant.endorse_claiming(&rp().rp_id, &rp().origin, FLAG_USER_PRESENT),
        Err(ChainError::Assertion(AssertionError::UserNotVerified))
    );
}

#[test]
fn an_endorsement_asserted_for_another_rp_id_is_rejected() {
    let tenant = Tenant::new();
    assert_eq!(
        tenant.endorse_claiming("other.example", &rp().origin, UP_AND_UV),
        Err(ChainError::Assertion(AssertionError::WrongRpId))
    );
}

#[test]
fn an_endorsement_asserted_at_another_origin_is_rejected() {
    let tenant = Tenant::new();
    for origin in near_miss_origins() {
        assert_eq!(
            tenant.endorse_claiming(&rp().rp_id, &origin, UP_AND_UV),
            Err(ChainError::Assertion(AssertionError::WrongOrigin)),
            "{origin}"
        );
    }
    assert!(tenant
        .endorse_claiming(&rp().rp_id, &rp().origin, UP_AND_UV)
        .is_ok());
}

#[test]
fn a_revoked_passkey_cannot_endorse() {
    let mut tenant = Tenant::new();
    let revoked = SoftAuthenticator::new();
    tenant.keys.push(revoked.stored(Status::Revoked));

    assert_eq!(
        tenant.endorse_over(&revoked, &tenant.challenge()),
        Err(ChainError::Policy(Refusal::EndorserNotTrusted))
    );
}

#[test]
fn revoking_the_only_trusted_passkey_is_refused() {
    let tenant = Tenant::new();
    let challenge = tenant.revocation_of(&tenant.first);

    // Whoever asserts: the passkey itself, or the tenant's other one, which is
    // pending. Both signatures are valid over the right challenge.
    for signer in [&tenant.first, &tenant.second] {
        assert_eq!(
            tenant.revoke_over(&tenant.first, signer, &challenge),
            Err(ChainError::Policy(Refusal::LastPasskey))
        );
    }

    // The refusal is of that target alone: the same passkey discards the
    // pending one, which leaves the tenant its trusted key.
    assert!(tenant
        .revoke_over(
            &tenant.second,
            &tenant.first,
            &tenant.revocation_of(&tenant.second)
        )
        .is_ok());
}

#[test]
fn a_revocation_asserted_by_the_passkey_being_revoked_is_rejected() {
    let tenant = Tenant::with_both_trusted();
    let challenge = tenant.revocation_of(&tenant.second);
    let assertion = tenant.second.assert(&rp(), &challenge);

    // Nothing is wrong with the assertion: it is a trusted key's own valid
    // signature over the very challenge a revocation of it must be made over,
    // and the tenant would keep a trusted key. The refusal below is of who
    // signed, not of what was signed.
    assert_eq!(
        assertion::verify(
            &rp(),
            &tenant.second.public_key_cose(),
            &challenge,
            &assertion
        ),
        Ok(0)
    );

    assert_eq!(
        chain::revoke(
            &rp(),
            NOW,
            &tenant.keys,
            &tenant.binding,
            &tenant.second.credential_id(),
            &assertion,
        ),
        Err(ChainError::Policy(Refusal::SelfRevocation))
    );
}

#[test]
fn another_trusted_passkey_revokes_over_the_revocation_challenge() {
    let tenant = Tenant::with_both_trusted();
    let assertion = tenant
        .first
        .assert(&rp(), &tenant.revocation_of(&tenant.second));

    let record = chain::revoke(
        &rp(),
        NOW,
        &tenant.keys,
        &tenant.binding,
        &tenant.second.credential_id(),
        &assertion,
    )
    .expect("a trusted passkey revokes another");

    assert_eq!(record.binding, tenant.binding);
    assert_eq!(record.body.credential_id, tenant.second.credential_id());
    assert_eq!(record.body.assertion, assertion);
    assert_eq!(
        record.body.assertion.credential_id,
        tenant.first.credential_id()
    );
}

#[test]
fn an_endorsement_assertion_is_not_accepted_as_a_revocation() {
    let tenant = Tenant::new();
    let endorsement = tenant.challenge();
    let revocation = tenant.revocation_of(&tenant.second);
    assert_ne!(endorsement, revocation);

    // The same trusted signer, the same subject and the same binding: only
    // what the assertion was made over differs.
    assert!(tenant.endorse_over(&tenant.first, &endorsement).is_ok());
    assert_eq!(
        tenant.revoke_over(&tenant.second, &tenant.first, &endorsement),
        Err(ChainError::Assertion(AssertionError::WrongChallenge))
    );
    assert!(tenant
        .revoke_over(&tenant.second, &tenant.first, &revocation)
        .is_ok());
}

#[test]
fn a_revocation_over_an_expired_binding_is_rejected() {
    let tenant = Tenant::with_both_trusted();
    let assertion = tenant
        .first
        .assert(&rp(), &tenant.revocation_of(&tenant.second));
    let at = |now| {
        chain::revoke(
            &rp(),
            now,
            &tenant.keys,
            &tenant.binding,
            &tenant.second.credential_id(),
            &assertion,
        )
        .map(|_| ())
    };

    assert_eq!(at(tenant.binding.expires_at), Ok(()));
    assert_eq!(at(tenant.binding.expires_at + 1), Err(ChainError::Expired));
}

#[test]
fn a_revocation_without_user_verification_is_rejected() {
    let tenant = Tenant::with_both_trusted();
    assert_eq!(
        tenant.revoke_claiming(&tenant.second, &rp().rp_id, &rp().origin, FLAG_USER_PRESENT),
        Err(ChainError::Assertion(AssertionError::UserNotVerified))
    );
}

#[test]
fn a_revocation_asserted_for_another_rp_id_is_rejected() {
    let tenant = Tenant::with_both_trusted();
    assert_eq!(
        tenant.revoke_claiming(&tenant.second, "other.example", &rp().origin, UP_AND_UV),
        Err(ChainError::Assertion(AssertionError::WrongRpId))
    );
}

#[test]
fn a_revocation_asserted_at_another_origin_is_rejected() {
    let tenant = Tenant::with_both_trusted();
    for origin in near_miss_origins() {
        assert_eq!(
            tenant.revoke_claiming(&tenant.second, &rp().rp_id, &origin, UP_AND_UV),
            Err(ChainError::Assertion(AssertionError::WrongOrigin)),
            "{origin}"
        );
    }
    assert!(tenant
        .revoke_claiming(&tenant.second, &rp().rp_id, &rp().origin, UP_AND_UV)
        .is_ok());
}

#[test]
fn a_revocation_record_reverifies_from_its_own_fields() {
    let (record, revoker_public_key, other_public_key) = {
        let tenant = Tenant::with_both_trusted();
        let record = tenant
            .revoke_over(
                &tenant.second,
                &tenant.first,
                &tenant.revocation_of(&tenant.second),
            )
            .expect("a trusted passkey revokes another");
        (
            record,
            tenant.first.public_key_cose(),
            tenant.second.public_key_cose(),
        )
    };

    // As for an endorsement: the record, its signer's key and the relying
    // party are all that is left.
    assert_eq!(
        chain::reverify_revocation(&rp(), &revoker_public_key, &record),
        Ok(())
    );
    assert_eq!(
        chain::reverify_revocation(&rp(), &other_public_key, &record),
        Err(AssertionError::BadSignature)
    );
}

/// Every alteration of a signed field is one the verifier's other checks let
/// through — the signature counter, whitespace after the client data, one bit
/// of the signature — so the signature is the only thing that can refuse it.
/// `assertion.credential_id` is not here: no signature covers it. It names the
/// key to verify under, and that key is passed in.
#[test]
fn a_record_with_any_field_altered_does_not_reverify() {
    let tenant = Tenant::new();
    let signer_public_key = tenant.first.public_key_cose();

    let endorsement = tenant
        .endorse_over(&tenant.first, &tenant.challenge())
        .expect("a trusted passkey endorses a pending one");
    let mut unnoticed = unnoticed_alterations(
        "endorsement",
        &endorsement,
        |record| chain::reverify_endorsement(&rp(), &signer_public_key, record),
        &[
            (
                "instance_id",
                |record| record.binding.instance_id.push('x'),
                AssertionError::WrongChallenge,
            ),
            (
                "request_id",
                |record| record.binding.request_id.push('A'),
                AssertionError::WrongChallenge,
            ),
            (
                "expires_at",
                |record| record.binding.expires_at += 1,
                AssertionError::WrongChallenge,
            ),
            (
                "new_credential_id",
                |record| record.body.new_credential_id.push('A'),
                AssertionError::WrongChallenge,
            ),
            (
                "new_public_key",
                |record| record.body.new_public_key.push('A'),
                AssertionError::WrongChallenge,
            ),
            (
                "authenticator_data",
                |record| {
                    alter_bytes(&mut record.body.assertion.authenticator_data, |bytes| {
                        flip_last_bit(bytes)
                    })
                },
                AssertionError::BadSignature,
            ),
            (
                "client_data_json",
                |record| {
                    alter_bytes(&mut record.body.assertion.client_data_json, |json| {
                        json.push(b' ')
                    })
                },
                AssertionError::BadSignature,
            ),
            (
                "signature",
                |record| {
                    alter_bytes(&mut record.body.assertion.signature, |bytes| {
                        flip_last_bit(bytes)
                    })
                },
                AssertionError::BadSignature,
            ),
        ],
    );

    let revocation = tenant
        .revoke_over(
            &tenant.second,
            &tenant.first,
            &tenant.revocation_of(&tenant.second),
        )
        .expect("a trusted passkey revokes a pending one");
    unnoticed.extend(unnoticed_alterations(
        "revocation",
        &revocation,
        |record| chain::reverify_revocation(&rp(), &signer_public_key, record),
        &[
            (
                "instance_id",
                |record| record.binding.instance_id.push('x'),
                AssertionError::WrongChallenge,
            ),
            (
                "request_id",
                |record| record.binding.request_id.push('A'),
                AssertionError::WrongChallenge,
            ),
            (
                "expires_at",
                |record| record.binding.expires_at += 1,
                AssertionError::WrongChallenge,
            ),
            (
                "credential_id",
                |record| record.body.credential_id.push('A'),
                AssertionError::WrongChallenge,
            ),
            (
                "authenticator_data",
                |record| {
                    alter_bytes(&mut record.body.assertion.authenticator_data, |bytes| {
                        flip_last_bit(bytes)
                    })
                },
                AssertionError::BadSignature,
            ),
            (
                "client_data_json",
                |record| {
                    alter_bytes(&mut record.body.assertion.client_data_json, |json| {
                        json.push(b' ')
                    })
                },
                AssertionError::BadSignature,
            ),
            (
                "signature",
                |record| {
                    alter_bytes(&mut record.body.assertion.signature, |bytes| {
                        flip_last_bit(bytes)
                    })
                },
                AssertionError::BadSignature,
            ),
        ],
    ));

    assert_eq!(unnoticed, Vec::<String>::new());
}

/// Each attribute is matched whole, and `Domain` by name in any case. A
/// substring search is satisfied by the `Secure` of a `__Secure-` name and
/// misses a `domain=`, which a browser reads the same.
#[test]
fn the_session_cookie_is_http_only_secure_and_same_site_strict() {
    let secret = SessionSecret::generate().expect("the system random source");
    let set_cookie = session::set_cookie(&secret);

    let mut parts = set_cookie.split("; ");
    assert_eq!(
        parts.next(),
        Some(format!("{}={}", session::COOKIE_NAME, secret.cookie_value()).as_str())
    );
    let attributes: Vec<&str> = parts.collect();
    for required in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
        assert!(
            attributes.contains(&required),
            "`{required}` is missing from `{attributes:?}`"
        );
    }
    assert!(
        !attributes
            .iter()
            .any(|attribute| attribute.to_ascii_lowercase().starts_with("domain")),
        "a session cookie names no Domain: `{attributes:?}`"
    );
}

/// The `POST` routes that act for a signed-in tenant. Each needs the CSRF
/// token of its session.
///
/// This table and the next list every state-changing route the Hub serves.
/// Their lengths are in their types so that losing a row is a compile error
/// and not a loop that quietly covers one route fewer.
const SESSION_BOUND_POSTS: [&str; 15] = [
    "/v1/session/end",
    "/v1/passkeys/add/start",
    "/v1/passkeys/add/finish",
    "/v1/passkeys/endorse/start",
    "/v1/passkeys/endorse/finish",
    "/v1/passkeys/revoke/start",
    "/v1/passkeys/revoke/finish",
    "/v1/pairing/start",
    "/v1/pairing/approve",
    "/v1/pairing/deny",
    "/v1/instances/{instance_id}/runs",
    "/v1/instances/{instance_id}/runs/{feature_id}/cancel",
    "/v1/instances/{instance_id}/runs/{feature_id}/steps/{step_id}/assignment",
    "/v1/instances/{instance_id}/gates/challenge",
    "/v1/instances/{instance_id}/gates/decision",
];

/// The `POST` routes that run before there is a session to derive a token
/// from. The `Origin` check is all that guards them.
const PRE_SESSION_POSTS: [&str; 4] = [
    "/v1/register/start",
    "/v1/register/finish",
    "/v1/session/start",
    "/v1/session/finish",
];

/// No route serves this path. A `403` is the guard's answer only where a
/// route is mounted, so this one answering anything else is what makes a
/// `403` from a path in the tables above show that its route exists.
const UNMOUNTED_POST: &str = "/v1/no-such-route";

fn session_cookie(secret: &SessionSecret) -> String {
    format!("{}={}", session::COOKIE_NAME, secret.cookie_value())
}

/// The status and the `error` code of a `POST` carrying exactly `headers`.
async fn post(base: &str, path: &str, headers: &[(&str, &str)]) -> (u16, Option<String>) {
    let mut request = reqwest::Client::new().post(format!("{base}{path}"));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await.expect("the loopback listener answers");
    let status = response.status().as_u16();
    let body = response.text().await.expect("a readable body");
    (status, error_code(&body))
}

fn error_code(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(json.get("error")?.as_str()?.to_owned())
}

fn refused(code: &str) -> (u16, Option<String>) {
    (403, Some(code.to_owned()))
}

#[tokio::test]
async fn a_state_changing_request_without_the_csrf_token_is_403() {
    let base = serve_hub().await;
    let origin = rp().origin;
    let secret = SessionSecret::generate().expect("the system random source");
    let cookie = session_cookie(&secret);
    let another_sessions_token = SessionSecret::generate()
        .expect("the system random source")
        .csrf_token();
    let without_token = [("origin", origin.as_str()), ("cookie", cookie.as_str())];
    let with_wrong_token = [
        ("origin", origin.as_str()),
        ("cookie", cookie.as_str()),
        (session::CSRF_HEADER, another_sessions_token.as_str()),
    ];

    for path in SESSION_BOUND_POSTS {
        assert_eq!(
            post(&base, path, &without_token).await,
            refused("csrf_required"),
            "{path} without the token"
        );
        assert_eq!(
            post(&base, path, &with_wrong_token).await,
            refused("csrf_mismatch"),
            "{path} with another session's token"
        );
    }

    for headers in [&without_token[..], &with_wrong_token[..]] {
        let (status, _) = post(&base, UNMOUNTED_POST, headers).await;
        assert_ne!(status, 403, "{UNMOUNTED_POST} is not a route");
    }
}

/// A cookie of another application on the same host is not the Hub's to
/// constrain. Were the guard blind to the session beside it, it would ask
/// nothing of the token and the answer would be the extractor's `401`.
#[tokio::test]
async fn a_session_beside_a_cookie_that_is_not_ascii_is_still_held_to_its_csrf_token() {
    let base = serve_hub().await;
    let secret = SessionSecret::generate().expect("the system random source");
    let cookie = format!("theme=café; {}", session_cookie(&secret));
    let cookie = reqwest::header::HeaderValue::from_bytes(cookie.as_bytes())
        .expect("bytes a header value may hold");
    let another_sessions_token = SessionSecret::generate()
        .expect("the system random source")
        .csrf_token();

    let response = reqwest::Client::new()
        .post(format!("{base}/v1/session/end"))
        .header("origin", rp().origin)
        .header("cookie", cookie)
        .header(session::CSRF_HEADER, another_sessions_token)
        .send()
        .await
        .expect("the loopback listener answers");

    let status = response.status().as_u16();
    let body = response.text().await.expect("a readable body");
    assert_eq!((status, error_code(&body)), refused("csrf_mismatch"));
}

/// A request that is this session's own in everything but where it claims to
/// come from.
fn from_origin<'a>(origin: &'a str, cookie: &'a str, token: &'a str) -> [(&'a str, &'a str); 3] {
    [
        ("origin", origin),
        ("cookie", cookie),
        (session::CSRF_HEADER, token),
    ]
}

#[tokio::test]
async fn a_state_changing_request_from_another_origin_is_403() {
    let base = serve_hub().await;
    let secret = SessionSecret::generate().expect("the system random source");
    let cookie = session_cookie(&secret);
    let token = secret.csrf_token();
    let without_origin = [
        ("cookie", cookie.as_str()),
        (session::CSRF_HEADER, token.as_str()),
    ];
    let mut foreign = vec!["https://evil.example".to_owned()];
    foreign.extend(near_miss_origins());

    let every_post: Vec<&str> = SESSION_BOUND_POSTS
        .into_iter()
        .chain(PRE_SESSION_POSTS)
        .collect();
    let distinct: BTreeSet<&str> = every_post.iter().copied().collect();
    assert_eq!(distinct.len(), 19, "a path is listed twice: {every_post:?}");

    for path in every_post {
        assert_eq!(
            post(&base, path, &without_origin).await,
            refused("origin_mismatch"),
            "{path} with no Origin"
        );
        for origin in &foreign {
            assert_eq!(
                post(&base, path, &from_origin(origin, &cookie, &token)).await,
                refused("origin_mismatch"),
                "{path} from {origin}"
            );
        }
    }

    let (status, _) = post(&base, UNMOUNTED_POST, &without_origin).await;
    assert_ne!(status, 403, "{UNMOUNTED_POST} is not a route");
    for origin in &foreign {
        let headers = from_origin(origin, &cookie, &token);
        let (status, _) = post(&base, UNMOUNTED_POST, &headers).await;
        assert_ne!(status, 403, "{UNMOUNTED_POST} is not a route");
    }
}

/// The status and the body of a `POST` of `body` as the Hub's own page sends
/// it before there is a session: same origin, and nothing else.
async fn post_json(base: &str, path: &str, body: &serde_json::Value) -> (u16, serde_json::Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("origin", rp().origin)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("the loopback listener answers");
    let status = response.status().as_u16();
    let body = response.text().await.expect("a readable body");
    (
        status,
        serde_json::from_str(&body).expect("a JSON body from a /v1 route"),
    )
}

/// The registration is held in memory until its finish, so starting one
/// reaches no database: the one this router has cannot answer.
#[tokio::test]
async fn starting_a_registration_answers_es256_only_options_for_the_configured_rp_id() {
    let base = serve_hub().await;
    let label = serde_json::json!({ "label": "Work laptop" });

    let (status, started) = post_json(&base, "/v1/register/start", &label).await;

    assert_eq!(status, 200, "{started}");
    assert!(
        started["ceremony_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "{started}"
    );
    let options = &started["options"]["publicKey"];
    assert_eq!(options["rp"]["id"], rp().rp_id);
    let offered = options["pubKeyCredParams"]
        .as_array()
        .expect("a `pubKeyCredParams` array");
    assert!(!offered.is_empty(), "{options}");
    for param in offered {
        assert_eq!(param["alg"], -7, "{offered:?}");
    }
    assert_eq!(options["user"]["name"], "Work laptop");
    assert_eq!(options["excludeCredentials"], serde_json::json!([]));
}

/// Neither answer needs a session, a token or a database, so each is the
/// handler's own and not the guard's.
#[tokio::test]
async fn a_registration_is_not_started_without_a_label_nor_finished_without_its_ceremony() {
    let base = serve_hub().await;
    let invalid = (400, serde_json::json!({ "error": "invalid_request" }));

    for label in ["", "   ", &"x".repeat(65), "a\0b"] {
        let body = serde_json::json!({ "label": label });
        assert_eq!(
            post_json(&base, "/v1/register/start", &body).await,
            invalid,
            "{label:?}"
        );
    }

    let credential = SoftAuthenticator::new().create(
        &rp().origin,
        &post_json(
            &base,
            "/v1/register/start",
            &serde_json::json!({ "label": "Work laptop" }),
        )
        .await
        .1["options"],
    );
    let never_started = serde_json::json!({
        "ceremony_id": "AAAAAAAAAAAAAAAAAAAAAA",
        "credential": credential,
    });
    assert_eq!(
        post_json(&base, "/v1/register/finish", &never_started).await,
        (400, serde_json::json!({ "error": "ceremony_expired" }))
    );
}

/// A sign-in names nobody until its finish, so starting one reaches no
/// database either.
#[tokio::test]
async fn starting_a_sign_in_answers_usernameless_options_that_require_user_verification() {
    let base = serve_hub().await;

    let (status, started) = post_json(&base, "/v1/session/start", &serde_json::json!({})).await;

    assert_eq!(status, 200, "{started}");
    assert!(
        started["ceremony_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "{started}"
    );
    let options = started["options"]["publicKey"]
        .as_object()
        .expect("request options under `publicKey`");
    assert!(!options.contains_key("allowCredentials"), "{options:?}");
    assert_eq!(options["userVerification"], "required");
    assert_eq!(options["rpId"], rp().rp_id);
    assert!(
        options["challenge"]
            .as_str()
            .and_then(base64url_decode)
            .is_some_and(|challenge| challenge.len() == 32),
        "{options:?}"
    );
}

/// The ceremony is asked for before the credential is looked up, so this is
/// the handler's own answer and needs no database.
#[tokio::test]
async fn a_sign_in_is_not_finished_without_its_ceremony() {
    let base = serve_hub().await;
    let (_, started) = post_json(&base, "/v1/session/start", &serde_json::json!({})).await;
    let never_started = serde_json::json!({
        "ceremony_id": "AAAAAAAAAAAAAAAAAAAAAA",
        "credential": SoftAuthenticator::new().get(&rp().origin, &started["options"], None),
    });

    assert_eq!(
        post_json(&base, "/v1/session/finish", &never_started).await,
        (400, serde_json::json!({ "error": "ceremony_expired" }))
    );
}

/// The `GET` routes that answer for a signed-in tenant. A `GET` changes
/// nothing, so the guard asks no token of it and the session is its only gate.
const SESSION_BOUND_GETS: &[&str] = &[
    "/v1/session",
    "/v1/passkeys",
    "/v1/fleet",
    "/v1/instances/{instance_id}",
    "/v1/runs",
    "/v1/instances/{instance_id}/runs/{feature_id}",
    "/v1/instances/{instance_id}/requests/{request_id}",
];

/// No request here reaches the database, which cannot answer: a session
/// lookup would turn each `401` into a `500`.
#[tokio::test]
async fn a_request_without_a_session_cookie_is_401_unauthenticated() {
    let base = serve_hub().await;

    for path in SESSION_BOUND_GETS {
        let response = reqwest::get(format!("{base}{path}"))
            .await
            .expect("the loopback listener answers");
        let status = response.status().as_u16();
        let body = response.text().await.expect("a readable body");
        assert_eq!(
            (status, body.as_str()),
            (401, r#"{"error":"unauthenticated"}"#),
            "{path}"
        );
    }

    // A token with no cookie is a missing sign-in, not a forgery: the guard
    // lets it through for the session lookup to refuse.
    let token = SessionSecret::generate()
        .expect("the system random source")
        .csrf_token();
    let origin = rp().origin;
    let signed_out = [
        ("origin", origin.as_str()),
        (session::CSRF_HEADER, token.as_str()),
    ];
    assert_eq!(
        post(&base, "/v1/session/end", &signed_out).await,
        (401, Some("unauthenticated".to_owned()))
    );
}

#[tokio::test]
async fn healthz_answers_200_on_the_real_router_with_no_postgres() {
    let base = serve_hub().await;

    let response = reqwest::get(format!("{base}/healthz"))
        .await
        .expect("the loopback listener answers");

    assert_eq!(response.status().as_u16(), 200);
}

/// Every `"/v1/…"` string literal in `source` before its unit tests, which
/// name paths no route serves. The literal and not the `.route(` call:
/// rustfmt puts the path of a long call on a line of its own. An axum
/// `Router` cannot be asked what it serves, so the source is.
fn v1_paths_named(source: &str) -> BTreeSet<&str> {
    let served = source
        .split_once("#[cfg(test)]")
        .map_or(source, |(served, _)| served);
    served
        .match_indices("\"/v1/")
        .filter_map(|(quote, _)| served[quote + 1..].split_once('"'))
        .map(|(path, _)| path)
        .collect()
}

/// Every `/v1` path the three tables above list, whatever its method.
fn listed_paths() -> BTreeSet<&'static str> {
    SESSION_BOUND_POSTS
        .into_iter()
        .chain(PRE_SESSION_POSTS)
        .chain(SESSION_BOUND_GETS.iter().copied())
        .collect()
}

fn route_table_violations(routes_source: &str, listed: &BTreeSet<&str>) -> Vec<String> {
    let named = v1_paths_named(routes_source);
    let unlisted = named
        .difference(listed)
        .map(|path| format!("routes.rs names `{path}`, which no table here lists"));
    let unserved = listed
        .difference(&named)
        .map(|path| format!("a table here lists `{path}`, which routes.rs does not name"));
    unlisted.chain(unserved).collect()
}

/// The calls that put a path on an axum `Router`.
const MOUNTING_CALLS: [&str; 4] = [".route(", ".route_service(", ".nest(", ".nest_service("];

fn mounting_violations(label: &str, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        for call in MOUNTING_CALLS {
            if line.contains(call) {
                out.push(format!("{label}:{}: contains `{call}`", idx + 1));
            }
        }
    }
    out
}

fn hub_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent directory")
        .join("crates")
        .join("demeteo-hub")
        .join("src")
}

fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            rust_files(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            into.push(path);
        }
    }
}

fn read_source(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// What ties the loops above to the router: a route in none of the tables is
/// one no loop here has asked anything of. Once a path is in
/// `SESSION_BOUND_POSTS` those loops are what show its route is behind the
/// guard, which is how one mounted after the `route_layer` is caught.
#[test]
fn every_v1_path_routes_rs_names_is_in_a_route_table() {
    let violations =
        route_table_violations(&read_source(&hub_src().join("routes.rs")), &listed_paths());
    assert_eq!(violations, Vec::<String>::new());
}

/// The scan above reads one file, so a route mounted from any other is one it
/// would never see. Unit tests are not exempt: the files are read whole.
#[test]
fn no_file_but_routes_rs_mounts_a_route() {
    let src = hub_src();
    let routes_rs = src.join("routes.rs");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.contains(&routes_rs), "{files:?}");

    let violations: Vec<String> = files
        .iter()
        .filter(|path| **path != routes_rs)
        .flat_map(|path| {
            let label = path
                .strip_prefix(&src)
                .unwrap_or(path)
                .display()
                .to_string();
            mounting_violations(&label, &read_source(path))
        })
        .collect();
    assert_eq!(violations, Vec::<String>::new());
}

/// `PRE_SESSION_POSTS` is deliberately not built from the Hub's constant. A
/// route the Hub exempts from the CSRF token has to fail here until someone
/// writes it into that table too.
#[test]
fn the_hub_exempts_from_the_csrf_token_exactly_the_pre_session_posts() {
    assert_eq!(routes::PRE_SESSION_ROUTES, PRE_SESSION_POSTS);
}

#[test]
fn the_route_scans_fire_on_violations() {
    let listed = BTreeSet::from(["/v1/session", "/v1/session/end"]);
    let as_listed = r#"
        fn v1() -> Router {
            Router::new()
                .route("/healthz", get(healthz))
                .route("/v1/session", get(read_session))
                .route(
                    "/v1/session/end",
                    post(end_session),
                )
        }
        #[cfg(test)]
        mod tests {
            const UNSERVED: &str = "/v1/a-route-nobody-listed";
        }
    "#;
    assert_eq!(
        v1_paths_named(as_listed),
        listed,
        "the path of a split call is read, and nothing of the unit tests"
    );
    assert_eq!(
        route_table_violations(as_listed, &listed),
        Vec::<String>::new()
    );

    let one_more = as_listed.replacen(
        "Router::new()",
        r#"Router::new().route("/v1/x", post(end_session))"#,
        1,
    );
    let violations = route_table_violations(&one_more, &listed);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("`/v1/x`"), "{violations:?}");

    let one_fewer = as_listed.replacen(r#""/v1/session/end""#, r#""/healthz""#, 1);
    let violations = route_table_violations(&one_fewer, &listed);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(
        violations[0].contains("`/v1/session/end`"),
        "{violations:?}"
    );

    for call in MOUNTING_CALLS {
        let elsewhere = format!(
            "fn extra(router: Router) -> Router {{\n    router{call}\"/v1/x\", post(x))\n}}"
        );
        let violations = mounting_violations("auth/handlers/mod.rs", &elsewhere);
        assert_eq!(violations.len(), 1, "{call}: {violations:?}");
        assert!(
            violations[0].starts_with("auth/handlers/mod.rs:2:"),
            "{violations:?}"
        );
    }
    assert!(mounting_violations("x", "let routes = routes::router(state, dir);").is_empty());
}
