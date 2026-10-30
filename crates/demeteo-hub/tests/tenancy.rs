//! Tenant isolation against a real Postgres. Needs the `postgres` service from
//! `deploy/hub/compose.yaml` as `deploy/hub/README.md` describes, and
//! `DEMETEO_HUB_TEST_DATABASE_URL` pointing at it.

mod support;

use demeteo_hub::auth::policy::Status;
use demeteo_hub::pairing::grant::{
    refresh_change, FamilyChange, FreshPair, RefreshRefusal, TokenSecret,
};
use demeteo_hub::pairing::jwk::DeviceJwk;
use demeteo_hub::pairing::testing::SoftDeviceKey;
use demeteo_hub::socket::connections::Connections;
use demeteo_hub::socket::port::SocketStore;
use demeteo_hub::socket::postgres::PostgresStore;
use demeteo_hub::socket::requests::OutcomeChange;
use demeteo_hub::socket::SNAPSHOT_KIND;
use demeteo_hub::store::attachments::{
    bind_to_request, consumed, delete_expired, fetchable, insert_attachment, staged_of,
    NewAttachment, WrappedKey,
};
use demeteo_hub::store::device_keys::{instance_of_key, NewDeviceKey};
use demeteo_hub::store::instances::{
    find_instance, granted_scopes, insert_instance, list_instances, pair_instance,
    record_departure, record_hello, Instance, InstanceKind, NewInstance, NewInstanceLabels,
    NewPairing,
};
use demeteo_hub::store::passkeys::{insert_passkey, list_passkeys, NewPasskey};
use demeteo_hub::store::pool::{create_user, tenants};
use demeteo_hub::store::requests::{
    find_request, insert_request, insert_request_binding, list_queued_requests,
    mark_request_expired, mark_request_sent, record_outcome, NewRequest, QueuedEnvelope, RequestId,
    RequestRecord, RequestState, RequestStatus,
};
use demeteo_hub::store::run_mirror::{mirror_event, mirror_of, MirrorRecord, NewEvent};
use demeteo_hub::store::sessions::{
    delete_session, delete_sessions_of_credential, find_session, insert_session, NewSession,
    Session, SessionHash,
};
use demeteo_hub::store::snapshots::{read_snapshot, replace_snapshot};
use demeteo_hub::store::tokens::{
    find_token_by_hash, insert_token, rotate_family, FamilyWritten, NewToken, TokenHash, TokenKind,
    TokenRecord,
};
use demeteo_hub::store::{Db, InstanceId, StoreError, UserId};
use demeteo_hub_protocol::{
    CancelFeature, Hello, RefusalReason, RequestOutcome, RequestPayload, RunEvent, RunEventKind,
    RunEventPayload, Scope, SignedRequest, Snapshot, PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use support::in_fresh_schema;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const FOREIGN_KEY_VIOLATION: &str = "23503";
const EXPECTED_TABLES: [&str; 12] = [
    "attachments",
    "device_keys",
    "instance_requests",
    "instances",
    "passkey_endorsements",
    "passkey_revocations",
    "passkeys",
    "run_mirror",
    "sessions",
    "snapshots",
    "tokens",
    "users",
];

fn new_instance(name: &str) -> NewInstance {
    NewInstance {
        kind: InstanceKind::Desktop,
        id: InstanceId::generate(),
        name: name.to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        version: "1.2.0".to_owned(),
        granted_scopes: vec!["projects:read".to_owned()],
    }
}

async fn user_with_instance(db: &Db, name: &str) -> (UserId, InstanceId) {
    let user = create_user(db).await.unwrap();
    let new = new_instance(name);
    insert_instance(user, db, &new).await.unwrap();
    (user, new.id)
}

fn new_session(secret: &[u8], credential_id: &[u8], lifetime: Duration) -> NewSession {
    NewSession {
        hash: SessionHash::digest(secret),
        credential_id: credential_id.to_vec(),
        expires_at: OffsetDateTime::now_utc() + lifetime,
    }
}

fn new_passkey(credential_id: &[u8]) -> NewPasskey {
    NewPasskey {
        credential_id: credential_id.to_vec(),
        public_key: vec![0xa5; 77],
        label: "laptop".to_owned(),
        sign_count: 0,
        credential: "{}".to_owned(),
    }
}

async fn enrol_trusted(db: &Db, user: UserId, credential_id: &[u8]) {
    insert_passkey(user, db, &new_passkey(credential_id), Status::Trusted)
        .await
        .unwrap();
}

fn violation_code(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()?.code().map(|c| c.into_owned())
}

const JKT: &str = "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs";
const OTHER_JKT: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";

fn unix_now() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap()
}

/// What a pairing grant leaves behind: the two rows of a pair issued now to
/// the key `JKT`, in `family_id`, and the `device_keys` row of `JKT` that a
/// presentation of either queues on.
async fn pair_family(db: &Db, user: UserId, family_id: Uuid) -> FreshPair {
    let instance = new_instance("laptop");
    insert_instance(user, db, &instance).await.unwrap();
    sqlx::query(
        "INSERT INTO device_keys (id, user_id, instance_id, public_jwk, jkt) \
         VALUES ($1, $2, $3, '{}'::jsonb, $4) ON CONFLICT (user_id, jkt) DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(instance.id)
    .bind(JKT)
    .execute(db)
    .await
    .unwrap();
    let pair = FreshPair::generate().unwrap();
    let rows = pair.rows(family_id, JKT, unix_now()).unwrap();
    for new in [&rows.access, &rows.refresh] {
        insert_token(user, db, new).await.unwrap();
    }
    pair
}

/// Presents `token` as a refresh: `refresh_change` decides, and a rotation
/// would issue `fresh`.
async fn present(
    db: &Db,
    token: &TokenSecret,
    proof_jkt: &str,
    fresh: &FreshPair,
) -> FamilyWritten<Result<(), RefreshRefusal>> {
    let hash = token.hash();
    rotate_family(db, &hash, |family| {
        let decision = refresh_change(family, &hash, proof_jkt, unix_now(), fresh).unwrap();
        (decision.change, decision.verdict)
    })
    .await
    .unwrap()
}

/// As [`present`], with the number of rows `refresh_change` was shown beside
/// its verdict.
async fn present_counting(
    db: &Db,
    token: &TokenSecret,
    proof_jkt: &str,
    fresh: &FreshPair,
) -> FamilyWritten<(usize, Result<(), RefreshRefusal>)> {
    let hash = token.hash();
    rotate_family(db, &hash, |family| {
        let decision = refresh_change(family, &hash, proof_jkt, unix_now(), fresh).unwrap();
        (decision.change, (family.len(), decision.verdict))
    })
    .await
    .unwrap()
}

/// Adds `count` rows of `kind` to `family_id`, bound to `jkt` and expiring
/// `expires_in` from now, retired already when `retired`.
async fn aged_rows(
    db: &Db,
    user: UserId,
    (family_id, jkt): (Uuid, &str),
    kind: TokenKind,
    expires_in: Duration,
    retired: bool,
    count: usize,
) -> Vec<TokenSecret> {
    let mut secrets = Vec::with_capacity(count);
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        let secret = TokenSecret::generate().unwrap();
        let new = NewToken {
            kind,
            hash: secret.hash(),
            family_id,
            jkt: jkt.to_owned(),
            expires_at: OffsetDateTime::now_utc() + expires_in,
        };
        ids.push(insert_token(user, db, &new).await.unwrap().id);
        secrets.push(secret);
    }
    if retired {
        sqlx::query("UPDATE tokens SET retired_at = now() WHERE user_id = $1 AND id = ANY($2)")
            .bind(user)
            .bind(&ids)
            .execute(db)
            .await
            .unwrap();
    }
    secrets
}

async fn stored(db: &Db, token: &TokenSecret) -> TokenRecord {
    find_token_by_hash(db, &token.hash())
        .await
        .unwrap()
        .unwrap()
}

async fn is_stored(db: &Db, token: &TokenSecret) -> bool {
    let found = find_token_by_hash(db, &token.hash()).await.unwrap();
    found.is_some()
}

fn device_key(key: &SoftDeviceKey) -> NewDeviceKey {
    let jwk = DeviceJwk::parse(&key.jwk()).unwrap();
    NewDeviceKey {
        public_jwk: jwk.canonical_json(),
        jkt: jwk.thumbprint(),
    }
}

fn labels([name, os, arch, version]: [&str; 4]) -> NewInstanceLabels {
    NewInstanceLabels {
        name: name.to_owned(),
        os: os.to_owned(),
        arch: arch.to_owned(),
        version: version.to_owned(),
    }
}

const LAPTOP: [&str; 4] = ["laptop", "linux", "x86_64", "1.2.0"];

/// A pairing of `key` as the grant hands it to the store, in a family of its
/// own, with the pair whose rows it carries.
fn new_pairing(key: &NewDeviceKey, named: [&str; 4], scopes: &[Scope]) -> (NewPairing, FreshPair) {
    let family_id = Uuid::new_v4();
    let pair = FreshPair::generate().unwrap();
    let rows = pair.rows(family_id, &key.jkt, unix_now()).unwrap();
    let new = NewPairing {
        kind: InstanceKind::Desktop,
        instance: labels(named),
        device_key: key.clone(),
        scopes: scopes.to_vec(),
        family_id,
        access: rows.access,
        refresh: rows.refresh,
    };
    (new, pair)
}

async fn instance_of(db: &Db, user: UserId, id: InstanceId) -> Instance {
    let listed = list_instances(user, db).await.unwrap();
    listed.into_iter().find(|i| i.id == id).unwrap()
}

