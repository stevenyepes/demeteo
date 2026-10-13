//! The passkey store against a real Postgres. Needs the `postgres` service
//! from `deploy/hub/compose.yaml` as `deploy/hub/README.md` describes, and
//! `DEMETEO_HUB_TEST_DATABASE_URL` pointing at it.

mod support;

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::ceremonies::{Ceremony, MAX_LIVE_CEREMONIES};
use demeteo_hub::auth::chain::{self, ChainError, EndorsementRecord, RevocationRecord, StoredKey};
use demeteo_hub::auth::challenge::{
    base64url_decode, base64url_encode, endorsement_challenge, revocation_challenge,
    ChallengeBinding,
};
use demeteo_hub::auth::policy::{Refusal, Status, MAX_ACTIVE_PASSKEYS};
use demeteo_hub::auth::session::{SessionSecret, COOKIE_NAME, CSRF_HEADER, SESSION_TTL_SECS};
use demeteo_hub::auth::testing::{serve, SoftAuthenticator};
use demeteo_hub::auth::webauthn::Verifier;
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::store::passkey_records::{
    insert_endorsement, insert_revocation, list_endorsements, list_revocations,
};
use demeteo_hub::store::passkeys::{
    endorse_passkey, find_passkey, insert_passkey, list_passkeys, revoke_passkey,
    update_sign_count, NewPasskey, TrustChangeError,
};
use demeteo_hub::store::pool::{create_user, register_user, ReservedUser};
use demeteo_hub::store::sessions::{
    find_session, insert_session, NewSession, Session, SessionHash,
};
use demeteo_hub::store::{Db, StoreError, UserId};
use demeteo_hub_protocol::{PasskeyEndorsement, PasskeyRevocation, WebAuthnAssertion};
use serde_json::{json, Value};
use std::iter;
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;
use support::in_fresh_schema;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

fn new_passkey(credential_id: &[u8], label: &str) -> NewPasskey {
    NewPasskey {
        credential_id: credential_id.to_vec(),
        public_key: [credential_id, b"-cose-key"].concat(),
        label: label.to_owned(),
        sign_count: 5,
        credential: json!({ "label": label, "cred": { "counter": 5 } }).to_string(),
    }
}

fn as_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

async fn users_with_id(db: &Db, id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

fn in_an_hour() -> OffsetDateTime {
    OffsetDateTime::now_utc() + Duration::hours(1)
}

fn rp() -> RelyingParty {
    RelyingParty {
        rp_id: "hub.example".to_owned(),
        origin: "https://hub.example".to_owned(),
    }
}

fn now() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap()
}

fn raw_id(key: &SoftAuthenticator) -> Vec<u8> {
    base64url_decode(&key.credential_id()).unwrap()
}

async fn enrol(db: &Db, user: UserId, key: &SoftAuthenticator, status: Status) {
    enrol_as(db, user, key, status.as_str(), status).await;
}

async fn enrol_as(db: &Db, user: UserId, key: &SoftAuthenticator, label: &str, status: Status) {
    let new = NewPasskey {
        credential_id: raw_id(key),
        public_key: key.public_key_cose(),
        label: label.to_owned(),
        sign_count: 0,
        credential: "{}".to_owned(),
    };
    insert_passkey(user, db, &new, status).await.unwrap();
}

/// No passkey route asks the key service for anything, so it holds no key and
/// one that started to would be answered an error.
fn state_over(db: Db) -> AppState {
    let no_keys = MemoryKeys::with_keys(iter::empty::<String>());
    AppState::new(db, rp(), Arc::new(no_keys), String::new()).unwrap()
}

/// The Hub's real router over `db` on a loopback port, and the base URL it
/// answers at. The web directory does not exist: no `/v1` route reads it.
async fn serve_hub(db: Db) -> String {
    serve_state(state_over(db)).await
}

/// [`serve_hub`] over a state the test holds a clone of, and so can reach
/// behind the routes.
async fn serve_state(state: AppState) -> String {
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    serve(listener, routes::router(state, &web_dir));
    format!("http://{addr}")
}

async fn status_of(db: &Db, key: &SoftAuthenticator) -> Status {
    let found = find_passkey(db, &raw_id(key)).await.unwrap().unwrap();
    found.status
}

async fn sign_in(db: &Db, user: UserId, key: &SoftAuthenticator, secret: &[u8]) -> SessionHash {
    let hash = SessionHash::digest(secret);
    let new = NewSession {
        hash: hash.clone(),
        credential_id: raw_id(key),
        expires_at: in_an_hour(),
    };
    insert_session(user, db, &new).await.unwrap();
    hash
}

fn endorsement_by(
    endorser: &SoftAuthenticator,
    binding: &ChallengeBinding,
    pending: &SoftAuthenticator,
) -> WebAuthnAssertion {
    let challenge = endorsement_challenge(
        binding,
        &pending.credential_id(),
        &base64url_encode(&pending.public_key_cose()),
    )
    .unwrap();
    endorser.assert(&rp(), &challenge)
}

fn revocation_by(
    revoker: &SoftAuthenticator,
    binding: &ChallengeBinding,
    target: &SoftAuthenticator,
) -> WebAuthnAssertion {
    let challenge = revocation_challenge(binding, &target.credential_id()).unwrap();
    revoker.assert(&rp(), &challenge)
}

