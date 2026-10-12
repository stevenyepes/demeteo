//! Tenant isolation against a real Postgres. Needs the `postgres` service from
//! `deploy/hub/compose.yaml` as `deploy/hub/README.md` describes, and
//! `DEMETEO_HUB_TEST_DATABASE_URL` pointing at it.

mod support;

use demeteo_hub::auth::policy::Status;
use demeteo_hub::store::attachments::{
    get_attachment, insert_attachment, NewAttachment, WrappedKey,
};
use demeteo_hub::store::instances::{insert_instance, list_instances, NewInstance};
use demeteo_hub::store::passkeys::{insert_passkey, list_passkeys, NewPasskey};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::sessions::{
    delete_session, delete_sessions_of_credential, find_session, insert_session, NewSession,
    Session, SessionHash,
};
use demeteo_hub::store::tokens::{find_token, insert_token, retire_token, NewToken, TokenHash};
use demeteo_hub::store::{Db, InstanceId, UserId};
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
async fn tokens_and_attachments_are_invisible_to_other_users() {
    in_fresh_schema(|db| async move {
        let (a, a_instance) = user_with_instance(&db, "a-laptop").await;
        let (b, _) = user_with_instance(&db, "b-laptop").await;

        let hash = TokenHash::digest(b"a-secret-token");
        let new_token = NewToken {
            hash: hash.clone(),
            family_id: Uuid::new_v4(),
            jkt: "thumbprint".to_owned(),
            expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
        };
        insert_token(a, &db, &new_token).await.unwrap();
        assert!(find_token(a, &db, &hash).await.unwrap().is_some());
        assert!(find_token(b, &db, &hash).await.unwrap().is_none());
        assert!(!retire_token(b, &db, &hash).await.unwrap());
        assert!(retire_token(a, &db, &hash).await.unwrap());

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