/// The tenant's `device_keys` rows as `(instance_id, public_jwk, jkt, revoked)`.
async fn device_keys_of(db: &Db, user: UserId) -> Vec<(InstanceId, String, String, bool)> {
    sqlx::query_as(
        "SELECT instance_id, public_jwk::text, jkt, revoked_at IS NOT NULL \
         FROM device_keys WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .unwrap()
}

fn hello(install_id: &str, scopes: &[Scope]) -> Hello {
    Hello {
        v: PROTOCOL_VERSION,
        install_id: install_id.to_owned(),
        app_version: "1.5.0".to_owned(),
        os: "macos".to_owned(),
        arch: "aarch64".to_owned(),
        scopes: scopes.to_vec(),
    }
}

async fn token_rows_of(db: &Db, user: UserId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM tokens WHERE user_id = $1")
        .bind(user)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn is_live(db: &Db, token: &TokenSecret) -> bool {
    stored(db, token).await.retired_at.is_none()
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn list_instances_never_returns_another_users_instance() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        let nobody = create_user(&db).await.unwrap();

        let mut a_ids = BTreeSet::new();
        for name in ["a-laptop", "a-server"] {
            let new = new_instance(name);
            insert_instance(a, &db, &new).await.unwrap();
            a_ids.insert(new.id.0);
        }
        let b_new = new_instance("b-laptop");
        insert_instance(b, &db, &b_new).await.unwrap();

        let seen_by_a = list_instances(a, &db).await.unwrap();
        assert_eq!(
            seen_by_a.iter().map(|i| i.id.0).collect::<BTreeSet<_>>(),
            a_ids
        );
        assert!(seen_by_a.iter().all(|i| i.user_id == a));
        assert!(!seen_by_a.iter().any(|i| i.id == b_new.id));

        let seen_by_b = list_instances(b, &db).await.unwrap();
        assert_eq!(seen_by_b.len(), 1);
        assert_eq!(seen_by_b[0].id, b_new.id);

        assert!(list_instances(nobody, &db).await.unwrap().is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn every_table_but_users_has_a_non_null_user_id() {
    in_fresh_schema(|db| async move {
        let tables: BTreeSet<String> = sqlx::query_scalar(
            "SELECT table_name::text FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'",
        )
        .fetch_all(&db)
        .await
        .unwrap()
        .into_iter()
        .filter(|name: &String| name != "_sqlx_migrations")
        .collect();
        assert_eq!(
            tables,
            EXPECTED_TABLES
                .iter()
                .map(|t| t.to_string())
                .collect::<BTreeSet<_>>()
        );

        for table in tables.iter().filter(|t| *t != "users") {
            let nullable: Option<String> = sqlx::query_scalar(
                "SELECT is_nullable::text FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = $1 \
                 AND column_name = 'user_id'",
            )
            .bind(table)
            .fetch_optional(&db)
            .await
            .unwrap();
            assert_eq!(nullable.as_deref(), Some("NO"), "{table}.user_id");
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn child_row_naming_another_users_instance_is_rejected() {
    in_fresh_schema(|db| async move {
        let (a, _) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;

        let insert = |owner: UserId| {
            let db = db.clone();
            async move {
                sqlx::query(
                    "INSERT INTO device_keys (id, user_id, instance_id, public_jwk, jkt) \
                     VALUES ($1, $2, $3, '{}'::jsonb, $4)",
                )
                .bind(Uuid::new_v4())
                .bind(owner)
                .bind(b_instance)
                .bind(Uuid::new_v4().to_string())
                .execute(&db)
                .await
            }
        };

        insert(b)
            .await
            .expect("the owner may attach a key to its own instance");
        let err = insert(a).await.expect_err("another tenant must not");
        assert_eq!(
            violation_code(&err).as_deref(),
            Some(FOREIGN_KEY_VIOLATION),
            "{err}"
        );
    })
    .await;
}

fn new_attachment(instance: InstanceId, expires_at: u64) -> NewAttachment {
    NewAttachment {
        id: Uuid::new_v4(),
        instance_id: instance,
        ciphertext: vec![1, 2, 3],
        wrapped_key: WrappedKey::parse("vault:v1:wrapped").unwrap(),
        sealed_digest: vec![9; 60],
        expires_at,
    }
}

async fn bound(
    user: UserId,
    db: &Db,
    instance: InstanceId,
    ids: &[Uuid],
    request: &NewRequest,
    now: u64,
) -> u64 {
    let mut tx = db.begin().await.unwrap();
    let bound = bind_to_request(
        user,
        &mut tx,
        instance,
        ids,
        &request.id,
        request.expires_at,
        now,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    bound
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn attachments_are_invisible_to_other_users() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, _) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let attachment = new_attachment(a_instance, now + 3600);

        assert!(insert_attachment(b, &db, &attachment).await.is_err());
        let stored = insert_attachment(a, &db, &attachment).await.unwrap();
        assert_eq!(stored.id, attachment.id);
        assert_eq!(stored.request_id, None);
        assert_eq!(stored.wrapped_key, "vault:v1:wrapped");
        assert_eq!(stored.sealed_digest, attachment.sealed_digest);
        assert_eq!(stored.size_bytes, 3);
        assert_eq!(stored.expires_at, now + 3600);
        let sha256: Vec<u8> = sqlx::query_scalar("SELECT sha256 FROM attachments WHERE id = $1")
            .bind(attachment.id)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(sha256, Sha256::digest(&attachment.ciphertext).as_slice());

        let ids = [attachment.id];
        assert_eq!(
            staged_of(a, &db, a_instance, &ids).await.unwrap(),
            vec![stored]
        );
        assert!(staged_of(b, &db, a_instance, &ids)
            .await
            .unwrap()
            .is_empty());
        let request = new_request([7; 16], a_instance, now + 300);
        insert_request(a, &db, a_instance, &request).await.unwrap();
        assert_eq!(bound(b, &db, a_instance, &ids, &request, now).await, 0);
        assert_eq!(bound(a, &db, a_instance, &ids, &request, now).await, 1);
        assert!(fetchable(b, &db, a_instance, attachment.id, now)
            .await
            .unwrap()
            .is_none());
        assert_eq!(delete_expired(b, &db, now + 7200).await.unwrap(), 0);
        assert!(!consumed(b, &db, a_instance, attachment.id, now)
            .await
            .unwrap());
        assert!(consumed(a, &db, a_instance, attachment.id, now)
            .await
            .unwrap());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn the_dispatch_read_answers_what_the_insert_returned_and_the_fetch_adds_the_ciphertext() {
    in_fresh_schema(|db| async move {
        let (a, instance) = user_with_instance(&db, "laptop").await;
        let now = unix_now();
        let new = new_attachment(instance, now + 3600);
        let stored = insert_attachment(a, &db, &new).await.unwrap();
        assert_eq!(
            staged_of(a, &db, instance, &[new.id]).await.unwrap(),
            vec![stored]
        );

        let request = new_request([7; 16], instance, now + 300);
        insert_request(a, &db, instance, &request).await.unwrap();
        assert_eq!(bound(a, &db, instance, &[new.id], &request, now).await, 1);
        let read = staged_of(a, &db, instance, &[new.id]).await.unwrap();
        let fetched = fetchable(a, &db, instance, new.id, now)
            .await
            .unwrap()
            .expect("bound to a queued request");
        assert_eq!(fetched.ciphertext, new.ciphertext);
        assert_eq!(vec![fetched.meta()], read);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_request_is_queued_only_with_every_attachment_it_binds() {
    in_fresh_schema(|db| async move {
        let (a, instance) = user_with_instance(&db, "laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let staged = new_attachment(instance, now + 3600);
        let foreign = new_attachment(b_instance, now + 3600);
        insert_attachment(a, &db, &staged).await.unwrap();
        insert_attachment(b, &db, &foreign).await.unwrap();
        let request = new_request([7; 16], instance, now + 300);

        for refused in [vec![staged.id, foreign.id], vec![staged.id, staged.id]] {
            let err = insert_request_binding(a, &db, instance, &request, &refused, now)
                .await
                .unwrap_err();
            assert!(matches!(err, StoreError::AttachmentUnavailable), "{err}");
            assert!(find_request(a, &db, instance, &request.id)
                .await
                .unwrap()
                .is_none());
            let rows = staged_of(a, &db, instance, &[staged.id]).await.unwrap();
            assert_eq!(rows[0].request_id, None);
            assert_eq!(rows[0].expires_at, now + 3600);
        }

        insert_request_binding(a, &db, instance, &request, &[staged.id], now)
            .await
            .unwrap();
        assert!(find_request(a, &db, instance, &request.id)
            .await
            .unwrap()
            .is_some());
        let rows = staged_of(a, &db, instance, &[staged.id]).await.unwrap();
        assert_eq!(rows[0].request_id, Some(request.id));
        assert_eq!(rows[0].expires_at, request.expires_at);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_attachment_is_bound_once_and_fetchable_only_while_its_request_is_queued() {
    in_fresh_schema(|db| async move {
        let (a, instance) = user_with_instance(&db, "laptop").await;
        let now = unix_now();
        let staged = new_attachment(instance, now + 3600);
        let lapsed = new_attachment(instance, now);
        for new in [&staged, &lapsed] {
            insert_attachment(a, &db, new).await.unwrap();
        }
        let request = new_request([7; 16], instance, now + 300);
        let other = new_request([8; 16], instance, now + 300);
        for new in [&request, &other] {
            insert_request(a, &db, instance, new).await.unwrap();
        }

        assert!(fetchable(a, &db, instance, staged.id, now)
            .await
            .unwrap()
            .is_none());
        let both = [staged.id, lapsed.id];
        assert_eq!(bound(a, &db, instance, &both, &request, now).await, 1);
        assert_eq!(bound(a, &db, instance, &[staged.id], &other, now).await, 0);

        let fetched = fetchable(a, &db, instance, staged.id, now)
            .await
            .unwrap()
            .expect("bound to a queued request");
        assert_eq!(fetched.request_id, Some(request.id));
        assert_eq!(fetched.expires_at, request.expires_at);
        assert!(fetchable(a, &db, instance, staged.id, request.expires_at)
            .await
            .unwrap()
            .is_none());
        assert!(fetchable(a, &db, instance, lapsed.id, now)
            .await
            .unwrap()
            .is_none());

        mark_request_expired(a, &db, instance, &request.id)
            .await
            .unwrap();
        assert!(fetchable(a, &db, instance, staged.id, now)
            .await
            .unwrap()
            .is_none());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_attachment_is_consumed_only_while_its_request_is_queued() {
    in_fresh_schema(|db| async move {
        let (a, instance) = user_with_instance(&db, "laptop").await;
        let now = unix_now();
        let mut bound_ids = Vec::new();
        for n in 1..=4 {
            let new = new_attachment(instance, now + 3600);
            insert_attachment(a, &db, &new).await.unwrap();
            let request = new_request([n; 16], instance, now + 300);
            insert_request(a, &db, instance, &request).await.unwrap();
            assert_eq!(bound(a, &db, instance, &[new.id], &request, now).await, 1);
            bound_ids.push((new.id, request.id));
        }
        let [answered, expired, lapsed, live] = bound_ids[..] else {
            unreachable!("four were bound");
        };
        let accepted = |_: RequestState| Some(RequestOutcome::Accepted);
        assert!(record_outcome(a, &db, instance, &answered.1, now, accepted)
            .await
            .unwrap());
        assert!(mark_request_expired(a, &db, instance, &expired.1)
            .await
            .unwrap());

        assert!(!consumed(a, &db, instance, answered.0, now).await.unwrap());
        for (name, id, at) in [
            ("marked expired", expired.0, now),
            ("past its expires_at", lapsed.0, now + 300),
        ] {
            assert!(!consumed(a, &db, instance, id, at).await.unwrap(), "{name}");
            let kept = staged_of(a, &db, instance, &[id]).await.unwrap();
            assert_eq!(kept.len(), 1, "{name} was deleted");
        }
        assert!(consumed(a, &db, instance, live.0, now).await.unwrap());
        assert!(!consumed(a, &db, instance, live.0, now).await.unwrap());
    })
    .await;
}

/// `b`'s request carries the same id bytes as the one answered: only the
/// tenant and the instance tell its row apart.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn recording_an_outcome_deletes_only_that_requests_attachments() {
    in_fresh_schema(|db| async move {
        let (a, instance) = user_with_instance(&db, "laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let mut rows = Vec::new();
        for (user, at, n) in [(a, instance, 1), (a, instance, 2), (b, b_instance, 1)] {
            let new = new_attachment(at, now + 3600);
            insert_attachment(user, &db, &new).await.unwrap();
            let request = new_request([n; 16], at, now + 300);
            insert_request(user, &db, at, &request).await.unwrap();
            assert_eq!(bound(user, &db, at, &[new.id], &request, now).await, 1);
            rows.push((new.id, request.id));
        }
        let [answered, other, strangers] = rows[..] else {
            unreachable!("three were bound");
        };
        let unbound = new_attachment(instance, now + 3600);
        insert_attachment(a, &db, &unbound).await.unwrap();
        let own = [answered.0, other.0, unbound.id];

        let keep = |_: RequestState| None;
        assert!(record_outcome(a, &db, instance, &answered.1, now, keep)
            .await
            .unwrap());
        let accepted = |_: RequestState| Some(RequestOutcome::Accepted);
        assert!(!record_outcome(a, &db, instance, &[9; 16], now, accepted)
            .await
            .unwrap());
        assert_eq!(staged_of(a, &db, instance, &own).await.unwrap().len(), 3);

        assert!(record_outcome(a, &db, instance, &answered.1, now, accepted)
            .await
            .unwrap());
        let left = staged_of(a, &db, instance, &own).await.unwrap();
        let mut left: Vec<Uuid> = left.iter().map(|row| row.id).collect();
        left.sort();
        let mut expected = vec![other.0, unbound.id];
        expected.sort();
        assert_eq!(left, expected);
        let theirs = staged_of(b, &db, b_instance, &[strangers.0]).await.unwrap();
        assert_eq!(theirs.len(), 1);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn the_sweep_deletes_one_tenants_expired_attachments_per_call() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let lapsed = new_attachment(a_instance, now);
        let waiting = new_attachment(a_instance, now + 1);
        let b_lapsed = new_attachment(b_instance, now - 1);
        insert_attachment(a, &db, &lapsed).await.unwrap();
        insert_attachment(a, &db, &waiting).await.unwrap();
        insert_attachment(b, &db, &b_lapsed).await.unwrap();

        let mut tenants = tenants(&db).await.unwrap();
        tenants.sort_by_key(|user| user.handle());
        let mut expected = vec![a, b];
        expected.sort_by_key(|user| user.handle());
        assert_eq!(tenants, expected);

        assert_eq!(delete_expired(a, &db, now).await.unwrap(), 1);
        let left = staged_of(a, &db, a_instance, &[lapsed.id, waiting.id])
            .await
            .unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, waiting.id);
        assert_eq!(
            staged_of(b, &db, b_instance, &[b_lapsed.id])
                .await
                .unwrap()
                .len(),
            1
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_token_found_by_hash_names_its_owner_and_its_kind() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        let family_id = Uuid::new_v4();

        for (owner, kind, secret) in [
            (a, TokenKind::Access, b"a-access-token".as_slice()),
            (a, TokenKind::Refresh, b"a-refresh-token".as_slice()),
            (b, TokenKind::Refresh, b"b-refresh-token".as_slice()),
        ] {
            let new = NewToken {
                kind,
                hash: TokenHash::digest(secret),
                family_id,
                jkt: "thumbprint".to_owned(),
                expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
            };
            let inserted = insert_token(owner, &db, &new).await.unwrap();

            let found = find_token_by_hash(&db, &new.hash).await.unwrap().unwrap();
            assert_eq!(found.user_id, owner);
            assert_eq!(found.kind, kind);
            assert_eq!(found.hash, new.hash);
            assert_eq!(found.id, inserted.id);
            assert_eq!(found.family_id, family_id);
            assert_eq!(found.retired_at, None);
        }

        let retired = TokenHash::digest(b"a-access-token");
        let done = sqlx::query(
            "UPDATE tokens SET retired_at = now() WHERE user_id = $1 AND token_hash = $2",
        )
        .bind(a)
        .bind(Sha256::digest(b"a-access-token").as_slice())
        .execute(&db)
        .await
        .unwrap();
        assert_eq!(done.rows_affected(), 1);
        let found = find_token_by_hash(&db, &retired).await.unwrap().unwrap();
        assert_eq!(found.user_id, a);
        assert!(found.retired_at.is_some());

        let unknown = TokenHash::digest(b"never-issued");
        assert_eq!(find_token_by_hash(&db, &unknown).await.unwrap(), None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_rotation_retires_the_presented_row_and_inserts_the_pair_in_its_family() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let family_id = Uuid::new_v4();
        let first = pair_family(&db, user, family_id).await;
        let presented = stored(&db, &first.refresh).await;
        let issued_with_it = stored(&db, &first.access).await;

        let fresh = FreshPair::generate().unwrap();
        let written = present(&db, &first.refresh, JKT, &fresh).await;

        assert_eq!(written.answer, Ok(()));
        let [retired] = written.retired.as_slice() else {
            panic!("{written:?}");
        };
        assert!(retired.retired_at.is_some());
        let presented_then_retired = TokenRecord {
            retired_at: retired.retired_at,
            ..presented
        };
        assert_eq!(*retired, presented_then_retired);

        let [access, refresh] = written.inserted.as_slice() else {
            panic!("{written:?}");
        };
        assert_eq!(access.kind, TokenKind::Access);
        assert_eq!(access.hash, fresh.access.hash());
        assert_eq!(refresh.kind, TokenKind::Refresh);
        assert_eq!(refresh.hash, fresh.refresh.hash());
        for row in [access, refresh] {
            assert_eq!(row.user_id, user);
            assert_eq!(row.family_id, family_id);
            assert_eq!(row.jkt, JKT);
            assert_eq!(row.retired_at, None);
        }

        assert_eq!(stored(&db, &first.refresh).await, *retired);
        assert_eq!(stored(&db, &fresh.access).await, *access);
        assert_eq!(stored(&db, &fresh.refresh).await, *refresh);
        assert_eq!(stored(&db, &first.access).await, issued_with_it);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_reuse_verdicts_change_is_committed_and_retires_every_live_row() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let first = pair_family(&db, user, Uuid::new_v4()).await;
        let second = FreshPair::generate().unwrap();
        assert_eq!(
            present(&db, &first.refresh, JKT, &second).await.answer,
            Ok(())
        );
        let used = stored(&db, &first.refresh).await;
        let live = [&first.access, &second.access, &second.refresh];
        let never_issued = FreshPair::generate().unwrap();

        let refused = present(&db, &first.refresh, OTHER_JKT, &never_issued).await;
        assert_eq!(refused.answer, Err(RefreshRefusal::KeyMismatch));
        assert_eq!((refused.retired.len(), refused.inserted.len()), (0, 0));
        for token in live {
            assert_eq!(stored(&db, token).await.retired_at, None);
        }

        let reused = present(&db, &first.refresh, JKT, &never_issued).await;
        assert_eq!(reused.answer, Err(RefreshRefusal::Reused));
        assert_eq!((reused.retired.len(), reused.inserted.len()), (3, 0));
        for token in live {
            let row = stored(&db, token).await;
            assert!(row.retired_at.is_some());
            assert!(reused.retired.contains(&row), "{row:?}");
        }
        assert_eq!(stored(&db, &first.refresh).await, used);
        assert!(!is_stored(&db, &never_issued.access).await);
        assert!(!is_stored(&db, &never_issued.refresh).await);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_hash_no_row_holds_is_decided_on_no_rows_and_has_nowhere_to_write() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        pair_family(&db, user, Uuid::new_v4()).await;
        let never_issued = TokenSecret::generate().unwrap();
        let hash = never_issued.hash();
        let fresh = FreshPair::generate().unwrap();

        let shown = rotate_family(&db, &hash, |family| (FamilyChange::default(), family.len()));
        let nothing_written = FamilyWritten {
            retired: Vec::new(),
            inserted: Vec::new(),
            answer: 0,
        };
        assert_eq!(shown.await.unwrap(), nothing_written);
        let refused = present(&db, &never_issued, JKT, &fresh).await;
        assert_eq!(refused.answer, Err(RefreshRefusal::Unknown));

        let rows = fresh.rows(Uuid::new_v4(), JKT, unix_now()).unwrap();
        let ownerless = FamilyChange {
            retire: Vec::new(),
            insert: vec![rows.access, rows.refresh],
        };
        let err = rotate_family(&db, &hash, |_| (ownerless, ()))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::ChangeOutsideFamily), "{err}");
        assert!(!is_stored(&db, &fresh.access).await);
        assert!(!is_stored(&db, &fresh.refresh).await);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_rotation_neither_sees_nor_changes_another_users_rows_of_the_same_family_id() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        let shared = Uuid::new_v4();
        let of_a = pair_family(&db, a, shared).await;
        let of_b = pair_family(&db, b, shared).await;
        let rows_of_b = || async {
            [
                stored(&db, &of_b.access).await,
                stored(&db, &of_b.refresh).await,
            ]
        };
        let b_before = rows_of_b().await;

        let hash = of_a.refresh.hash();
        let fresh = FreshPair::generate().unwrap();
        let rotated = rotate_family(&db, &hash, |family| {
            let owners: Vec<UserId> = family.iter().map(|row| row.user_id).collect();
            let decision = refresh_change(family, &hash, JKT, unix_now(), &fresh).unwrap();
            (decision.change, (owners, decision.verdict))
        })
        .await
        .unwrap();
        assert_eq!(rotated.answer, (vec![a, a], Ok(())));
        for row in rotated.retired.iter().chain(&rotated.inserted) {
            assert_eq!((row.user_id, row.family_id), (a, shared));
        }
        assert_eq!(rows_of_b().await, b_before);

        let never_issued = FreshPair::generate().unwrap();
        let reused = present(&db, &of_a.refresh, JKT, &never_issued).await;
        assert_eq!(reused.answer, Err(RefreshRefusal::Reused));
        assert_eq!(reused.retired.len(), 3);
        assert!(reused.retired.iter().all(|row| row.user_id == a));
        assert_eq!(rows_of_b().await, b_before);

        let names_a_row_of_b = FamilyChange {
            retire: vec![b_before[1].id],
            insert: Vec::new(),
        };
        let err = rotate_family(&db, &hash, |_| (names_a_row_of_b, ()))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::ChangeOutsideFamily), "{err}");
        assert_eq!(rows_of_b().await, b_before);

        let by_b = present(&db, &of_b.refresh, JKT, &never_issued).await;
        assert_eq!(by_b.answer, Ok(()));
        assert!(by_b.inserted.iter().all(|row| row.user_id == b));
    })
    .await;
}

/// The presentation that waited for the other has to be shown the pair the
/// other inserted, which nothing it read before it waited includes. Decided
/// without them, its reuse verdict retires the old access token and leaves
/// the winner's pair alive.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn two_concurrent_presentations_of_one_refresh_token_leave_no_live_row() {
    in_fresh_schema(|db| async move {
        for round in 0..8 {
            let user = create_user(&db).await.unwrap();
            let family_id = Uuid::new_v4();
            let first = pair_family(&db, user, family_id).await;
            let one = FreshPair::generate().unwrap();
            let other = FreshPair::generate().unwrap();

            let (by_one, by_other) = tokio::join!(
                present(&db, &first.refresh, JKT, &one),
                present(&db, &first.refresh, JKT, &other),
            );

            let reused = Err(RefreshRefusal::Reused);
            let answers = [by_one.answer, by_other.answer];
            assert!(
                answers == [Ok(()), reused] || answers == [reused, Ok(())],
                "round {round}: {answers:?}"
            );
            let (rows, live): (i64, i64) = sqlx::query_as(
                "SELECT count(*), count(*) FILTER (WHERE retired_at IS NULL) \
                 FROM tokens WHERE user_id = $1 AND family_id = $2",
            )
            .bind(user)
            .bind(family_id)
            .fetch_one(&db)
            .await
            .unwrap();
            assert_eq!((rows, live), (4, 0), "round {round}");
        }
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_refresh_of_an_old_family_reads_only_its_presented_and_live_rows_and_reuse_still_revokes_it(
) {
    in_fresh_schema(|db| async move {
        const AGED: usize = 60;
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (new, first) = new_pairing(&key, LAPTOP, &[Scope::Read]);
        pair_instance(user, &db, &new).await.unwrap();
        let family = (new.family_id, key.jkt.as_str());
        let long_gone = Duration::hours(-1);
        let still_good = Duration::days(1);
        aged_rows(&db, user, family, TokenKind::Access, long_gone, false, AGED).await;
        let used_and_expired =
            aged_rows(&db, user, family, TokenKind::Refresh, long_gone, true, AGED).await;
        let used = aged_rows(
            &db,
            user,
            family,
            TokenKind::Refresh,
            still_good,
            true,
            AGED,
        )
        .await;
        let rows_past_expiry = || async {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM tokens \
                 WHERE user_id = $1 AND jkt = $2 AND expires_at <= now()",
            )
            .bind(user)
            .bind(&key.jkt)
            .fetch_one(&db)
            .await
            .unwrap()
        };
        assert_eq!(rows_past_expiry().await, 2 * AGED as i64);

        let second = FreshPair::generate().unwrap();
        let rotated = present_counting(&db, &first.refresh, &key.jkt, &second).await;
        assert_eq!(rotated.answer, (2, Ok(())));
        assert_eq!((rotated.retired.len(), rotated.inserted.len()), (1, 2));

        assert_eq!(rows_past_expiry().await, 0);
        for token in &used {
            assert!(is_stored(&db, token).await);
        }

        let never_issued = FreshPair::generate().unwrap();
        let reused = present_counting(&db, &used[0], &key.jkt, &never_issued).await;
        assert_eq!(reused.answer, (4, Err(RefreshRefusal::Reused)));
        assert_eq!((reused.retired.len(), reused.inserted.len()), (3, 0));
        for token in [&first.access, &second.access, &second.refresh] {
            assert!(!is_live(&db, token).await);
        }
        let live: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tokens \
             WHERE user_id = $1 AND family_id = $2 AND retired_at IS NULL",
        )
        .bind(user)
        .bind(new.family_id)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_eq!(live, 0);

        let rows_before = token_rows_of(&db, user).await;
        let forgotten = present_counting(&db, &used_and_expired[0], &key.jkt, &never_issued).await;
        assert_eq!(forgotten.answer, (0, Err(RefreshRefusal::Unknown)));
        assert_eq!((forgotten.retired.len(), forgotten.inserted.len()), (0, 0));
        assert_eq!(token_rows_of(&db, user).await, rows_before);
    })
    .await;
}

/// Deleting an instance takes its `device_keys` row with it and leaves its
/// tokens stored.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_token_whose_device_key_row_is_gone_is_decided_on_no_rows_and_writes_nothing() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (new, pair) = new_pairing(&key, LAPTOP, &[Scope::Read]);
        let paired = pair_instance(user, &db, &new).await.unwrap();
        sqlx::query("DELETE FROM instances WHERE user_id = $1 AND id = $2")
            .bind(user)
            .bind(paired.instance_id)
            .execute(&db)
            .await
            .unwrap();
        assert!(device_keys_of(&db, user).await.is_empty());
        let before = [
            stored(&db, &pair.access).await,
            stored(&db, &pair.refresh).await,
        ];

        let fresh = FreshPair::generate().unwrap();
        let refused = present_counting(&db, &pair.refresh, &key.jkt, &fresh).await;
        assert_eq!(refused.answer, (0, Err(RefreshRefusal::Unknown)));
        assert_eq!((refused.retired.len(), refused.inserted.len()), (0, 0));

        let rows = fresh.rows(new.family_id, &key.jkt, unix_now()).unwrap();
        let into_its_family = FamilyChange {
            retire: Vec::new(),
            insert: vec![rows.access, rows.refresh],
        };
        let err = rotate_family(&db, &pair.refresh.hash(), |_| (into_its_family, ()))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::ChangeOutsideFamily), "{err}");

        let after = [
            stored(&db, &pair.access).await,
            stored(&db, &pair.refresh).await,
        ];
        assert_eq!(after, before);
        assert_eq!(token_rows_of(&db, user).await, 2);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_first_pairing_writes_an_instance_its_device_key_and_an_access_and_a_refresh_row() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let stranger = create_user(&db).await.unwrap();
        for (owner, credential_id, status) in [
            (user, b"trusted".as_slice(), Status::Trusted),
            (user, b"pending".as_slice(), Status::Pending),
            (stranger, b"theirs".as_slice(), Status::Trusted),
        ] {
            insert_passkey(owner, &db, &new_passkey(credential_id), status)
                .await
                .unwrap();
        }
        let key = device_key(&SoftDeviceKey::new());
        let (new, pair) = new_pairing(&key, LAPTOP, &[Scope::Read, Scope::Gates]);

        let paired = pair_instance(user, &db, &new).await.unwrap();

        let [instance] = list_instances(user, &db).await.unwrap().try_into().unwrap();
        assert_eq!(instance.id, paired.instance_id);
        assert_eq!(
            [
                &instance.name,
                &instance.os,
                &instance.arch,
                &instance.version
            ],
            LAPTOP
        );
        assert_eq!(instance.granted_scopes, ["read", "gates"]);
        assert!(list_instances(stranger, &db).await.unwrap().is_empty());

        let [(of_instance, public_jwk, jkt, revoked)] =
            device_keys_of(&db, user).await.try_into().unwrap();
        assert_eq!(of_instance, paired.instance_id);
        assert_eq!(jkt, key.jkt);
        assert!(!revoked);
        let as_stored: serde_json::Value = serde_json::from_str(&public_jwk).unwrap();
        let as_handed_over: serde_json::Value = serde_json::from_str(&key.public_jwk).unwrap();
        assert_eq!(as_stored, as_handed_over);

        assert_eq!(token_rows_of(&db, user).await, 2);
        for (secret, kind, row) in [
            (&pair.access, TokenKind::Access, &new.access),
            (&pair.refresh, TokenKind::Refresh, &new.refresh),
        ] {
            let stored = stored(&db, secret).await;
            assert_eq!(stored.kind, kind);
            assert_eq!(stored.user_id, user);
            assert_eq!(stored.family_id, new.family_id);
            assert_eq!(stored.jkt, key.jkt);
            assert_eq!(stored.expires_at, row.expires_at);
            assert_eq!(stored.retired_at, None);
        }

        let statuses: Vec<Status> = paired.passkeys.iter().map(|p| p.status).collect();
        assert_eq!(statuses, [Status::Trusted, Status::Pending]);
        assert_eq!(paired.passkeys, list_passkeys(user, &db).await.unwrap());
    })
    .await;
}

/// A runner's grant stores `runner`, and pairing the same key again takes the
/// kind of the grant that paired it last, either way round.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_instance_has_the_kind_of_the_grant_that_paired_it_last() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let as_kind = |kind| {
            let (new, _) = new_pairing(&key, LAPTOP, &[Scope::Read]);
            NewPairing { kind, ..new }
        };

        let paired = pair_instance(user, &db, &as_kind(InstanceKind::Runner))
            .await
            .unwrap();
        assert_eq!(
            instance_of(&db, user, paired.instance_id).await.kind,
            "runner"
        );

        pair_instance(user, &db, &as_kind(InstanceKind::Desktop))
            .await
            .unwrap();
        assert_eq!(
            instance_of(&db, user, paired.instance_id).await.kind,
            "desktop"
        );

        pair_instance(user, &db, &as_kind(InstanceKind::Runner))
            .await
            .unwrap();
        let found = find_instance(user, &db, paired.instance_id).await.unwrap();
        assert_eq!(found.map(|row| row.kind), Some("runner".to_owned()));

        let unnamed = sqlx::query(
            "INSERT INTO instances (id, user_id, name, os, arch, version, granted_scopes) \
             VALUES ($1, $2, 'x', 'x', 'x', 'x', '{}')",
        )
        .bind(InstanceId::generate())
        .bind(user)
        .execute(&db)
        .await;
        assert!(unnamed.is_err(), "an insert that names no kind is refused");
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn pairing_a_device_key_again_keeps_its_instance_and_retires_its_earlier_tokens() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let other_key = device_key(&SoftDeviceKey::new());

        let (first, first_pair) = new_pairing(&key, LAPTOP, &[Scope::Read, Scope::Spend]);
        let paired = pair_instance(user, &db, &first).await.unwrap();
        let rotated = FreshPair::generate().unwrap();
        let rotation = present(&db, &first_pair.refresh, &key.jkt, &rotated).await;
        assert_eq!(rotation.answer, Ok(()));
        let desktop = ["desktop", "windows", "x86_64", "1.2.0"];
        let (other, other_pair) = new_pairing(&other_key, desktop, &[Scope::Read]);
        let other_paired = pair_instance(user, &db, &other).await.unwrap();
        let before = instance_of(&db, user, paired.instance_id).await;

        let renamed = ["workstation", "macos", "aarch64", "1.3.0"];
        let (again, again_pair) = new_pairing(&key, renamed, &[Scope::Gates]);
        let paired_again = pair_instance(user, &db, &again).await.unwrap();

        assert_eq!(paired_again.instance_id, paired.instance_id);
        assert_ne!(other_paired.instance_id, paired.instance_id);
        assert_eq!(list_instances(user, &db).await.unwrap().len(), 2);
        let after = instance_of(&db, user, paired.instance_id).await;
        assert_eq!(
            [&after.name, &after.os, &after.arch, &after.version],
            renamed
        );
        assert_eq!(after.granted_scopes, ["gates"]);
        assert_eq!(after.created_at, before.created_at);
        let untouched = instance_of(&db, user, other_paired.instance_id).await;
        assert_eq!(untouched.name, "desktop");
        assert_eq!(untouched.granted_scopes, ["read"]);

        let keys: Vec<(InstanceId, String)> = device_keys_of(&db, user)
            .await
            .into_iter()
            .map(|(instance_id, _, jkt, _)| (instance_id, jkt))
            .collect();
        assert_eq!(
            keys,
            [
                (paired.instance_id, key.jkt.clone()),
                (other_paired.instance_id, other_key.jkt.clone()),
            ]
        );

        let earlier = [
            &first_pair.access,
            &first_pair.refresh,
            &rotated.access,
            &rotated.refresh,
        ];
        for token in earlier {
            assert!(!is_live(&db, token).await);
        }
        let kept = [
            &again_pair.access,
            &again_pair.refresh,
            &other_pair.access,
            &other_pair.refresh,
        ];
        for token in kept {
            assert!(is_live(&db, token).await);
        }
        assert_ne!(again.family_id, first.family_id);
        assert_eq!(
            stored(&db, &again_pair.refresh).await.family_id,
            again.family_id
        );
        assert_eq!(token_rows_of(&db, user).await, 8);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn pairing_a_device_key_another_user_holds_writes_its_own_rows_and_retires_none_of_theirs() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        enrol_trusted(&db, a, b"a-passkey").await;
        enrol_trusted(&db, b, b"b-passkey").await;
        let key = device_key(&SoftDeviceKey::new());

        let (of_a, pair_of_a) = new_pairing(&key, LAPTOP, &[Scope::Read, Scope::Spend]);
        let a_paired = pair_instance(a, &db, &of_a).await.unwrap();
        let a_before = instance_of(&db, a, a_paired.instance_id).await;

        let renamed = ["workstation", "macos", "aarch64", "1.3.0"];
        let (of_b, pair_of_b) = new_pairing(&key, renamed, &[Scope::Gates]);
        let b_paired = pair_instance(b, &db, &of_b).await.unwrap();
        let (b_again, second_pair_of_b) = new_pairing(&key, renamed, &[Scope::Gates]);
        let b_paired_again = pair_instance(b, &db, &b_again).await.unwrap();

        assert_ne!(b_paired.instance_id, a_paired.instance_id);
        assert_eq!(b_paired_again.instance_id, b_paired.instance_id);
        for (user, paired, credential_id) in [
            (a, &a_paired, b"a-passkey"),
            (b, &b_paired, b"b-passkey"),
            (b, &b_paired_again, b"b-passkey"),
        ] {
            let [instance] = list_instances(user, &db).await.unwrap().try_into().unwrap();
            assert_eq!(instance.id, paired.instance_id);
            let [(of_instance, _, jkt, _)] = device_keys_of(&db, user).await.try_into().unwrap();
            assert_eq!(of_instance, paired.instance_id);
            assert_eq!(jkt, key.jkt);
            let [passkey] = paired.passkeys.as_slice() else {
                panic!("{paired:?}");
            };
            assert_eq!(passkey.user_id, user);
            assert_eq!(passkey.credential_id, credential_id);
        }

        let a_after = instance_of(&db, a, a_paired.instance_id).await;
        assert_eq!(a_after.name, a_before.name);
        assert_eq!(a_after.version, a_before.version);
        assert_eq!(a_after.granted_scopes, ["read", "spend"]);

        for token in [&pair_of_a.access, &pair_of_a.refresh] {
            let row = stored(&db, token).await;
            assert_eq!(row.user_id, a);
            assert_eq!(row.retired_at, None);
        }
        for token in [&pair_of_b.access, &pair_of_b.refresh] {
            assert!(!is_live(&db, token).await);
        }
        for token in [&second_pair_of_b.access, &second_pair_of_b.refresh] {
            let row = stored(&db, token).await;
            assert_eq!(row.user_id, b);
            assert_eq!(row.retired_at, None);
        }
        assert_eq!(token_rows_of(&db, a).await, 2);
        assert_eq!(token_rows_of(&db, b).await, 4);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_pairing_whose_parts_disagree_is_refused_and_writes_nothing() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (mut new, pair) = new_pairing(&key, LAPTOP, &[Scope::Read]);
        new.refresh.jkt = device_key(&SoftDeviceKey::new()).jkt;

        let err = pair_instance(user, &db, &new).await.unwrap_err();

        assert!(matches!(err, StoreError::PairingDisagrees), "{err}");
        assert!(list_instances(user, &db).await.unwrap().is_empty());
        assert!(device_keys_of(&db, user).await.is_empty());
        assert!(!is_stored(&db, &pair.access).await);
        assert!(!is_stored(&db, &pair.refresh).await);
    })
    .await;
}

/// `tokens.token_hash` is `UNIQUE`, so a pairing carrying rows that are
/// already stored fails on its last writes, after everything else it does.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_pairing_that_fails_on_its_token_rows_leaves_nothing_it_wrote_before_them() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let newcomer = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (first, pair) = new_pairing(&key, LAPTOP, &[Scope::Read]);
        let paired = pair_instance(user, &db, &first).await.unwrap();

        let again = NewPairing {
            instance: labels(["workstation", "macos", "aarch64", "1.3.0"]),
            scopes: vec![Scope::Gates],
            ..first.clone()
        };
        for who in [user, newcomer] {
            let err = pair_instance(who, &db, &again).await.unwrap_err();
            assert!(matches!(err, StoreError::Database(_)), "{err}");
        }

        let kept = instance_of(&db, user, paired.instance_id).await;
        assert_eq!(kept.name, "laptop");
        assert_eq!(kept.granted_scopes, ["read"]);
        assert!(is_live(&db, &pair.access).await);
        assert!(is_live(&db, &pair.refresh).await);
        assert_eq!(token_rows_of(&db, user).await, 2);
        assert!(list_instances(newcomer, &db).await.unwrap().is_empty());
        assert!(device_keys_of(&db, newcomer).await.is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_session_names_its_owner_and_no_other_user_can_end_it() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();

        let credential_id = b"a-passkey".as_slice();
        enrol_trusted(&db, a, credential_id).await;
        let new = new_session(b"a-session-secret", credential_id, Duration::hours(1));
        insert_session(a, &db, &new).await.unwrap();
        let owned_by_a = Some(Session {
            user_id: a,
            credential_id: credential_id.to_vec(),
        });
        assert_eq!(find_session(&db, &new.hash).await.unwrap(), owned_by_a);

        assert!(!delete_session(b, &db, &new.hash).await.unwrap());
        assert_eq!(
            delete_sessions_of_credential(b, &db, credential_id)
                .await
                .unwrap(),
            0
        );
        assert_eq!(find_session(&db, &new.hash).await.unwrap(), owned_by_a);

        assert!(delete_session(a, &db, &new.hash).await.unwrap());
        assert_eq!(find_session(&db, &new.hash).await.unwrap(), None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_expired_session_is_not_found_and_the_next_sign_in_collects_it() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        let count = |user: UserId| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM sessions WHERE user_id = $1")
                    .bind(user)
                    .fetch_one(&db)
                    .await
                    .unwrap()
            }
        };

        enrol_trusted(&db, a, b"a-passkey").await;
        enrol_trusted(&db, a, b"a-second-passkey").await;
        let expired_a = new_session(b"a-old-secret", b"a-passkey", Duration::hours(-1));
        let expired_b = new_session(b"b-old-secret", b"b-passkey", Duration::hours(-1));
        insert_session(a, &db, &expired_a).await.unwrap();
        insert_session(b, &db, &expired_b).await.unwrap();
        assert_eq!(find_session(&db, &expired_a.hash).await.unwrap(), None);
        assert_eq!((count(a).await, count(b).await), (1, 1));

        let live = new_session(b"a-new-secret", b"a-passkey", Duration::hours(1));
        let other = new_session(b"a-other-secret", b"a-second-passkey", Duration::hours(1));
        let mut tx = db.begin().await.unwrap();
        insert_session(a, &mut *tx, &live).await.unwrap();
        insert_session(a, &mut *tx, &other).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!((count(a).await, count(b).await), (2, 1));
        assert!(find_session(&db, &live.hash).await.unwrap().is_some());

        let mut tx = db.begin().await.unwrap();
        assert_eq!(
            delete_sessions_of_credential(a, &mut *tx, b"a-passkey")
                .await
                .unwrap(),
            1
        );
        tx.commit().await.unwrap();
        assert!(find_session(&db, &live.hash).await.unwrap().is_none());
        assert!(find_session(&db, &other.hash).await.unwrap().is_some());
    })
    .await;
}

/// The passkey is one that authenticates its own tenant, so what keeps the
/// crossed row from being found is whose passkey it is and nothing else.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_session_naming_another_users_passkey_is_not_found() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        let passkey_of_b = b"b-passkey".as_slice();
        enrol_trusted(&db, b, passkey_of_b).await;

        let crossed = new_session(b"a-crossed-secret", passkey_of_b, Duration::hours(1));
        let own = new_session(b"b-session-secret", passkey_of_b, Duration::hours(1));
        insert_session(a, &db, &crossed).await.unwrap();
        insert_session(b, &db, &own).await.unwrap();

        assert_eq!(find_session(&db, &crossed.hash).await.unwrap(), None);
        let owned_by_b = Some(Session {
            user_id: b,
            credential_id: passkey_of_b.to_vec(),
        });
        assert_eq!(find_session(&db, &own.hash).await.unwrap(), owned_by_b);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn list_passkeys_never_returns_another_users_passkey() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        let nobody = create_user(&db).await.unwrap();

        for (owner, credential_id, status) in [
            (a, b"a-first".as_slice(), Status::Trusted),
            (a, b"a-second".as_slice(), Status::Pending),
            (b, b"b-first".as_slice(), Status::Trusted),
        ] {
            insert_passkey(owner, &db, &new_passkey(credential_id), status)
                .await
                .unwrap();
        }

        let ids_seen_by = |user: UserId| {
            let db = db.clone();
            async move {
                let listed = list_passkeys(user, &db).await.unwrap();
                assert!(listed.iter().all(|p| p.user_id == user));
                listed
                    .into_iter()
                    .map(|p| p.credential_id)
                    .collect::<BTreeSet<_>>()
            }
        };
        assert_eq!(
            ids_seen_by(a).await,
            BTreeSet::from([b"a-first".to_vec(), b"a-second".to_vec()])
        );
        assert_eq!(ids_seen_by(b).await, BTreeSet::from([b"b-first".to_vec()]));
        assert!(ids_seen_by(nobody).await.is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_hello_naming_another_users_instance_writes_nothing() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();

        let as_b = hello("b-install", &[Scope::Gates]);
        let greeted = record_hello(b, &db, a_instance, &as_b, Uuid::new_v4(), now)
            .await
            .unwrap();
        assert_eq!(greeted, None);
        let untouched = instance_of(&db, a, a_instance).await;
        assert_eq!(untouched.install_id, None);
        assert_eq!(untouched.connection_id, None);
        assert_eq!(untouched.last_seen, None);
        assert_eq!(untouched.version, "1.2.0");
        assert_eq!(untouched.granted_scopes, ["projects:read"]);
        let own = instance_of(&db, b, b_instance).await;
        assert_eq!(own.install_id, None);
        assert_eq!(own.connection_id, None);

        let connection = Uuid::new_v4();
        let as_a = hello("a-install", &[Scope::Read]);
        let greeted = record_hello(a, &db, a_instance, &as_a, connection, now)
            .await
            .unwrap();
        assert!(greeted.is_some());
        let written = instance_of(&db, a, a_instance).await;
        assert_eq!(written.install_id.as_deref(), Some("a-install"));
        assert_eq!(written.connection_id, Some(connection));
        assert_eq!(instance_of(&db, b, b_instance).await.install_id, None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_departure_naming_another_users_instance_leaves_it_present() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let b = create_user(&db).await.unwrap();
        let connection = Uuid::new_v4();
        let now = unix_now();
        record_hello(
            a,
            &db,
            a_instance,
            &hello("a-install", &[]),
            connection,
            now,
        )
        .await
        .unwrap();
        let present = instance_of(&db, a, a_instance).await;

        assert!(
            !record_departure(b, &db, a_instance, connection, now + 60)
                .await
                .unwrap(),
            "the connection id of another tenant's instance is no way in"
        );
        let still = instance_of(&db, a, a_instance).await;
        assert_eq!(still.connection_id, Some(connection));
        assert_eq!(still.connected_at, present.connected_at);
        assert_eq!(still.last_seen, present.last_seen);

        assert!(record_departure(a, &db, a_instance, connection, now + 60)
            .await
            .unwrap());
        assert_eq!(instance_of(&db, a, a_instance).await.connection_id, None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn granted_scopes_are_never_read_from_another_users_instance() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        for (user, instance, scope) in
            [(a, a_instance, Scope::Spend), (b, b_instance, Scope::Gates)]
        {
            record_hello(
                user,
                &db,
                instance,
                &hello("i", &[scope]),
                Uuid::new_v4(),
                now,
            )
            .await
            .unwrap();
        }

        assert_eq!(granted_scopes(b, &db, a_instance).await.unwrap(), None);
        assert_eq!(granted_scopes(a, &db, b_instance).await.unwrap(), None);
        assert_eq!(
            granted_scopes(a, &db, a_instance).await.unwrap(),
            Some(vec![Scope::Spend])
        );
        assert_eq!(
            granted_scopes(b, &db, b_instance).await.unwrap(),
            Some(vec![Scope::Gates])
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_device_key_names_the_instance_of_the_user_asking_and_no_other() {
    in_fresh_schema(|db| async move {
        let a = create_user(&db).await.unwrap();
        let b = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (of_a, _) = new_pairing(&key, LAPTOP, &[Scope::Read]);
        let a_instance = pair_instance(a, &db, &of_a).await.unwrap().instance_id;

        assert_eq!(
            instance_of_key(a, &db, &key.jkt).await.unwrap(),
            Some(a_instance)
        );
        assert_eq!(instance_of_key(b, &db, &key.jkt).await.unwrap(), None);

        let (of_b, _) = new_pairing(&key, LAPTOP, &[Scope::Read]);
        let b_instance = pair_instance(b, &db, &of_b).await.unwrap().instance_id;
        assert_ne!(b_instance, a_instance);
        assert_eq!(
            instance_of_key(a, &db, &key.jkt).await.unwrap(),
            Some(a_instance)
        );
        assert_eq!(
            instance_of_key(b, &db, &key.jkt).await.unwrap(),
            Some(b_instance)
        );
    })
    .await;
}

/// Every tenant's mirror rows as `(user_id, instance_id, feature_id,
/// event_offset, event)`: read without a tenant so that a row written under
/// the wrong one is seen too.
async fn every_mirror_row(db: &Db) -> Vec<(UserId, InstanceId, String, i64, String)> {
    sqlx::query_as(
        "SELECT user_id, instance_id, feature_id, event_offset, event::text FROM run_mirror \
         ORDER BY instance_id, feature_id, event_offset",
    )
    .fetch_all(db)
    .await
    .unwrap()
}

/// Every tenant's snapshot rows as `(user_id, instance_id, kind, body,
/// updated_at)`, read the way [`every_mirror_row`] reads.
async fn every_snapshot_row(db: &Db) -> Vec<(UserId, InstanceId, String, String, OffsetDateTime)> {
    sqlx::query_as(
        "SELECT user_id, instance_id, kind, body::text, updated_at FROM snapshots \
         ORDER BY instance_id, kind",
    )
    .fetch_all(db)
    .await
    .unwrap()
}

fn new_event(feature_id: &str, event_offset: u64, event: &str) -> NewEvent {
    NewEvent {
        feature_id: feature_id.to_owned(),
        event_offset,
        event: event.to_owned(),
    }
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_event_naming_another_users_instance_is_not_stored_and_moves_no_cursor() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let never_asked = |_: u64, _: u64| -> u64 { panic!("asked about another user's cursor") };

        let from_b = new_event("f-1", 5, r#"{"by": "b"}"#);
        let stored = mirror_event(b, &db, a_instance, &from_b, never_asked)
            .await
            .unwrap();
        assert_eq!(stored, None);
        assert!(every_mirror_row(&db).await.is_empty());
        assert_eq!(instance_of(&db, a, a_instance).await.event_cursor, 0);

        let from_a = new_event("f-1", 5, r#"{"by": "a"}"#);
        let stored = mirror_event(a, &db, a_instance, &from_a, |_, offset| offset)
            .await
            .unwrap();
        assert_eq!(stored, Some(5));
        let of_a = (a, a_instance, "f-1".to_owned(), 5, from_a.event.clone());
        assert_eq!(every_mirror_row(&db).await, std::slice::from_ref(&of_a));

        let later_from_b = new_event("f-1", 9, &from_b.event);
        for over_a in [&from_b, &later_from_b] {
            let stored = mirror_event(b, &db, a_instance, over_a, never_asked)
                .await
                .unwrap();
            assert_eq!(stored, None);
        }
        assert_eq!(every_mirror_row(&db).await, std::slice::from_ref(&of_a));
        assert_eq!(instance_of(&db, a, a_instance).await.event_cursor, 5);
        assert_eq!(instance_of(&db, b, b_instance).await.event_cursor, 0);

        let stored = mirror_event(b, &db, b_instance, &later_from_b, |_, offset| offset)
            .await
            .unwrap();
        assert_eq!(stored, Some(9));
        let of_b = (b, b_instance, "f-1".to_owned(), 9, from_b.event);
        let mut both = every_mirror_row(&db).await;
        both.sort_by_key(|row| row.3);
        assert_eq!(both, [of_a, of_b]);
        assert_eq!(instance_of(&db, a, a_instance).await.event_cursor, 5);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_snapshot_naming_another_users_instance_neither_fills_nor_replaces_its_slot() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let (by_a, by_b) = (r#"{"by": "a"}"#, r#"{"by": "b"}"#);

        let err = replace_snapshot(b, &db, a_instance, "full", by_b, now)
            .await
            .expect_err("an empty slot of another tenant's instance must not be filled");
        let StoreError::Database(err) = err else {
            panic!("{err}");
        };
        assert_eq!(
            violation_code(&err).as_deref(),
            Some(FOREIGN_KEY_VIOLATION),
            "{err}"
        );
        assert!(every_snapshot_row(&db).await.is_empty());

        assert!(replace_snapshot(a, &db, a_instance, "full", by_a, now)
            .await
            .unwrap());
        let of_a = every_snapshot_row(&db).await;
        let [(owner, instance, kind, body, _)] = of_a.as_slice() else {
            panic!("{of_a:?}");
        };
        assert_eq!((*owner, *instance, kind.as_str()), (a, a_instance, "full"));
        assert_eq!(body, by_a);

        assert!(
            !replace_snapshot(b, &db, a_instance, "full", by_b, now + 60)
                .await
                .unwrap(),
            "a slot found by its instance id alone is no way in"
        );
        assert_eq!(every_snapshot_row(&db).await, of_a);

        assert!(replace_snapshot(b, &db, b_instance, "full", by_b, now + 60)
            .await
            .unwrap());
        let both = every_snapshot_row(&db).await;
        assert_eq!(both.len(), 2);
        assert!(both.contains(&of_a[0]));
        assert!(both.iter().any(|(owner, instance, _, body, _)| (
            *owner,
            *instance,
            body.as_str()
        ) == (b, b_instance, by_b)));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn find_instance_never_returns_another_users_instance() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;

        assert!(find_instance(b, &db, a_instance).await.unwrap().is_none());
        assert!(find_instance(a, &db, b_instance).await.unwrap().is_none());
        assert!(find_instance(a, &db, InstanceId::generate())
            .await
            .unwrap()
            .is_none());

        let of_a = find_instance(a, &db, a_instance).await.unwrap().unwrap();
        assert_eq!((of_a.id, of_a.user_id), (a_instance, a));
        assert_eq!(of_a.name, "a-laptop");
        let of_b = find_instance(b, &db, b_instance).await.unwrap().unwrap();
        assert_eq!((of_b.id, of_b.user_id), (b_instance, b));
    })
    .await;
}

fn mirror_record(feature_id: &str, offset: u64, event: &str) -> MirrorRecord {
    MirrorRecord {
        feature_id: feature_id.to_owned(),
        offset,
        event: event.to_owned(),
    }
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn mirror_of_never_returns_another_users_events() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let by_a = r#"{"by": "a"}"#;
        for (feature_id, offset) in [("f-2", 3), ("f-1", 2), ("f-1", 1)] {
            let new = new_event(feature_id, offset, by_a);
            mirror_event(a, &db, a_instance, &new, u64::max)
                .await
                .unwrap()
                .unwrap();
        }
        let by_b = r#"{"by": "b"}"#;
        mirror_event(b, &db, b_instance, &new_event("f-1", 9, by_b), u64::max)
            .await
            .unwrap()
            .unwrap();

        for feature in [None, Some("f-1"), Some("f-2")] {
            assert_eq!(
                mirror_of(b, &db, a_instance, feature).await.unwrap(),
                [],
                "{feature:?}"
            );
        }

        assert_eq!(
            mirror_of(a, &db, a_instance, None).await.unwrap(),
            [
                mirror_record("f-1", 1, by_a),
                mirror_record("f-1", 2, by_a),
                mirror_record("f-2", 3, by_a),
            ]
        );
        assert_eq!(
            mirror_of(a, &db, a_instance, Some("f-2")).await.unwrap(),
            [mirror_record("f-2", 3, by_a)]
        );
        assert_eq!(
            mirror_of(a, &db, a_instance, Some("f-3")).await.unwrap(),
            []
        );
        assert_eq!(
            mirror_of(b, &db, b_instance, None).await.unwrap(),
            [mirror_record("f-1", 9, by_b)]
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn read_snapshot_never_returns_another_users_snapshot() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let by_a = r#"{"by": "a"}"#;
        assert!(
            replace_snapshot(a, &db, a_instance, SNAPSHOT_KIND, by_a, now)
                .await
                .unwrap()
        );
        assert!(
            replace_snapshot(b, &db, b_instance, "another-kind", r#"{"by": "b"}"#, now)
                .await
                .unwrap()
        );

        assert_eq!(read_snapshot(b, &db, a_instance).await.unwrap(), None);
        assert_eq!(
            read_snapshot(a, &db, a_instance).await.unwrap().as_deref(),
            Some(by_a)
        );
        assert_eq!(
            read_snapshot(b, &db, b_instance).await.unwrap(),
            None,
            "a slot of another kind is not the snapshot"
        );
    })
    .await;
}

type RequestRow = (UserId, InstanceId, Vec<u8>, String, i32, Option<String>);

/// Every tenant's request rows as `(user_id, instance_id, id, status,
/// deliveries, outcome)` in queue order, read the way [`every_mirror_row`]
/// reads.
async fn every_request_row(db: &Db) -> Vec<RequestRow> {
    sqlx::query_as(
        "SELECT user_id, instance_id, id, status, deliveries, outcome FROM instance_requests \
         ORDER BY queue_seq",
    )
    .fetch_all(db)
    .await
    .unwrap()
}

fn new_request(id: RequestId, instance: InstanceId, expires_at: u64) -> NewRequest {
    NewRequest {
        id,
        kind: "cancel_feature".to_owned(),
        envelope: SignedRequest {
            v: PROTOCOL_VERSION,
            request_id: Uuid::from_bytes(id).simple().to_string(),
            instance_id: instance.0.to_string(),
            issued_at: expires_at - 300,
            expires_at,
            payload: RequestPayload::CancelFeature(CancelFeature {
                feature_id: "f-1".to_owned(),
            }),
            signature: String::new(),
        },
        expires_at,
    }
}

fn queued_row(owner: UserId, instance: InstanceId, id: RequestId) -> RequestRow {
    (owner, instance, id.to_vec(), "queued".to_owned(), 0, None)
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_request_naming_another_users_instance_is_not_queued_and_no_queue_is_read_across_users() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let id = [7; 16];
        let for_a = new_request(id, a_instance, unix_now() + 300);

        let err = insert_request(b, &db, a_instance, &for_a)
            .await
            .expect_err("another tenant's instance must not be sent a request");
        let StoreError::Database(err) = err else {
            panic!("{err}");
        };
        assert_eq!(
            violation_code(&err).as_deref(),
            Some(FOREIGN_KEY_VIOLATION),
            "{err}"
        );
        assert!(every_request_row(&db).await.is_empty());

        insert_request(a, &db, a_instance, &for_a).await.unwrap();
        assert_eq!(
            every_request_row(&db).await,
            [queued_row(a, a_instance, id)]
        );
        assert!(list_queued_requests(b, &db, a_instance)
            .await
            .unwrap()
            .is_empty());
        assert!(list_queued_requests(b, &db, b_instance)
            .await
            .unwrap()
            .is_empty());

        let for_b = new_request(id, b_instance, unix_now() + 600);
        insert_request(b, &db, b_instance, &for_b).await.unwrap();
        for (owner, instance, own) in [(a, a_instance, &for_a), (b, b_instance, &for_b)] {
            let queue = list_queued_requests(owner, &db, instance).await.unwrap();
            let [only] = queue.as_slice() else {
                panic!("{queue:?}");
            };
            assert_eq!(
                only.envelope,
                QueuedEnvelope::Readable(Box::new(own.envelope.clone()))
            );
        }
        assert!(list_queued_requests(a, &db, b_instance)
            .await
            .unwrap()
            .is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_delivery_or_an_expiry_naming_another_users_request_changes_no_row() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let id = [7; 16];
        insert_request(a, &db, a_instance, &new_request(id, a_instance, now + 300))
            .await
            .unwrap();
        let as_queued = [queued_row(a, a_instance, id)];

        for named in [a_instance, b_instance] {
            assert!(!mark_request_sent(b, &db, named, &id, now).await.unwrap());
            assert!(!mark_request_expired(b, &db, named, &id).await.unwrap());
        }
        assert!(!mark_request_sent(a, &db, b_instance, &id, now)
            .await
            .unwrap());
        assert!(!mark_request_expired(a, &db, b_instance, &id).await.unwrap());
        assert_eq!(every_request_row(&db).await, as_queued);

        assert!(mark_request_sent(a, &db, a_instance, &id, now)
            .await
            .unwrap());
        assert!(mark_request_expired(a, &db, a_instance, &id).await.unwrap());
        assert_eq!(
            every_request_row(&db).await,
            [(a, a_instance, id.to_vec(), "expired".to_owned(), 1, None)]
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_outcome_naming_another_users_request_is_not_recorded_and_decide_is_not_asked() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let id = [7; 16];
        insert_request(a, &db, a_instance, &new_request(id, a_instance, now + 300))
            .await
            .unwrap();
        let never_asked = |state: RequestState| -> Option<RequestOutcome> {
            panic!("asked about another user's request: {state:?}")
        };

        for (asking, named) in [(b, a_instance), (b, b_instance), (a, b_instance)] {
            let found = record_outcome(asking, &db, named, &id, now, never_asked)
                .await
                .unwrap();
            assert!(!found);
        }
        assert_eq!(
            every_request_row(&db).await,
            [queued_row(a, a_instance, id)]
        );

        let refused = |_: RequestState| {
            Some(RequestOutcome::Refused {
                reason: RefusalReason::ScopeOff,
            })
        };
        assert!(record_outcome(a, &db, a_instance, &id, now, refused)
            .await
            .unwrap());
        let answered = (
            a,
            a_instance,
            id.to_vec(),
            "answered".to_owned(),
            0,
            Some("refused".to_owned()),
        );
        assert_eq!(every_request_row(&db).await, [answered]);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn find_request_never_returns_another_users_request() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let id = [7; 16];
        insert_request(
            a,
            &db,
            a_instance,
            &new_request(id, a_instance, unix_now() + 300),
        )
        .await
        .unwrap();

        for (asking, named, id) in [
            (b, a_instance, id),
            (b, b_instance, id),
            (a, b_instance, id),
            (a, a_instance, [8; 16]),
        ] {
            assert_eq!(
                find_request(asking, &db, named, &id).await.unwrap(),
                None,
                "{named:?} {id:?}"
            );
        }
        let found = find_request(a, &db, a_instance, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, id);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_request_reads_back_queued_unsent_and_unanswered_until_it_is_sent_and_answered() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let before = unix_now();
        let id = [7; 16];
        let expires_at = before + 300;
        insert_request(a, &db, a_instance, &new_request(id, a_instance, expires_at))
            .await
            .unwrap();

        let fresh = find_request(a, &db, a_instance, &id)
            .await
            .unwrap()
            .unwrap();
        // `created_at` is the column's `DEFAULT now()`, Postgres's clock and
        // not this process's, so it is held to a window and not a value.
        assert!(
            fresh.created_at + 60 >= before && fresh.created_at <= unix_now() + 60,
            "{fresh:?}"
        );
        assert_eq!(
            fresh,
            RequestRecord {
                id,
                kind: "cancel_feature".to_owned(),
                status: RequestStatus::Queued,
                deliveries: 0,
                outcome: None,
                expires_at,
                created_at: fresh.created_at,
                answered_at: None,
            }
        );

        let answered_at = before + 7;
        assert!(mark_request_sent(a, &db, a_instance, &id, before)
            .await
            .unwrap());
        let refused = RequestOutcome::Refused {
            reason: RefusalReason::ScopeOff,
        };
        assert!(
            record_outcome(a, &db, a_instance, &id, answered_at, |_| Some(refused))
                .await
                .unwrap()
        );
        assert_eq!(
            find_request(a, &db, a_instance, &id)
                .await
                .unwrap()
                .unwrap(),
            RequestRecord {
                status: RequestStatus::Answered,
                deliveries: 1,
                outcome: Some(refused),
                answered_at: Some(answered_at),
                ..fresh
            }
        );
    })
    .await;
}

/// Every row of the four tables a socket writes, as `(table, the row as
/// Postgres spells it in JSON)`: read without a tenant, as
/// [`every_mirror_row`] reads, and whole, so that a write to any column of
/// any of them shows.
async fn every_socket_row(db: &Db) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    for table in ["instances", "run_mirror", "snapshots", "instance_requests"] {
        let select = format!("SELECT to_jsonb(t)::text FROM {table} AS t ORDER BY 1");
        let of_table: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(select))
            .fetch_all(db)
            .await
            .unwrap();
        rows.extend(of_table.into_iter().map(|row| (table, row)));
    }
    rows
}

fn never_moved(before: u64, offset: u64) -> u64 {
    panic!("asked about another user's cursor: {before}, {offset}")
}

/// The session's own store, bound as no admission binds it: the tenant of
/// one user and the instance of another. Every method is called against an
/// instance that has a row in each table for it to find.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_socket_store_of_one_user_bound_to_another_users_instance_writes_nothing() {
    in_fresh_schema(|db| async move {
        let (a, _) = user_with_instance(&db, "a-laptop").await;
        let (b, b_instance) = user_with_instance(&db, "b-laptop").await;
        let now = unix_now();
        let connections = Connections::default();
        let seated = |instance| connections.register(instance).unwrap().connection;
        let event = |offset| RunEvent {
            v: PROTOCOL_VERSION,
            feature_id: "f-1".to_owned(),
            offset,
            kind: RunEventKind::StepStarted,
            occurred_at: now,
            payload: RunEventPayload::default(),
        };
        let snapshot = Snapshot {
            v: PROTOCOL_VERSION,
            projects: Vec::new(),
            workflows: Vec::new(),
            run_shape: Vec::new(),
            features: Vec::new(),
            harnesses: None,
        };
        let id = [7; 16];

        let own = PostgresStore::new(db.clone(), b, b_instance);
        let b_connection = seated(b_instance);
        let said = hello("install-b", &[Scope::Spend]);
        assert_eq!(own.greeted(&said, b_connection, now).await.unwrap(), 0);
        assert_eq!(
            own.ingested(&event(5), |_, offset| offset).await.unwrap(),
            5
        );
        own.snapshot_replaced(&snapshot, now).await.unwrap();
        own.request_inserted(new_request(id, b_instance, now + 300))
            .await
            .unwrap();
        let before = every_socket_row(&db).await;
        let tables: Vec<&str> = before.iter().map(|(table, _)| *table).collect();
        assert_eq!(
            tables,
            [
                "instances",
                "instances",
                "run_mirror",
                "snapshots",
                "instance_requests"
            ]
        );

        let crossed = PostgresStore::new(db.clone(), a, b_instance);
        let no_instance = |err: StoreError| matches!(err, StoreError::NoSuchInstance);
        let said = hello("install-a", &[Scope::Gates]);
        let err = crossed
            .greeted(&said, seated(InstanceId::generate()), now + 60)
            .await
            .expect_err("a hello must not be written onto another tenant's instance");
        assert!(no_instance(err));
        let err = crossed
            .ingested(&event(9), never_moved)
            .await
            .expect_err("an event must not be stored for another tenant's instance");
        assert!(no_instance(err));
        let err = crossed
            .snapshot_replaced(&snapshot, now + 60)
            .await
            .expect_err("another tenant's snapshot must not be replaced");
        assert!(no_instance(err));
        let err = crossed
            .granted_scopes()
            .await
            .expect_err("another tenant's scopes must not be read");
        assert!(no_instance(err));

        let err = crossed
            .request_inserted(new_request([9; 16], b_instance, now + 300))
            .await
            .expect_err("another tenant's instance must not be sent a request");
        let StoreError::Database(err) = err else {
            panic!("{err}");
        };
        assert_eq!(
            violation_code(&err).as_deref(),
            Some(FOREIGN_KEY_VIOLATION),
            "{err}"
        );
        assert!(crossed.queued_requests().await.unwrap().is_empty());
        let err = crossed
            .request_sent(id, now + 60)
            .await
            .expect_err("a delivery must not be counted on another tenant's request");
        assert!(matches!(err, StoreError::NoSuchRequest), "{err}");
        crossed.request_expired(id).await.unwrap();
        let never_asked = |state: RequestState| -> OutcomeChange {
            panic!("asked about another user's request: {state:?}")
        };
        assert!(!crossed
            .outcome_recorded(id, &never_asked, now + 60)
            .await
            .unwrap());
        crossed.departed(b_connection, now + 60).await.unwrap();

        assert_eq!(every_socket_row(&db).await, before);
        assert_eq!(own.queued_requests().await.unwrap().len(), 1);
        assert_eq!(
            instance_of(&db, b, b_instance).await.connection_id,
            Some(b_connection.as_uuid())
        );
    })
    .await;
}