fn by_credential_id(mut keys: Vec<StoredKey>) -> Vec<StoredKey> {
    keys.sort_by(|a, b| a.credential_id.cmp(&b.credential_id));
    keys
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_passkey_round_trips_with_the_status_it_was_given() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let given = [
            (new_passkey(b"first", "laptop"), Status::Trusted),
            (new_passkey(b"second", "phone"), Status::Pending),
            (new_passkey(b"third", "lost key"), Status::Revoked),
        ];

        let mut tx = db.begin().await.unwrap();
        for (new, status) in &given {
            let stored = insert_passkey(user, &mut *tx, new, *status).await.unwrap();
            assert_eq!(stored.status, *status, "{}", new.label);
        }
        assert!(list_passkeys(user, &db).await.unwrap().is_empty());
        tx.commit().await.unwrap();

        let listed = list_passkeys(user, &db).await.unwrap();
        assert_eq!(listed.len(), given.len());
        for (new, status) in &given {
            let found = listed
                .iter()
                .find(|p| p.credential_id == new.credential_id)
                .unwrap_or_else(|| panic!("{} is not listed", new.label));
            assert_eq!(found.user_id, user);
            assert_eq!(found.public_key, new.public_key);
            assert_eq!(found.label, new.label);
            assert_eq!(found.sign_count, new.sign_count);
            assert_eq!(found.status, *status, "{}", new.label);
            assert_eq!(as_json(&found.credential), as_json(&new.credential));
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_duplicate_credential_id_is_the_typed_error() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        let first = new_passkey(b"shared", "a-laptop");
        insert_passkey(a, &db, &first, Status::Trusted)
            .await
            .unwrap();

        for (owner, label) in [(a, "a-again"), (b, "b-laptop")] {
            let err = insert_passkey(owner, &db, &new_passkey(b"shared", label), Status::Pending)
                .await
                .unwrap_err();
            assert!(
                matches!(err, StoreError::CredentialExists),
                "{label}: {err}"
            );
        }

        let kept = list_passkeys(a, &db).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(
            (kept[0].label.as_str(), kept[0].status),
            ("a-laptop", Status::Trusted)
        );
        assert!(list_passkeys(b, &db).await.unwrap().is_empty());

        // An insert that breaks a different rule stays a database error.
        let stranger = create_user(&db).await.unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(stranger)
            .execute(&db)
            .await
            .unwrap();
        let err = insert_passkey(stranger, &db, &new_passkey(b"orphan", "x"), Status::Pending)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::Database(_)), "{err}");
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn the_credential_id_lookup_returns_the_owner() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        insert_passkey(a, &db, &new_passkey(b"a-key", "a-laptop"), Status::Trusted)
            .await
            .unwrap();
        insert_passkey(b, &db, &new_passkey(b"b-key", "b-laptop"), Status::Pending)
            .await
            .unwrap();

        let of_a = find_passkey(&db, b"a-key").await.unwrap().unwrap();
        assert_eq!((of_a.user_id, of_a.status), (a, Status::Trusted));
        assert_eq!(of_a.label, "a-laptop");

        let of_b = find_passkey(&db, b"b-key").await.unwrap().unwrap();
        assert_eq!((of_b.user_id, of_b.status), (b, Status::Pending));

        assert_eq!(find_passkey(&db, b"a-ke").await.unwrap(), None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn only_the_owner_updates_a_sign_count_and_only_of_that_passkey() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        insert_passkey(a, &db, &new_passkey(b"a-key", "a-laptop"), Status::Trusted)
            .await
            .unwrap();
        insert_passkey(a, &db, &new_passkey(b"a-other", "a-phone"), Status::Pending)
            .await
            .unwrap();
        let count_of = |credential_id: &'static [u8]| {
            let db = db.clone();
            async move {
                let found = find_passkey(&db, credential_id).await.unwrap().unwrap();
                (found.sign_count, found.status)
            }
        };

        assert!(!update_sign_count(b, &db, b"a-key", 99).await.unwrap());
        assert_eq!(count_of(b"a-key").await, (5, Status::Trusted));

        assert!(update_sign_count(a, &db, b"a-key", u32::MAX).await.unwrap());
        assert_eq!(count_of(b"a-key").await, (u32::MAX, Status::Trusted));
        assert_eq!(count_of(b"a-other").await, (5, Status::Pending));

        assert!(update_sign_count(a, &db, b"a-key", 0).await.unwrap());
        assert_eq!(count_of(b"a-key").await.0, 0);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn registration_creates_the_user_its_passkey_and_its_session() {
    in_fresh_schema(|db| async move {
        let reserved = ReservedUser::generate();
        let handle = reserved.handle();
        let hash = SessionHash::digest(b"first-session");
        assert_eq!(users_with_id(&db, handle).await, 0);

        let user = register_user(
            reserved,
            &db,
            &new_passkey(b"first", "laptop"),
            Status::Trusted,
            hash.clone(),
            in_an_hour(),
        )
        .await
        .unwrap();

        assert_eq!(user.handle(), handle);
        assert_eq!(users_with_id(&db, handle).await, 1);

        let listed = list_passkeys(user, &db).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].user_id, user);
        assert_eq!(listed[0].credential_id, b"first");
        assert_eq!(listed[0].label, "laptop");
        assert_eq!(listed[0].status, Status::Trusted);

        let session = Session {
            user_id: user,
            credential_id: b"first".to_vec(),
        };
        assert_eq!(find_session(&db, &hash).await.unwrap(), Some(session));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_registration_whose_passkey_is_refused_leaves_no_user_and_no_session() {
    in_fresh_schema(|db| async move {
        let holder = create_user(&db).await.unwrap();
        insert_passkey(
            holder,
            &db,
            &new_passkey(b"taken", "theirs"),
            Status::Trusted,
        )
        .await
        .unwrap();

        let reserved = ReservedUser::generate();
        let handle = reserved.handle();
        let hash = SessionHash::digest(b"never-issued");
        let err = register_user(
            reserved,
            &db,
            &new_passkey(b"taken", "mine"),
            Status::Trusted,
            hash.clone(),
            in_an_hour(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, StoreError::CredentialExists), "{err}");
        assert_eq!(users_with_id(&db, handle).await, 0);
        assert_eq!(find_session(&db, &hash).await.unwrap(), None);
        let kept = find_passkey(&db, b"taken").await.unwrap().unwrap();
        assert_eq!((kept.user_id, kept.label.as_str()), (holder, "theirs"));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_endorsement_trusts_the_pending_passkey_and_its_stored_record_reverifies() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        enrol(&db, user, &trusted, Status::Trusted).await;
        enrol(&db, user, &pending, Status::Pending).await;
        let binding = ChallengeBinding::mint(now()).unwrap();
        let assertion = endorsement_by(&trusted, &binding, &pending);

        let decided = endorse_passkey(user, &db, |keys| {
            chain::endorse(
                &rp(),
                now(),
                keys,
                &binding,
                &pending.credential_id(),
                &assertion,
            )
        })
        .await
        .unwrap();

        assert_eq!(decided.passkey.status, Status::Trusted);
        assert_eq!(
            find_passkey(&db, &raw_id(&pending)).await.unwrap(),
            Some(decided.passkey)
        );
        assert_eq!(status_of(&db, &trusted).await, Status::Trusted);

        let listed = list_endorsements(user, &db).await.unwrap();
        assert_eq!(listed, [decided.record]);
        assert_eq!(listed[0].binding, binding);
        assert_eq!(listed[0].body.assertion, assertion);
        assert_eq!(
            chain::reverify_endorsement(&rp(), &trusted.public_key_cose(), &listed[0]),
            Ok(())
        );
        assert!(list_revocations(user, &db).await.unwrap().is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_refused_endorsement_writes_no_record_and_changes_no_status() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        enrol(&db, user, &trusted, Status::Trusted).await;
        enrol(&db, user, &pending, Status::Pending).await;
        let binding = ChallengeBinding::mint(now()).unwrap();

        let self_signed = endorsement_by(&pending, &binding, &pending);
        let refused = endorse_passkey(user, &db, |keys| {
            chain::endorse(
                &rp(),
                now(),
                keys,
                &binding,
                &pending.credential_id(),
                &self_signed,
            )
        })
        .await
        .unwrap_err();
        assert!(
            matches!(
                refused,
                TrustChangeError::Refused(ChainError::Policy(Refusal::SelfEndorsement))
            ),
            "{refused}"
        );

        // A decision the store cannot apply is undone whole: the record is
        // inserted before the status change finds no pending row.
        let names_a_trusted_key = EndorsementRecord {
            binding: binding.clone(),
            body: PasskeyEndorsement {
                new_credential_id: trusted.credential_id(),
                new_public_key: base64url_encode(&trusted.public_key_cose()),
                assertion: self_signed.clone(),
            },
        };
        let unapplied = endorse_passkey(user, &db, |_| Ok::<_, ChainError>(names_a_trusted_key))
            .await
            .unwrap_err();
        assert!(
            matches!(
                unapplied,
                TrustChangeError::Store(StoreError::RecordNamesNoPasskey(Status::Trusted))
            ),
            "{unapplied}"
        );

        // Nor is a record kept whose assertion no passkey of the tenant made.
        let asserted_by_a_stranger = EndorsementRecord {
            binding: binding.clone(),
            body: PasskeyEndorsement {
                new_credential_id: pending.credential_id(),
                new_public_key: base64url_encode(&pending.public_key_cose()),
                assertion: endorsement_by(&SoftAuthenticator::new(), &binding, &pending),
            },
        };
        let unapplied = endorse_passkey(user, &db, |_| Ok::<_, ChainError>(asserted_by_a_stranger))
            .await
            .unwrap_err();
        assert!(
            matches!(
                unapplied,
                TrustChangeError::Store(StoreError::AssertionGivesNoCounter)
            ),
            "{unapplied}"
        );

        assert_eq!(status_of(&db, &pending).await, Status::Pending);
        assert_eq!(status_of(&db, &trusted).await, Status::Trusted);
        assert!(list_endorsements(user, &db).await.unwrap().is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_revocation_ends_that_passkeys_sessions_and_its_stored_record_reverifies() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let revoker = SoftAuthenticator::new();
        let target = SoftAuthenticator::new();
        enrol(&db, user, &revoker, Status::Trusted).await;
        enrol(&db, user, &target, Status::Trusted).await;
        let kept = sign_in(&db, user, &revoker, b"revoker-session").await;
        let ended = [
            sign_in(&db, user, &target, b"target-laptop").await,
            sign_in(&db, user, &target, b"target-phone").await,
        ];
        let binding = ChallengeBinding::mint(now()).unwrap();
        let assertion = revocation_by(&revoker, &binding, &target);

        let decided = revoke_passkey(user, &db, |keys| {
            chain::revoke(
                &rp(),
                now(),
                keys,
                &binding,
                &target.credential_id(),
                &assertion,
            )
        })
        .await
        .unwrap();

        assert_eq!(decided.passkey.status, Status::Revoked);
        assert_eq!(
            find_passkey(&db, &raw_id(&target)).await.unwrap(),
            Some(decided.passkey)
        );
        assert_eq!(status_of(&db, &revoker).await, Status::Trusted);
        for hash in &ended {
            assert_eq!(find_session(&db, hash).await.unwrap(), None);
        }
        assert_eq!(sessions_by_credential(&db).await, [raw_id(&revoker)]);
        let still_signed_in = Session {
            user_id: user,
            credential_id: raw_id(&revoker),
        };
        assert_eq!(
            find_session(&db, &kept).await.unwrap(),
            Some(still_signed_in)
        );

        let listed = list_revocations(user, &db).await.unwrap();
        assert_eq!(listed, [decided.record]);
        assert_eq!(listed[0].binding, binding);
        assert_eq!(listed[0].body.assertion, assertion);
        assert_eq!(
            chain::reverify_revocation(&rp(), &revoker.public_key_cose(), &listed[0]),
            Ok(())
        );
        assert!(list_endorsements(user, &db).await.unwrap().is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_refused_revocation_writes_no_record_and_ends_no_session() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let only = SoftAuthenticator::new();
        let gone = SoftAuthenticator::new();
        enrol(&db, user, &only, Status::Trusted).await;
        enrol(&db, user, &gone, Status::Revoked).await;
        sign_in(&db, user, &only, b"only-session").await;
        sign_in(&db, user, &gone, b"gone-session").await;
        let binding = ChallengeBinding::mint(now()).unwrap();
        let assertion = revocation_by(&only, &binding, &only);

        let refused = revoke_passkey(user, &db, |keys| {
            chain::revoke(
                &rp(),
                now(),
                keys,
                &binding,
                &only.credential_id(),
                &assertion,
            )
        })
        .await
        .unwrap_err();
        assert!(
            matches!(
                refused,
                TrustChangeError::Refused(ChainError::Policy(Refusal::LastPasskey))
            ),
            "{refused}"
        );

        // As for an endorsement: the record is inserted before the status
        // change finds the row already revoked, and is undone with it.
        let names_a_revoked_key = RevocationRecord {
            binding: binding.clone(),
            body: PasskeyRevocation {
                credential_id: gone.credential_id(),
                assertion: assertion.clone(),
            },
        };
        let unapplied = revoke_passkey(user, &db, |_| Ok::<_, ChainError>(names_a_revoked_key))
            .await
            .unwrap_err();
        assert!(
            matches!(
                unapplied,
                TrustChangeError::Store(StoreError::RecordNamesNoPasskey(Status::Revoked))
            ),
            "{unapplied}"
        );

        assert_eq!(status_of(&db, &only).await, Status::Trusted);
        assert_eq!(
            sessions_by_credential(&db).await,
            [raw_id(&only), raw_id(&gone)]
        );
        assert!(list_revocations(user, &db).await.unwrap().is_empty());
    })
    .await;
}

async fn sign_count_of(db: &Db, key: &SoftAuthenticator) -> u32 {
    let found = find_passkey(db, &raw_id(key)).await.unwrap().unwrap();
    found.sign_count
}

/// The revocation's counter is lower than the endorsement's, and is stored
/// all the same.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_trust_change_stores_the_counter_its_assertion_reported_on_the_asserting_passkey() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let asserting = SoftAuthenticator::new().with_sign_count(9);
        let other = SoftAuthenticator::new();
        enrol(&db, user, &asserting, Status::Trusted).await;
        enrol(&db, user, &other, Status::Pending).await;
        let binding = ChallengeBinding::mint(now()).unwrap();

        let assertion = endorsement_by(&asserting, &binding, &other);
        endorse_passkey(user, &db, |keys| {
            chain::endorse(
                &rp(),
                now(),
                keys,
                &binding,
                &other.credential_id(),
                &assertion,
            )
        })
        .await
        .unwrap();
        assert_eq!(sign_count_of(&db, &asserting).await, 9);
        assert_eq!(sign_count_of(&db, &other).await, 0);

        let asserting = asserting.with_sign_count(3);
        let assertion = revocation_by(&asserting, &binding, &other);
        revoke_passkey(user, &db, |keys| {
            chain::revoke(
                &rp(),
                now(),
                keys,
                &binding,
                &other.credential_id(),
                &assertion,
            )
        })
        .await
        .unwrap();
        assert_eq!(sign_count_of(&db, &asserting).await, 3);
        assert_eq!(sign_count_of(&db, &other).await, 0);
    })
    .await;
}

/// Without the row locks both decisions read "two trusted" and both commit,
/// leaving a tenant nobody can sign in to.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn two_concurrent_cross_revocations_leave_exactly_one_trusted_passkey() {
    in_fresh_schema(|db| async move {
        for round in 0..8 {
            let user = create_user(&db).await.unwrap();
            let a = SoftAuthenticator::new();
            let b = SoftAuthenticator::new();
            enrol(&db, user, &a, Status::Trusted).await;
            enrol(&db, user, &b, Status::Trusted).await;
            let a_binding = ChallengeBinding::mint(now()).unwrap();
            let b_binding = ChallengeBinding::mint(now()).unwrap();
            let by_a = revocation_by(&a, &a_binding, &b);
            let by_b = revocation_by(&b, &b_binding, &a);

            let (a_revokes_b, b_revokes_a) = tokio::join!(
                revoke_passkey(user, &db, |keys| {
                    chain::revoke(&rp(), now(), keys, &a_binding, &b.credential_id(), &by_a)
                }),
                revoke_passkey(user, &db, |keys| {
                    chain::revoke(&rp(), now(), keys, &b_binding, &a.credential_id(), &by_b)
                }),
            );

            let (won, lost) = match (a_revokes_b, b_revokes_a) {
                (Ok(won), Err(lost)) | (Err(lost), Ok(won)) => (won, lost),
                (Ok(_), Ok(_)) => panic!("round {round}: both revocations were accepted"),
                (Err(first), Err(second)) => panic!("round {round}: {first}; {second}"),
            };
            assert!(
                matches!(lost, TrustChangeError::Refused(ChainError::Policy(_))),
                "round {round}: {lost}"
            );
            let statuses = [status_of(&db, &a).await, status_of(&db, &b).await];
            assert!(
                statuses == [Status::Trusted, Status::Revoked]
                    || statuses == [Status::Revoked, Status::Trusted],
                "round {round}: {statuses:?}"
            );
            assert_eq!(list_revocations(user, &db).await.unwrap(), [won.record]);
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_decision_sees_only_its_own_tenants_passkeys_and_can_change_no_others() {
    in_fresh_schema(|db| async move {
        let mine = create_user(&db).await.unwrap();
        let theirs = create_user(&db).await.unwrap();
        let my_trusted = SoftAuthenticator::new();
        let my_pending = SoftAuthenticator::new();
        let their_trusted = SoftAuthenticator::new();
        let their_other = SoftAuthenticator::new();
        let their_pending = SoftAuthenticator::new();
        enrol(&db, mine, &my_trusted, Status::Trusted).await;
        enrol(&db, mine, &my_pending, Status::Pending).await;
        enrol(&db, theirs, &their_trusted, Status::Trusted).await;
        enrol(&db, theirs, &their_other, Status::Trusted).await;
        enrol(&db, theirs, &their_pending, Status::Pending).await;
        let my_keys = by_credential_id(vec![
            my_trusted.stored(Status::Trusted),
            my_pending.stored(Status::Pending),
        ]);
        let their_keys = [
            their_trusted.stored(Status::Trusted),
            their_other.stored(Status::Trusted),
            their_pending.stored(Status::Pending),
        ];

        let mut shown_to_endorse = Vec::new();
        let undecided = endorse_passkey(mine, &db, |keys| {
            shown_to_endorse = keys.to_vec();
            Err::<EndorsementRecord, _>("undecided")
        })
        .await
        .unwrap_err();
        assert!(matches!(undecided, TrustChangeError::Refused("undecided")));
        assert_eq!(by_credential_id(shown_to_endorse), my_keys);

        let mut shown_to_revoke = Vec::new();
        let undecided = revoke_passkey(mine, &db, |keys| {
            shown_to_revoke = keys.to_vec();
            Err::<RevocationRecord, _>("undecided")
        })
        .await
        .unwrap_err();
        assert!(matches!(undecided, TrustChangeError::Refused("undecided")));
        assert_eq!(by_credential_id(shown_to_revoke), my_keys);

        // Records the other tenant's own keys would be right to accept, handed
        // back from a transaction over mine.
        let binding = ChallengeBinding::mint(now()).unwrap();
        let endorsement = chain::endorse(
            &rp(),
            now(),
            &their_keys,
            &binding,
            &their_pending.credential_id(),
            &endorsement_by(&their_trusted, &binding, &their_pending),
        )
        .unwrap();
        let revocation = chain::revoke(
            &rp(),
            now(),
            &their_keys,
            &binding,
            &their_other.credential_id(),
            &revocation_by(&their_trusted, &binding, &their_other),
        )
        .unwrap();
        let session = sign_in(&db, theirs, &their_other, b"their-session").await;

        let endorsed = endorse_passkey(mine, &db, |_| Ok::<_, ChainError>(endorsement))
            .await
            .unwrap_err();
        assert!(
            matches!(
                endorsed,
                TrustChangeError::Store(StoreError::RecordNamesNoPasskey(Status::Trusted))
            ),
            "{endorsed}"
        );
        let revoked = revoke_passkey(mine, &db, |_| Ok::<_, ChainError>(revocation))
            .await
            .unwrap_err();
        assert!(
            matches!(
                revoked,
                TrustChangeError::Store(StoreError::RecordNamesNoPasskey(Status::Revoked))
            ),
            "{revoked}"
        );

        assert_eq!(status_of(&db, &their_pending).await, Status::Pending);
        assert_eq!(status_of(&db, &their_other).await, Status::Trusted);
        assert!(find_session(&db, &session).await.unwrap().is_some());
        for user in [mine, theirs] {
            assert!(list_endorsements(user, &db).await.unwrap().is_empty());
            assert!(list_revocations(user, &db).await.unwrap().is_empty());
        }
    })
    .await;
}

/// The store keeps whatever text a record carries. Padded base64 is text no
/// verifier accepts, which is what shows it was not decoded and encoded again
/// on the way in or out.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_record_is_stored_as_the_text_and_the_integer_it_carries() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let other = create_user(&db).await.unwrap();
        let as_received = WebAuthnAssertion {
            credential_id: "c2lnbmVy".to_owned(),
            authenticator_data: "YXV0aA==".to_owned(),
            client_data_json: "e30=".to_owned(),
            signature: "c2ln+/8=".to_owned(),
        };
        let binding = |request_id: &str, expires_at| ChallengeBinding {
            instance_id: "an-instance".to_owned(),
            request_id: request_id.to_owned(),
            expires_at,
        };
        let endorsement = |expires_at| EndorsementRecord {
            binding: binding("endorse-request", expires_at),
            body: PasskeyEndorsement {
                new_credential_id: "bmV3".to_owned(),
                new_public_key: "a2V5==".to_owned(),
                assertion: as_received.clone(),
            },
        };
        let revocation = |expires_at| RevocationRecord {
            binding: binding("revoke-request", expires_at),
            body: PasskeyRevocation {
                credential_id: "b2xk".to_owned(),
                assertion: as_received.clone(),
            },
        };
        const LARGEST: u64 = i64::MAX as u64;

        insert_endorsement(user, &db, &endorsement(LARGEST))
            .await
            .unwrap();
        insert_revocation(user, &db, &revocation(LARGEST))
            .await
            .unwrap();
        let too_large = insert_endorsement(user, &db, &endorsement(LARGEST + 1))
            .await
            .unwrap_err();
        assert!(
            matches!(too_large, StoreError::ExpiryTooLarge(e) if e == LARGEST + 1),
            "{too_large}"
        );
        let too_large = insert_revocation(user, &db, &revocation(u64::MAX))
            .await
            .unwrap_err();
        assert!(
            matches!(too_large, StoreError::ExpiryTooLarge(u64::MAX)),
            "{too_large}"
        );

        assert_eq!(
            list_endorsements(user, &db).await.unwrap(),
            [endorsement(LARGEST)]
        );
        assert_eq!(
            list_revocations(user, &db).await.unwrap(),
            [revocation(LARGEST)]
        );
        assert!(list_endorsements(other, &db).await.unwrap().is_empty());
        assert!(list_revocations(other, &db).await.unwrap().is_empty());
    })
    .await;
}

/// The list is compared whole and by credential id, so a passkey of the other
/// tenant, or a field a client must not see, would show as an extra.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn the_passkey_list_is_the_sessions_tenants_own_and_marks_the_one_that_signed_in() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let enrolled = [
            (SoftAuthenticator::new(), "laptop", Status::Trusted),
            (SoftAuthenticator::new(), "phone", Status::Pending),
            (SoftAuthenticator::new(), "lost key", Status::Revoked),
        ];
        for (key, label, status) in &enrolled {
            enrol_as(&db, user, key, label, *status).await;
        }
        let stranger = create_user(&db).await.unwrap();
        enrol(&db, stranger, &SoftAuthenticator::new(), Status::Trusted).await;

        let signed_in_with = &enrolled[0].0;
        let secret = SessionSecret::generate().unwrap();
        let session = NewSession {
            hash: secret.hash(),
            credential_id: raw_id(signed_in_with),
            expires_at: in_an_hour(),
        };
        insert_session(user, &db, &session).await.unwrap();

        let base = serve_hub(db.clone()).await;
        let response = reqwest::Client::new()
            .get(format!("{base}/v1/passkeys"))
            .header("cookie", format!("{COOKIE_NAME}={}", secret.cookie_value()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let mut body = as_json(&response.text().await.unwrap());

        let mut expected = Vec::new();
        for (key, label, status) in &enrolled {
            let stored = find_passkey(&db, &raw_id(key)).await.unwrap().unwrap();
            expected.push(json!({
                "credential_id": key.credential_id(),
                "label": label,
                "status": status.as_str(),
                "created_at": stored.created_at.unix_timestamp(),
                "current": key.credential_id() == signed_in_with.credential_id(),
            }));
        }
        let by_id = |passkey: &Value| passkey["credential_id"].as_str().map(str::to_owned);
        expected.sort_by_key(by_id);
        body["passkeys"]
            .as_array_mut()
            .expect("a `passkeys` array")
            .sort_by_key(by_id);
        assert_eq!(body, json!({ "passkeys": expected }));
    })
    .await;
}

