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
use demeteo_hub::store::attachments::{
    get_attachment, insert_attachment, NewAttachment, WrappedKey,
};
use demeteo_hub::store::device_keys::NewDeviceKey;
use demeteo_hub::store::instances::{
    insert_instance, list_instances, pair_instance, Instance, NewInstance, NewInstanceLabels,
    NewPairing,
};
use demeteo_hub::store::passkeys::{insert_passkey, list_passkeys, NewPasskey};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::sessions::{
    delete_session, delete_sessions_of_credential, find_session, insert_session, NewSession,
    Session, SessionHash,
};
use demeteo_hub::store::tokens::{
    find_token_by_hash, insert_token, rotate_family, FamilyWritten, NewToken, TokenHash, TokenKind,
    TokenRecord,
};
use demeteo_hub::store::{Db, InstanceId, StoreError, UserId};
use demeteo_hub_protocol::Scope;
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

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn attachments_are_invisible_to_other_users() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, _) = user_with_instance(&db, "b-laptop").await;

        let request_id = vec![7u8; 16];
        sqlx::query(
            "INSERT INTO instance_requests (id, user_id, instance_id, kind, envelope, expires_at) \
             VALUES ($1, $2, $3, 'probe', '{}'::jsonb, now() + interval '1 hour')",
        )
        .bind(&request_id)
        .bind(a)
        .bind(a_instance)
        .execute(&db)
        .await
        .unwrap();

        let attachment = NewAttachment {
            instance_id: a_instance,
            request_id,
            ciphertext: vec![1, 2, 3],
            wrapped_key: WrappedKey::parse("vault:v1:wrapped").unwrap(),
        };
        assert!(insert_attachment(b, &db, &attachment).await.is_err());
        let stored = insert_attachment(a, &db, &attachment).await.unwrap();
        assert_eq!(stored.wrapped_key, "vault:v1:wrapped");
        assert_eq!(
            stored.sha256,
            Sha256::digest(&attachment.ciphertext).as_slice()
        );
        assert_eq!(stored.size_bytes, 3);
        assert!(get_attachment(a, &db, stored.id).await.unwrap().is_some());
        assert!(get_attachment(b, &db, stored.id).await.unwrap().is_none());
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
