//! Tenant isolation against a real Postgres. Needs the `postgres` service from
//! `deploy/hub/compose.yaml` as `deploy/hub/README.md` describes, and
//! `DEMETEO_HUB_TEST_DATABASE_URL` pointing at it.
//!
//! Each test migrates a schema of its own and drops it at the end, so tests
//! run in parallel and leave the database as they found it.

use demeteo_hub::store::attachments::{
    get_attachment, insert_attachment, NewAttachment, WrappedKey,
};
use demeteo_hub::store::instances::{insert_instance, list_instances, NewInstance};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::tokens::{find_token, insert_token, retire_token, NewToken, TokenHash};
use demeteo_hub::store::{self, Db, InstanceId, UserId};
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::collections::BTreeSet;
use std::future::Future;
use std::str::FromStr;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const URL_ENV: &str = "DEMETEO_HUB_TEST_DATABASE_URL";
const FOREIGN_KEY_VIOLATION: &str = "23503";
const EXPECTED_TABLES: [&str; 9] = [
    "attachments",
    "device_keys",
    "instance_requests",
    "instances",
    "passkeys",
    "run_mirror",
    "snapshots",
    "tokens",
    "users",
];

async fn in_fresh_schema<F, Fut>(body: F)
where
    F: FnOnce(Db) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let url = std::env::var(URL_ENV)
        .unwrap_or_else(|_| panic!("{URL_ENV} must be set (see deploy/hub/README.md)"));
    let schema = format!("hub_tenancy_{}", Uuid::new_v4().simple());

    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();

    // Everything after CREATE SCHEMA runs in the task, so a panic in connect
    // or migrate is caught here too and the DROP below still runs.
    let task_schema = schema.clone();
    let outcome = tokio::spawn(async move {
        let options = PgConnectOptions::from_str(&url)
            .unwrap()
            .options([("search_path", task_schema.as_str())]);
        let db = PgPoolOptions::new().connect_with(options).await.unwrap();
        store::migrate(&db).await.unwrap();
        body(db.clone()).await;
        db.close().await;
    })
    .await;

    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    if let Err(failure) = outcome {
        std::panic::resume_unwind(failure.into_panic());
    }
}

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