async fn open_session(db: &Db, user: UserId, key: &SoftAuthenticator) -> SessionSecret {
    let secret = SessionSecret::generate().unwrap();
    let session = NewSession {
        hash: secret.hash(),
        credential_id: raw_id(key),
        expires_at: in_an_hour(),
    };
    insert_session(user, db, &session).await.unwrap();
    secret
}

/// A `POST` as the Hub's own page sends it for `session`: same origin, the
/// cookie, and the CSRF token derived from it.
async fn post_as(base: &str, path: &str, session: &SessionSecret, body: &Value) -> (u16, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("origin", rp().origin)
        .header(
            "cookie",
            format!("{COOKIE_NAME}={}", session.cookie_value()),
        )
        .header(CSRF_HEADER, session.csrf_token())
        .json(body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, as_json(&response.text().await.unwrap()))
}

async fn start_endorsement(base: &str, session: &SessionSecret, pending: &str) -> (u16, Value) {
    let target = json!({ "credential_id": pending });
    post_as(base, "/v1/passkeys/endorse/start", session, &target).await
}

fn challenge_in(started: &Value) -> [u8; 32] {
    started["options"]["publicKey"]["challenge"]
        .as_str()
        .and_then(base64url_decode)
        .and_then(|bytes| bytes.try_into().ok())
        .expect("a 32-byte base64url challenge")
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_trusted_passkeys_assertion_over_the_started_challenge_trusts_the_pending_one() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        enrol_as(&db, user, &trusted, "laptop", Status::Trusted).await;
        enrol_as(&db, user, &pending, "phone", Status::Pending).await;
        let session = open_session(&db, user, &trusted).await;
        let base = serve_hub(db.clone()).await;

        let (status, started) = start_endorsement(&base, &session, &pending.credential_id()).await;
        assert_eq!(status, 200, "{started}");
        let options = &started["options"]["publicKey"];
        assert_eq!(options["rpId"], rp().rp_id);
        assert_eq!(options["userVerification"], "required");
        assert_eq!(
            options["allowCredentials"],
            json!([{ "type": "public-key", "id": trusted.credential_id() }])
        );
        let challenge = challenge_in(&started);
        let assertion = trusted.assert(&rp(), &challenge);
        assert_eq!(status_of(&db, &pending).await, Status::Pending);

        let finish = json!({ "ceremony_id": started["ceremony_id"], "assertion": assertion });
        let (status, finished) =
            post_as(&base, "/v1/passkeys/endorse/finish", &session, &finish).await;

        let stored = find_passkey(&db, &raw_id(&pending)).await.unwrap().unwrap();
        assert_eq!(
            (status, finished),
            (
                200,
                json!({
                    "passkey": {
                        "credential_id": pending.credential_id(),
                        "label": "phone",
                        "status": "trusted",
                        "created_at": stored.created_at.unix_timestamp(),
                        "current": false,
                    }
                })
            )
        );
        assert_eq!(stored.status, Status::Trusted);

        let records = list_endorsements(user, &db).await.unwrap();
        assert_eq!(records.len(), 1, "{records:?}");
        let new_public_key = base64url_encode(&pending.public_key_cose());
        assert_eq!(
            endorsement_challenge(
                &records[0].binding,
                &pending.credential_id(),
                &new_public_key
            ),
            Ok(challenge)
        );
        assert_eq!(
            records[0].body,
            PasskeyEndorsement {
                new_credential_id: pending.credential_id(),
                new_public_key,
                assertion,
            }
        );
        assert_eq!(
            chain::reverify_endorsement(&rp(), &trusted.public_key_cose(), &records[0]),
            Ok(())
        );

        assert_eq!(
            post_as(&base, "/v1/passkeys/endorse/finish", &session, &finish).await,
            (400, json!({ "error": "ceremony_expired" }))
        );
    })
    .await;
}

/// The assertion is a valid one over the challenge the Hub served, so what
/// refuses it is who made it.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_pending_passkey_asserting_for_itself_is_403_self_endorsement_and_stays_pending() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        enrol(&db, user, &trusted, Status::Trusted).await;
        enrol(&db, user, &pending, Status::Pending).await;
        let session = open_session(&db, user, &trusted).await;
        let base = serve_hub(db.clone()).await;

        let (status, started) = start_endorsement(&base, &session, &pending.credential_id()).await;
        assert_eq!(status, 200, "{started}");
        let self_signed = pending.assert(&rp(), &challenge_in(&started));
        let finish = json!({ "ceremony_id": started["ceremony_id"], "assertion": self_signed });

        assert_eq!(
            post_as(&base, "/v1/passkeys/endorse/finish", &session, &finish).await,
            (403, json!({ "error": "self_endorsement" }))
        );
        assert_eq!(status_of(&db, &pending).await, Status::Pending);
        assert!(list_endorsements(user, &db).await.unwrap().is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_endorsement_is_started_only_for_a_pending_passkey_of_the_sessions_tenant() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let trusted = SoftAuthenticator::new();
        enrol(&db, user, &trusted, Status::Trusted).await;
        let stranger = create_user(&db).await.unwrap();
        let strangers_pending = SoftAuthenticator::new();
        enrol(&db, stranger, &SoftAuthenticator::new(), Status::Trusted).await;
        enrol(&db, stranger, &strangers_pending, Status::Pending).await;
        let session = open_session(&db, user, &trusted).await;
        let base = serve_hub(db.clone()).await;

        assert_eq!(
            start_endorsement(&base, &session, &strangers_pending.credential_id()).await,
            (404, json!({ "error": "not_found" }))
        );
        assert_eq!(
            start_endorsement(&base, &session, &trusted.credential_id()).await,
            (409, json!({ "error": "not_pending" }))
        );
    })
    .await;
}

async fn start_revocation(base: &str, session: &SessionSecret, target: &str) -> (u16, Value) {
    let target = json!({ "credential_id": target });
    post_as(base, "/v1/passkeys/revoke/start", session, &target).await
}

async fn session_is_live(base: &str, session: &SessionSecret) -> bool {
    let response = reqwest::Client::new()
        .get(format!("{base}/v1/session"))
        .header(
            "cookie",
            format!("{COOKIE_NAME}={}", session.cookie_value()),
        )
        .send()
        .await
        .unwrap();
    match response.status().as_u16() {
        200 => true,
        401 => false,
        other => panic!("GET /v1/session answered {other}"),
    }
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_revocation_is_not_started_for_the_only_trusted_passkey_nor_for_one_it_cannot_revoke() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let only = SoftAuthenticator::new();
        let gone = SoftAuthenticator::new();
        enrol(&db, user, &only, Status::Trusted).await;
        enrol(&db, user, &gone, Status::Revoked).await;
        let stranger = create_user(&db).await.unwrap();
        let strangers = SoftAuthenticator::new();
        enrol(&db, stranger, &strangers, Status::Trusted).await;
        enrol(&db, stranger, &SoftAuthenticator::new(), Status::Trusted).await;
        let session = open_session(&db, user, &only).await;
        let base = serve_hub(db.clone()).await;

        assert_eq!(
            start_revocation(&base, &session, &only.credential_id()).await,
            (409, json!({ "error": "last_passkey" }))
        );
        assert_eq!(
            start_revocation(&base, &session, &gone.credential_id()).await,
            (409, json!({ "error": "already_revoked" }))
        );
        assert_eq!(
            start_revocation(&base, &session, &strangers.credential_id()).await,
            (404, json!({ "error": "not_found" }))
        );
    })
    .await;
}

/// The assertion is a valid one over the challenge the Hub served, so what
/// refuses it is who made it.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_passkey_asserting_its_own_revocation_is_403_self_revocation_and_nothing_changes() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let a = SoftAuthenticator::new();
        let b = SoftAuthenticator::new();
        enrol(&db, user, &a, Status::Trusted).await;
        enrol(&db, user, &b, Status::Trusted).await;
        let session = open_session(&db, user, &b).await;
        let base = serve_hub(db.clone()).await;

        let (status, started) = start_revocation(&base, &session, &b.credential_id()).await;
        assert_eq!(status, 200, "{started}");
        let self_signed = b.assert(&rp(), &challenge_in(&started));
        let finish = json!({ "ceremony_id": started["ceremony_id"], "assertion": self_signed });

        assert_eq!(
            post_as(&base, "/v1/passkeys/revoke/finish", &session, &finish).await,
            (403, json!({ "error": "self_revocation" }))
        );
        assert_eq!(status_of(&db, &a).await, Status::Trusted);
        assert_eq!(status_of(&db, &b).await, Status::Trusted);
        assert!(list_revocations(user, &db).await.unwrap().is_empty());
        assert!(session_is_live(&base, &session).await);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn another_trusted_passkeys_assertion_revokes_the_target_and_ends_its_sessions() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let a = SoftAuthenticator::new();
        let b = SoftAuthenticator::new();
        enrol_as(&db, user, &a, "laptop", Status::Trusted).await;
        enrol_as(&db, user, &b, "phone", Status::Trusted).await;
        enrol(&db, user, &SoftAuthenticator::new(), Status::Pending).await;
        enrol(&db, user, &SoftAuthenticator::new(), Status::Revoked).await;
        let session = open_session(&db, user, &a).await;
        let ended = [
            open_session(&db, user, &b).await,
            open_session(&db, user, &b).await,
        ];
        let base = serve_hub(db.clone()).await;

        let (status, started) = start_revocation(&base, &session, &b.credential_id()).await;
        assert_eq!(status, 200, "{started}");
        let options = &started["options"]["publicKey"];
        assert_eq!(options["rpId"], rp().rp_id);
        assert_eq!(options["userVerification"], "required");
        assert_eq!(
            options["allowCredentials"],
            json!([{ "type": "public-key", "id": a.credential_id() }])
        );
        let challenge = challenge_in(&started);
        let assertion = a.assert(&rp(), &challenge);
        assert_eq!(status_of(&db, &b).await, Status::Trusted);
        for theirs in &ended {
            assert!(session_is_live(&base, theirs).await);
        }

        let finish = json!({ "ceremony_id": started["ceremony_id"], "assertion": assertion });
        let (status, finished) =
            post_as(&base, "/v1/passkeys/revoke/finish", &session, &finish).await;

        let stored = find_passkey(&db, &raw_id(&b)).await.unwrap().unwrap();
        assert_eq!(
            (status, finished),
            (
                200,
                json!({
                    "passkey": {
                        "credential_id": b.credential_id(),
                        "label": "phone",
                        "status": "revoked",
                        "created_at": stored.created_at.unix_timestamp(),
                        "current": false,
                    }
                })
            )
        );
        assert_eq!(stored.status, Status::Revoked);
        assert_eq!(status_of(&db, &a).await, Status::Trusted);
        assert_eq!(sessions_by_credential(&db).await, [raw_id(&a)]);
        for theirs in &ended {
            assert!(!session_is_live(&base, theirs).await);
        }
        assert!(session_is_live(&base, &session).await);

        let records = list_revocations(user, &db).await.unwrap();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(
            revocation_challenge(&records[0].binding, &b.credential_id()),
            Ok(challenge)
        );
        assert_eq!(
            records[0].body,
            PasskeyRevocation {
                credential_id: b.credential_id(),
                assertion,
            }
        );
        assert_eq!(
            chain::reverify_revocation(&rp(), &a.public_key_cose(), &records[0]),
            Ok(())
        );

        assert_eq!(
            post_as(&base, "/v1/passkeys/revoke/finish", &session, &finish).await,
            (400, json!({ "error": "ceremony_expired" }))
        );
    })
    .await;
}

/// Writes a session row for each passkey as a sign-in's last statement does.
/// Both rows are stored; only the one whose passkey is trusted authenticates,
/// in the store and over HTTP.
async fn assert_only_the_trusted_passkeys_session_authenticates(
    db: &Db,
    user: UserId,
    untrusted: &SoftAuthenticator,
    trusted: &SoftAuthenticator,
) {
    let inert = open_session(db, user, untrusted).await;
    let control = open_session(db, user, trusted).await;
    assert!(sessions_by_credential(db)
        .await
        .contains(&raw_id(untrusted)));
    let base = serve_hub(db.clone()).await;

    assert_eq!(find_session(db, &inert.hash()).await.unwrap(), None);
    assert!(!session_is_live(&base, &inert).await);
    let signed_in = Session {
        user_id: user,
        credential_id: raw_id(trusted),
    };
    assert_eq!(
        find_session(db, &control.hash()).await.unwrap(),
        Some(signed_in)
    );
    assert!(session_is_live(&base, &control).await);
}

/// A sign-in reads its passkey's status and inserts the session as separate
/// statements, so a revocation can commit between the two and find no session
/// to end. The two writes are made here in the order that race produces.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_session_written_after_its_passkeys_revocation_does_not_authenticate() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let revoker = SoftAuthenticator::new();
        let target = SoftAuthenticator::new();
        enrol(&db, user, &revoker, Status::Trusted).await;
        enrol(&db, user, &target, Status::Trusted).await;
        let binding = ChallengeBinding::mint(now()).unwrap();
        let assertion = revocation_by(&revoker, &binding, &target);

        revoke_passkey(user, &db, |keys| {
            chain::revoke(
                &rp(),
                now(),
                keys,
                &binding,
                &target.credential_id(),
                &assertion,
            )
        })
        .await
        .unwrap();
        assert_eq!(status_of(&db, &target).await, Status::Revoked);

        assert_only_the_trusted_passkeys_session_authenticates(&db, user, &target, &revoker).await;
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_session_naming_a_pending_passkey_does_not_authenticate() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        enrol(&db, user, &trusted, Status::Trusted).await;
        enrol(&db, user, &pending, Status::Pending).await;

        assert_only_the_trusted_passkeys_session_authenticates(&db, user, &pending, &trusted).await;
    })
    .await;
}

/// Both ceremonies are over the same passkey and the assertion is the one
/// that would endorse it. Neither way of presenting it removes the passkey:
/// not under the revocation's own ceremony, and not under the endorsement's.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_endorsement_assertion_does_not_finish_a_revocation() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let trusted = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        enrol(&db, user, &trusted, Status::Trusted).await;
        enrol(&db, user, &pending, Status::Pending).await;
        let session = open_session(&db, user, &trusted).await;
        let base = serve_hub(db.clone()).await;

        let (status, revoking) = start_revocation(&base, &session, &pending.credential_id()).await;
        assert_eq!(status, 200, "{revoking}");
        let (status, endorsing) =
            start_endorsement(&base, &session, &pending.credential_id()).await;
        assert_eq!(status, 200, "{endorsing}");
        let endorsement = trusted.assert(&rp(), &challenge_in(&endorsing));

        let finish = json!({ "ceremony_id": revoking["ceremony_id"], "assertion": endorsement });
        assert_eq!(
            post_as(&base, "/v1/passkeys/revoke/finish", &session, &finish).await,
            (403, json!({ "error": "assertion_invalid" }))
        );
        let finish = json!({ "ceremony_id": endorsing["ceremony_id"], "assertion": endorsement });
        assert_eq!(
            post_as(&base, "/v1/passkeys/revoke/finish", &session, &finish).await,
            (400, json!({ "error": "ceremony_expired" }))
        );

        assert_eq!(status_of(&db, &pending).await, Status::Pending);
        assert!(list_revocations(user, &db).await.unwrap().is_empty());
    })
    .await;
}

/// Sign-in starts need no session, so anyone can keep their budget full. The
/// first assertion is the control: without it the three that follow would
/// pass against a map that was never full.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_flood_of_sign_in_starts_refuses_sign_ins_and_no_signed_in_tenants_ceremony() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let trusted = SoftAuthenticator::new();
        let other = SoftAuthenticator::new();
        let pending = SoftAuthenticator::new();
        enrol_as(&db, user, &trusted, "laptop", Status::Trusted).await;
        enrol_as(&db, user, &other, "phone", Status::Trusted).await;
        enrol(&db, user, &pending, Status::Pending).await;
        let session = open_session(&db, user, &trusted).await;
        let state = state_over(db.clone());
        let base = serve_state(state.clone()).await;
        for _ in 0..MAX_LIVE_CEREMONIES {
            let (_, sign_in) = state.verifier().start_sign_in().unwrap();
            let flooding = Ceremony::SignIn { sign_in };
            state.ceremonies().insert(now(), flooding).unwrap();
        }

        let refused = post_signed_out(&base, "/v1/session/start", &json!({})).await;
        assert_eq!(refused.status().as_u16(), 503);
        assert_eq!(
            as_json(&refused.text().await.unwrap()),
            json!({ "error": "too_many_ceremonies" })
        );

        let (status, revoking) = start_revocation(&base, &session, &other.credential_id()).await;
        assert_eq!(status, 200, "{revoking}");
        let (status, endorsing) =
            start_endorsement(&base, &session, &pending.credential_id()).await;
        assert_eq!(status, 200, "{endorsing}");
        let label = json!({ "label": "tablet" });
        let (status, adding) = post_as(&base, "/v1/passkeys/add/start", &session, &label).await;
        assert_eq!(status, 200, "{adding}");
    })
    .await;
}

/// A `POST` as the Hub's own page sends it before there is a session: same
/// origin, and nothing else.
async fn post_signed_out(base: &str, path: &str, body: &Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("origin", rp().origin)
        .json(body)
        .send()
        .await
        .unwrap()
}

async fn start_registration(base: &str, label: &str) -> Value {
    let response = post_signed_out(base, "/v1/register/start", &json!({ "label": label })).await;
    assert_eq!(response.status().as_u16(), 200);
    as_json(&response.text().await.unwrap())
}

/// Finishes `started` with a credential `key` makes at the Hub's own origin.
async fn finish_registration(
    base: &str,
    started: &Value,
    key: &SoftAuthenticator,
) -> reqwest::Response {
    let finish = json!({
        "ceremony_id": started["ceremony_id"],
        "credential": key.create(&rp().origin, &started["options"]),
    });
    post_signed_out(base, "/v1/register/finish", &finish).await
}

/// The tenant a started registration would create, read from the user handle
/// its options give the authenticator.
fn reserved_handle(started: &Value) -> Uuid {
    let handle = started["options"]["publicKey"]["user"]["id"]
        .as_str()
        .and_then(base64url_decode)
        .expect("a base64url user handle");
    Uuid::from_slice(&handle).unwrap()
}

async fn get_as(base: &str, path: &str, session: &SessionSecret) -> (u16, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .header(
            "cookie",
            format!("{COOKIE_NAME}={}", session.cookie_value()),
        )
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, as_json(&response.text().await.unwrap()))
}

/// The session is the one the `Set-Cookie` of the finish carries and nothing
/// else: every later request here presents only what a browser would have
/// kept of that header.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_registration_creates_a_tenant_signed_in_with_its_one_trusted_passkey() {
    in_fresh_schema(|db| async move {
        let base = serve_hub(db.clone()).await;
        let key = SoftAuthenticator::new().with_sign_count(3);

        let started = start_registration(&base, "  Work laptop ").await;
        let handle = reserved_handle(&started);
        assert_eq!(users_with_id(&db, handle).await, 0);

        let response = finish_registration(&base, &started, &key).await;
        assert_eq!(response.status().as_u16(), 200);
        let set_cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .to_owned();
        let finished = as_json(&response.text().await.unwrap());

        let (pair, attributes) = set_cookie.split_once("; ").unwrap();
        let attributes: Vec<&str> = attributes.split("; ").collect();
        for required in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
            assert!(attributes.contains(&required), "{required}: {attributes:?}");
        }
        let session = SessionSecret::from_cookie_header(pair).expect("the session cookie");
        assert_eq!(
            finished,
            json!({
                "csrf_token": session.csrf_token(),
                "credential_id": key.credential_id(),
            })
        );
        assert_eq!(
            get_as(&base, "/v1/session", &session).await,
            (200, finished)
        );

        let stored = find_passkey(&db, &raw_id(&key)).await.unwrap().unwrap();
        assert_eq!(stored.user_id.handle(), handle);
        assert_eq!(users_with_id(&db, handle).await, 1);
        assert_eq!(stored.public_key, key.public_key_cose());
        assert_eq!(stored.sign_count, 3);
        assert_eq!(
            get_as(&base, "/v1/passkeys", &session).await,
            (
                200,
                json!({
                    "passkeys": [{
                        "credential_id": key.credential_id(),
                        "label": "Work laptop",
                        "status": "trusted",
                        "created_at": stored.created_at.unix_timestamp(),
                        "current": true,
                    }]
                })
            )
        );

        let expires_at: OffsetDateTime =
            sqlx::query_scalar("SELECT expires_at FROM sessions WHERE user_id = $1")
                .bind(handle)
                .fetch_one(&db)
                .await
                .unwrap();
        let lifetime = expires_at - OffsetDateTime::now_utc();
        let ttl = Duration::seconds(i64::try_from(SESSION_TTL_SECS).unwrap());
        assert!(
            ttl - Duration::minutes(1) < lifetime && lifetime <= ttl,
            "{lifetime}"
        );

        let again = finish_registration(&base, &started, &key).await;
        assert_eq!(again.status().as_u16(), 400);
        assert_eq!(
            as_json(&again.text().await.unwrap()),
            json!({ "error": "ceremony_expired" })
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_credential_that_is_already_a_passkey_registers_no_second_tenant() {
    in_fresh_schema(|db| async move {
        let base = serve_hub(db.clone()).await;
        let key = SoftAuthenticator::new();
        let first = start_registration(&base, "laptop").await;
        let registered = finish_registration(&base, &first, &key).await;
        assert_eq!(registered.status().as_u16(), 200);

        let second = start_registration(&base, "the same key again").await;
        let refused = finish_registration(&base, &second, &key).await;

        assert_eq!(refused.status().as_u16(), 409);
        assert!(refused.headers().get("set-cookie").is_none());
        assert_eq!(
            as_json(&refused.text().await.unwrap()),
            json!({ "error": "credential_exists" })
        );
        assert_eq!(users_with_id(&db, reserved_handle(&second)).await, 0);
        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(sessions, 1);
        let kept = find_passkey(&db, &raw_id(&key)).await.unwrap().unwrap();
        assert_eq!(kept.user_id.handle(), reserved_handle(&first));
        assert_eq!(kept.label, "laptop");
    })
    .await;
}

/// Enrols `key` as the Hub's own registration would have stored it, under a
/// status of the caller's choosing. Its sign-in assertions therefore verify,
/// and `status` is all that can stand between one of them and a session.
async fn enrol_verified(db: &Db, user: UserId, key: &SoftAuthenticator, status: Status) {
    let verifier = Verifier::new(&rp()).unwrap();
    let (options, registration) = verifier
        .start_registration(user.handle(), status.as_str(), &[])
        .unwrap();
    let registered = verifier
        .finish_registration(&key.create(&rp().origin, &options), registration)
        .unwrap();
    let new = NewPasskey {
        credential_id: registered.credential_id,
        public_key: registered.public_key.to_vec(),
        label: status.as_str().to_owned(),
        sign_count: registered.sign_count,
        credential: registered.credential,
    };
    insert_passkey(user, db, &new, status).await.unwrap();
}

/// A whole sign-in over HTTP: a started ceremony, finished with the assertion
/// `key` makes for it at `origin`.
async fn sign_in_at(base: &str, key: &SoftAuthenticator, origin: &str) -> reqwest::Response {
    let response = post_signed_out(base, "/v1/session/start", &json!({})).await;
    assert_eq!(response.status().as_u16(), 200);
    let started = as_json(&response.text().await.unwrap());
    let finish = json!({
        "ceremony_id": started["ceremony_id"],
        "credential": key.get(origin, &started["options"], None),
    });
    post_signed_out(base, "/v1/session/finish", &finish).await
}

/// What a browser would have kept of the response's `Set-Cookie`.
fn session_set_by(response: &reqwest::Response) -> SessionSecret {
    let set_cookie = response.headers()["set-cookie"].to_str().unwrap();
    let (pair, _attributes) = set_cookie.split_once("; ").unwrap();
    SessionSecret::from_cookie_header(pair).expect("the session cookie")
}

/// The one answer a refused sign-in gets, whatever it was refused for.
async fn assert_not_signed_in(response: reqwest::Response) {
    assert_eq!(response.status().as_u16(), 401);
    assert!(response.headers().get("set-cookie").is_none());
    assert_eq!(
        response.text().await.unwrap(),
        r#"{"error":"unauthenticated"}"#
    );
}

async fn sessions_by_credential(db: &Db) -> Vec<Vec<u8>> {
    sqlx::query_scalar("SELECT credential_id FROM sessions ORDER BY created_at, id")
        .fetch_all(db)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_tenant_that_signed_out_signs_back_in_with_its_passkey() {
    in_fresh_schema(|db| async move {
        let base = serve_hub(db.clone()).await;
        let key = SoftAuthenticator::new();
        let started = start_registration(&base, "laptop").await;
        let registered = finish_registration(&base, &started, &key).await;
        let first = session_set_by(&registered);
        assert_eq!(
            post_as(&base, "/v1/session/end", &first, &json!({})).await,
            (200, json!({}))
        );
        assert!(!session_is_live(&base, &first).await);
        let key = key.with_sign_count(5);

        let response = sign_in_at(&base, &key, &rp().origin).await;

        assert_eq!(response.status().as_u16(), 200);
        let session = session_set_by(&response);
        assert_ne!(session.cookie_value(), first.cookie_value());
        let signed_in = as_json(&response.text().await.unwrap());
        assert_eq!(
            signed_in,
            json!({
                "csrf_token": session.csrf_token(),
                "credential_id": key.credential_id(),
            })
        );
        assert_eq!(
            get_as(&base, "/v1/session", &session).await,
            (200, signed_in)
        );

        let stored = find_passkey(&db, &raw_id(&key)).await.unwrap().unwrap();
        assert_eq!(stored.user_id.handle(), reserved_handle(&started));
        assert_eq!(stored.sign_count, 5);
        let found = find_session(&db, &session.hash()).await.unwrap().unwrap();
        assert_eq!(
            (found.user_id, found.credential_id),
            (stored.user_id, raw_id(&key))
        );
    })
    .await;
}

/// The trusted passkey is the control: it is enrolled and asserts exactly as
/// `key` does, and signs in. `key`'s status is the only difference.
async fn a_valid_assertion_does_not_sign_in_a_passkey_that_is(status: Status) {
    in_fresh_schema(move |db| async move {
        let base = serve_hub(db.clone()).await;
        let user = create_user(&db).await.unwrap();
        let (trusted, key) = (SoftAuthenticator::new(), SoftAuthenticator::new());
        enrol_verified(&db, user, &trusted, Status::Trusted).await;
        enrol_verified(&db, user, &key, status).await;
        let key = key.with_sign_count(5);

        let control = sign_in_at(&base, &trusted, &rp().origin).await;
        assert_eq!(control.status().as_u16(), 200);

        assert_not_signed_in(sign_in_at(&base, &key, &rp().origin).await).await;

        assert_eq!(sessions_by_credential(&db).await, [raw_id(&trusted)]);
        let stored = find_passkey(&db, &raw_id(&key)).await.unwrap().unwrap();
        assert_eq!((stored.status, stored.sign_count), (status, 0));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_pending_passkeys_valid_assertion_is_401() {
    a_valid_assertion_does_not_sign_in_a_passkey_that_is(Status::Pending).await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_revoked_passkeys_valid_assertion_is_401() {
    a_valid_assertion_does_not_sign_in_a_passkey_that_is(Status::Revoked).await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_unknown_credential_and_a_trusted_ones_bad_assertion_are_the_same_401() {
    in_fresh_schema(|db| async move {
        let base = serve_hub(db.clone()).await;
        let user = create_user(&db).await.unwrap();
        let (trusted, stranger) = (SoftAuthenticator::new(), SoftAuthenticator::new());
        enrol_verified(&db, user, &trusted, Status::Trusted).await;

        assert_not_signed_in(sign_in_at(&base, &stranger, &rp().origin).await).await;
        assert_not_signed_in(sign_in_at(&base, &trusted, "https://evil.example").await).await;

        assert!(sessions_by_credential(&db).await.is_empty());
    })
    .await;
}

async fn start_addition(base: &str, session: &SessionSecret, label: &str) -> Value {
    let label = json!({ "label": label });
    let (status, started) = post_as(base, "/v1/passkeys/add/start", session, &label).await;
    assert_eq!(status, 200, "{started}");
    started
}

/// Finishes `started` as `session`, with a credential `key` makes at the
/// Hub's own origin.
async fn finish_addition(
    base: &str,
    session: &SessionSecret,
    started: &Value,
    key: &SoftAuthenticator,
) -> (u16, Value) {
    let finish = json!({
        "ceremony_id": started["ceremony_id"],
        "credential": key.create(&rp().origin, &started["options"]),
    });
    post_as(base, "/v1/passkeys/add/finish", session, &finish).await
}

/// What `GET /v1/passkeys` says of `key` to `session`, or `None` when it
/// lists no such passkey.
async fn listed_status(
    base: &str,
    session: &SessionSecret,
    key: &SoftAuthenticator,
) -> Option<String> {
    let (status, listed) = get_as(base, "/v1/passkeys", session).await;
    assert_eq!(status, 200, "{listed}");
    let passkeys = listed["passkeys"].as_array().expect("a `passkeys` array");
    let found = passkeys
        .iter()
        .find(|passkey| passkey["credential_id"] == key.credential_id())?;
    found["status"].as_str().map(str::to_owned)
}

/// Finishes the ceremony `started` over one of the tenant's passkeys with the
/// assertion `signer` makes over its challenge.
async fn finish_with(
    base: &str,
    path: &str,
    session: &SessionSecret,
    started: &Value,
    signer: &SoftAuthenticator,
) -> (u16, Value) {
    let finish = json!({
        "ceremony_id": started["ceremony_id"],
        "assertion": signer.assert(&rp(), &challenge_in(started)),
    });
    post_as(base, path, session, &finish).await
}

/// The whole life of a second passkey, with nothing enrolled behind the Hub's
/// back: every row here was written by a route, so each step is judged on the
/// state the steps before it left.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_added_passkey_is_pending_until_a_trusted_one_endorses_it_and_then_acts_as_one() {
    in_fresh_schema(|db| async move {
        let base = serve_hub(db.clone()).await;
        let (a, b) = (SoftAuthenticator::new(), SoftAuthenticator::new());
        let registering = start_registration(&base, "laptop").await;
        let registered = finish_registration(&base, &registering, &a).await;
        assert_eq!(registered.status().as_u16(), 200);
        let as_a = session_set_by(&registered);

        let adding = start_addition(&base, &as_a, "  phone ").await;
        let options = &adding["options"]["publicKey"];
        assert_eq!(
            options["user"]["id"],
            registering["options"]["publicKey"]["user"]["id"]
        );
        assert_eq!(
            options["excludeCredentials"],
            json!([{ "type": "public-key", "id": a.credential_id() }])
        );
        let (status, added) = finish_addition(&base, &as_a, &adding, &b).await;

        let stored = find_passkey(&db, &raw_id(&b)).await.unwrap().unwrap();
        assert_eq!(
            (status, added),
            (
                200,
                json!({
                    "passkey": {
                        "credential_id": b.credential_id(),
                        "label": "phone",
                        "status": "pending",
                        "created_at": stored.created_at.unix_timestamp(),
                        "current": false,
                    }
                })
            )
        );
        assert_eq!(stored.user_id.handle(), reserved_handle(&registering));
        assert_eq!(stored.public_key, b.public_key_cose());
        assert_eq!(stored.status, Status::Pending);
        let listed = |key| listed_status(&base, &as_a, key);
        assert_eq!(listed(&a).await.as_deref(), Some("trusted"));
        assert_eq!(listed(&b).await.as_deref(), Some("pending"));
        assert_eq!(
            finish_addition(&base, &as_a, &adding, &b).await,
            (400, json!({ "error": "ceremony_expired" }))
        );

        assert_not_signed_in(sign_in_at(&base, &b, &rp().origin).await).await;

        let endorse = "/v1/passkeys/endorse/finish";
        let (status, endorsing) = start_endorsement(&base, &as_a, &b.credential_id()).await;
        assert_eq!(status, 200, "{endorsing}");
        assert_eq!(
            finish_with(&base, endorse, &as_a, &endorsing, &b).await,
            (403, json!({ "error": "self_endorsement" }))
        );
        assert_eq!(listed(&b).await.as_deref(), Some("pending"));
        assert_not_signed_in(sign_in_at(&base, &b, &rp().origin).await).await;

        let (status, endorsing) = start_endorsement(&base, &as_a, &b.credential_id()).await;
        assert_eq!(status, 200, "{endorsing}");
        let (status, endorsed) = finish_with(&base, endorse, &as_a, &endorsing, &a).await;
        assert_eq!(
            (status, &endorsed["passkey"]["status"]),
            (200, &json!("trusted")),
            "{endorsed}"
        );

        let signed_in = sign_in_at(&base, &b, &rp().origin).await;
        assert_eq!(signed_in.status().as_u16(), 200);
        let as_b = session_set_by(&signed_in);
        assert_eq!(
            listed_status(&base, &as_b, &b).await.as_deref(),
            Some("trusted")
        );

        let (status, revoking) = start_revocation(&base, &as_b, &a.credential_id()).await;
        assert_eq!(status, 200, "{revoking}");
        let revoke = "/v1/passkeys/revoke/finish";
        let (status, revoked) = finish_with(&base, revoke, &as_b, &revoking, &b).await;
        assert_eq!(
            (status, &revoked["passkey"]["status"]),
            (200, &json!("revoked")),
            "{revoked}"
        );
        assert_eq!(status_of(&db, &a).await, Status::Revoked);
        assert!(!session_is_live(&base, &as_a).await);

        assert_eq!(
            start_revocation(&base, &as_b, &b.credential_id()).await,
            (409, json!({ "error": "last_passkey" }))
        );
        assert_eq!(status_of(&db, &b).await, Status::Trusted);
    })
    .await;
}

async fn passkeys_of(db: &Db, handle: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM passkeys WHERE user_id = $1")
        .bind(handle)
        .fetch_one(db)
        .await
        .unwrap()
}

/// Two tenants, each signed in with its one passkey. Nothing the first
/// presents at the add routes leaves it a second.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_passkey_is_added_only_once_and_only_under_the_ceremony_its_own_tenant_started() {
    in_fresh_schema(|db| async move {
        let base = serve_hub(db.clone()).await;
        let (mine, theirs) = (SoftAuthenticator::new(), SoftAuthenticator::new());
        let registering = start_registration(&base, "laptop").await;
        let me = session_set_by(&finish_registration(&base, &registering, &mine).await);
        let registering_them = start_registration(&base, "their laptop").await;
        let them = session_set_by(&finish_registration(&base, &registering_them, &theirs).await);
        let exists = (409, json!({ "error": "credential_exists" }));
        let expired = (400, json!({ "error": "ceremony_expired" }));

        for label in ["", "   ", &"x".repeat(65)] {
            let body = json!({ "label": label });
            assert_eq!(
                post_as(&base, "/v1/passkeys/add/start", &me, &body).await,
                (400, json!({ "error": "invalid_request" })),
                "{label:?}"
            );
        }

        let adding = start_addition(&base, &me, "the same key again").await;
        assert_eq!(finish_addition(&base, &me, &adding, &mine).await, exists);
        let adding = start_addition(&base, &me, "another tenant's key").await;
        assert_eq!(finish_addition(&base, &me, &adding, &theirs).await, exists);
        let kept = find_passkey(&db, &raw_id(&theirs)).await.unwrap().unwrap();
        assert_eq!(kept.user_id.handle(), reserved_handle(&registering_them));
        assert_eq!(
            (kept.label.as_str(), kept.status),
            ("their laptop", Status::Trusted)
        );

        let fresh = SoftAuthenticator::new();
        let adding = start_addition(&base, &me, "phone").await;
        assert_eq!(
            finish_addition(&base, &them, &adding, &fresh).await,
            expired
        );
        assert_eq!(finish_addition(&base, &me, &adding, &fresh).await, expired);

        let adding = start_addition(&base, &me, "phone").await;
        let as_a_new_tenant = finish_registration(&base, &adding, &fresh).await;
        assert!(as_a_new_tenant.headers().get("set-cookie").is_none());
        assert_eq!(
            (
                as_a_new_tenant.status().as_u16(),
                as_json(&as_a_new_tenant.text().await.unwrap())
            ),
            expired
        );
        let registering = start_registration(&base, "phone").await;
        assert_eq!(
            finish_addition(&base, &me, &registering, &fresh).await,
            expired
        );

        assert_eq!(find_passkey(&db, &raw_id(&fresh)).await.unwrap(), None);
        let my_handle = find_passkey(&db, &raw_id(&mine)).await.unwrap().unwrap();
        assert_eq!(passkeys_of(&db, my_handle.user_id.handle()).await, 1);
        assert_eq!(passkeys_of(&db, kept.user_id.handle()).await, 1);
    })
    .await;
}

/// Enrols, behind the Hub's back, `count` passkeys nobody holds.
async fn enrol_others(db: &Db, user: UserId, count: usize, status: Status) {
    for _ in 0..count {
        enrol(db, user, &SoftAuthenticator::new(), status).await;
    }
}

/// The finish is refused on what the tenant holds when it arrives, not when
/// its ceremony started: both ceremonies here start one short of the limit.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_tenant_holding_the_most_passkeys_it_may_is_refused_another_until_one_is_revoked() {
    in_fresh_schema(|db| async move {
        let base = serve_hub(db.clone()).await;
        let mine = SoftAuthenticator::new();
        let registering = start_registration(&base, "laptop").await;
        let me = session_set_by(&finish_registration(&base, &registering, &mine).await);
        let found = find_passkey(&db, &raw_id(&mine)).await.unwrap().unwrap();
        let user = found.user_id;
        let too_many = (409, json!({ "error": "too_many_passkeys" }));
        let start = "/v1/passkeys/add/start";
        let label = json!({ "label": "phone" });
        let added_pending = |answer: (u16, Value)| {
            assert_eq!(
                (answer.0, &answer.1["passkey"]["status"]),
                (200, &json!("pending")),
                "{answer:?}"
            );
        };

        let spare = SoftAuthenticator::new();
        enrol(&db, user, &spare, Status::Pending).await;
        enrol_others(&db, user, MAX_ACTIVE_PASSKEYS - 3, Status::Pending).await;
        enrol_others(&db, user, 3, Status::Revoked).await;
        let (sixteenth, late) = (SoftAuthenticator::new(), SoftAuthenticator::new());
        let adding = start_addition(&base, &me, "the sixteenth").await;
        let adding_late = start_addition(&base, &me, "one too many").await;
        added_pending(finish_addition(&base, &me, &adding, &sixteenth).await);
        let rows = passkeys_of(&db, user.handle()).await;
        assert_eq!(usize::try_from(rows).unwrap(), MAX_ACTIVE_PASSKEYS + 3);

        assert_eq!(post_as(&base, start, &me, &label).await, too_many);
        assert_eq!(
            finish_addition(&base, &me, &adding_late, &late).await,
            too_many
        );
        assert_eq!(find_passkey(&db, &raw_id(&late)).await.unwrap(), None);
        assert_eq!(passkeys_of(&db, user.handle()).await, rows);

        let (status, revoking) = start_revocation(&base, &me, &spare.credential_id()).await;
        assert_eq!(status, 200, "{revoking}");
        let revoke = "/v1/passkeys/revoke/finish";
        let (status, revoked) = finish_with(&base, revoke, &me, &revoking, &mine).await;
        assert_eq!(
            (status, &revoked["passkey"]["status"]),
            (200, &json!("revoked")),
            "{revoked}"
        );
        let adding = start_addition(&base, &me, "in the revoked one's place").await;
        added_pending(finish_addition(&base, &me, &adding, &late).await);
        assert_eq!(post_as(&base, start, &me, &label).await, too_many);
        assert_eq!(passkeys_of(&db, user.handle()).await, rows + 1);
    })
    .await;
}

/// Each finish locks the tenant's rows and then counts them. Counting the
/// rows its lock statement returned instead, every finish that waited counts
/// the fifteen it locked, without the one the first inserted, and all four
/// are stored.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn concurrent_additions_one_short_of_the_limit_store_exactly_one() {
    in_fresh_schema(|db| async move {
        let base = serve_hub(db.clone()).await;
        let too_many = (409, json!({ "error": "too_many_passkeys" }));
        for round in 0..8 {
            let user = create_user(&db).await.unwrap();
            let mine = SoftAuthenticator::new();
            enrol(&db, user, &mine, Status::Trusted).await;
            enrol_others(&db, user, MAX_ACTIVE_PASSKEYS - 2, Status::Pending).await;
            let me = open_session(&db, user, &mine).await;
            let keys = [(); 4].map(|()| SoftAuthenticator::new());
            let mut started = Vec::new();
            for _ in &keys {
                started.push(start_addition(&base, &me, "phone").await);
            }

            let finish = |nth: usize| finish_addition(&base, &me, &started[nth], &keys[nth]);
            let answers: [(u16, Value); 4] =
                tokio::join!(finish(0), finish(1), finish(2), finish(3)).into();

            let stored = answers.iter().filter(|(status, _)| *status == 200).count();
            let refused = answers.iter().filter(|answer| **answer == too_many).count();
            assert_eq!((stored, refused), (1, 3), "round {round}: {answers:?}");
            let rows = passkeys_of(&db, user.handle()).await;
            assert_eq!(
                usize::try_from(rows).unwrap(),
                MAX_ACTIVE_PASSKEYS,
                "round {round}"
            );
        }
    })
    .await;
}
