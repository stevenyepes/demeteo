//! The attachment lifecycle over HTTP against a real Postgres: what
//! `src-tauri/tests/hub_attachments.rs` holds to a store double, which applies
//! what it is handed and so cannot show that a statement does. Left for this
//! file is what only Postgres and the real router can show: that the bind is
//! atomic under two dispatches at once, that of two fetches at once only one
//! deletes the row, that the sweep's per-tenant `DELETE` reaches every tenant,
//! that the upload route's `DefaultBodyLimit` is the configured one, and that
//! no stored cell holds the content.
//!
//! The tests are ignored for the reason `crates/demeteo-hub/tests/pairing.rs`
//! gives in its header, and an ignored run shows nothing of them.

mod support;

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::attachments::postgres::PostgresAttachments;
use demeteo_hub::attachments::sweep::sweep_once;
use demeteo_hub::attachments::AttachmentSettings;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::session::SessionSecret;
use demeteo_hub::auth::testing::SoftAuthenticator;
use demeteo_hub::pairing::grant::FreshPair;
use demeteo_hub::pairing::jwk::DeviceJwk;
use demeteo_hub::pairing::testing::SoftDeviceKey;
use demeteo_hub::routes::AppState;
use demeteo_hub::store::attachments::{insert_attachment, NewAttachment, WrappedKey};
use demeteo_hub::store::device_keys::NewDeviceKey;
use demeteo_hub::store::instances::{pair_instance, InstanceKind, NewInstanceLabels, NewPairing};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::{Db, InstanceId, UserId};
use demeteo_hub_protocol::Scope;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use support::http::{
    enrol, get_with_dpop, open_session, post_as, post_bytes_as, rp, serve_state, unix_now,
};
use support::in_fresh_schema;
use uuid::Uuid;

const SIGNING_KEY: &str = "demeteo-hub-signing";
const TRANSIT_KEY: &str = "demeteo-hub";

fn state_over(db: Db, settings: AttachmentSettings) -> AppState {
    let keys = MemoryKeys::with_keys([SIGNING_KEY, TRANSIT_KEY]);
    AppState::new(
        db,
        rp(),
        Arc::new(keys),
        SIGNING_KEY.to_owned(),
        TRANSIT_KEY.to_owned(),
        settings,
    )
    .unwrap()
}

/// A tenant signed in with its one trusted passkey.
struct Tenant {
    user: UserId,
    session: SessionSecret,
}

impl Tenant {
    async fn new(db: &Db) -> Self {
        let user = create_user(db).await.unwrap();
        let key = SoftAuthenticator::new();
        enrol(db, user, &key, Status::Trusted).await;
        let session = open_session(db, user, &key).await;
        Self { user, session }
    }

    /// An instance of this tenant's, paired now with `read` and `spend`.
    async fn paired(&self, db: &Db) -> Desktop {
        let key = SoftDeviceKey::new();
        let jwk = DeviceJwk::parse(&key.jwk()).unwrap();
        let device_key = NewDeviceKey {
            public_jwk: jwk.canonical_json(),
            jkt: jwk.thumbprint(),
        };
        let family_id = Uuid::new_v4();
        let pair = FreshPair::generate().unwrap();
        let rows = pair.rows(family_id, &device_key.jkt, unix_now()).unwrap();
        let new = NewPairing {
            kind: InstanceKind::Desktop,
            instance: NewInstanceLabels {
                name: "laptop".to_owned(),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                version: "1.2.0".to_owned(),
            },
            device_key,
            scopes: vec![Scope::Read, Scope::Spend],
            family_id,
            access: rows.access,
            refresh: rows.refresh,
        };
        let instance = pair_instance(self.user, db, &new)
            .await
            .unwrap()
            .instance_id;
        Desktop {
            instance,
            key,
            access_token: pair.access.text(),
        }
    }

    async fn upload(&self, base: &str, desktop: &Desktop, content: &[u8]) -> String {
        let path = desktop.at("/attachments");
        let (status, staged) = post_bytes_as(base, &path, &self.session, content.to_vec()).await;
        assert_eq!(status, 201, "{staged}");
        staged["attachment_id"].as_str().unwrap().to_owned()
    }

    async fn dispatch(&self, base: &str, desktop: &Desktop, attachment_id: &str) -> (u16, Value) {
        let body = json!({
            "project_id": "p",
            "workflow_id": "w",
            "title": "t",
            "description": "",
            "attachment_ids": [attachment_id],
        });
        post_as(base, &desktop.at("/runs"), &self.session, &body).await
    }

    /// `content` uploaded for `desktop` and named by a dispatch to it, so the
    /// desktop may fetch it.
    async fn sent(&self, base: &str, desktop: &Desktop, content: &[u8]) -> String {
        let id = self.upload(base, desktop, content).await;
        let (status, queued) = self.dispatch(base, desktop, &id).await;
        assert_eq!(status, 202, "{queued}");
        id
    }
}

/// A paired instance and what it presents to the Hub.
struct Desktop {
    instance: InstanceId,
    key: SoftDeviceKey,
    access_token: String,
}

impl Desktop {
    fn at(&self, rest: &str) -> String {
        format!("/v1/instances/{}{rest}", self.instance.0)
    }

    async fn fetch(&self, base: &str, attachment_id: &str) -> (u16, Vec<u8>) {
        let path = format!("/attachments/{attachment_id}");
        get_with_dpop(base, &path, &self.key, &self.access_token).await
    }
}

fn not_found() -> (u16, Vec<u8>) {
    (404, br#"{"error":"not_found"}"#.to_vec())
}

async fn count(db: &Db, select: &'static str) -> i64 {
    sqlx::query_scalar(select).fetch_one(db).await.unwrap()
}

async fn attachment_rows(db: &Db) -> i64 {
    count(db, "SELECT count(*) FROM attachments").await
}

async fn request_rows(db: &Db) -> i64 {
    count(db, "SELECT count(*) FROM instance_requests").await
}

/// Distinctive, and longer than any id or key the rows hold, so a match is
/// the content and not a coincidence.
fn content() -> Vec<u8> {
    b"the content of an attachment, which no stored cell may hold. ".repeat(3)
}

const LIFECYCLE_TABLES: [&str; 2] = ["attachments", "instance_requests"];

/// One stored value: a `BYTEA` as its bytes and anything else as the text
/// Postgres writes it as.
struct Cell {
    table: &'static str,
    column: String,
    bytes: Vec<u8>,
}

/// Every non-null value of every row of `table`: the scan of `cells_of` in
/// `crates/demeteo-hub/tests/socket.rs`, which `tests/pairing.rs` explains.
async fn cells_of(db: &Db, table: &'static str) -> Vec<Cell> {
    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(db)
    .await
    .unwrap();
    assert!(!columns.is_empty(), "no table `{table}` in this schema");

    let mut cells = Vec::new();
    for (column, data_type) in columns {
        let as_bytes = match data_type.as_str() {
            "bytea" => format!("\"{column}\""),
            _ => format!("convert_to(\"{column}\"::text, 'UTF8')"),
        };
        let select = format!("SELECT {as_bytes} FROM {table}");
        let stored: Vec<Option<Vec<u8>>> = sqlx::query_scalar(sqlx::AssertSqlSafe(select))
            .fetch_all(db)
            .await
            .unwrap();
        cells.extend(stored.into_iter().flatten().map(|bytes| Cell {
            table,
            column: column.clone(),
            bytes,
        }));
    }
    cells
}

fn columns_holding<'a>(cells: &'a [Cell], needle: &[u8]) -> Vec<(&'a str, &'a str)> {
    let mut columns: Vec<(&str, &str)> = cells
        .iter()
        .filter(|cell| cell.bytes.windows(needle.len()).any(|run| run == needle))
        .map(|cell| (cell.table, cell.column.as_str()))
        .collect();
    columns.dedup();
    columns
}

/// The search is first watched find a value of each table, the ciphertext
/// in its `BYTEA` and the id in both, and only then asked for the content.
/// The hex digest is in the signed envelope and nowhere else: the protocol
/// signs it, and `docs/HUB.md` §9 records that the request row keeps it.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn no_lifecycle_cell_holds_the_content_or_its_raw_digest() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db).await;
        let desktop = a.paired(&db).await;
        let base = serve_state(state_over(db.clone(), AttachmentSettings::default())).await;
        let content = content();
        let id = a.sent(&base, &desktop, &content).await;

        let mut cells = Vec::new();
        for table in LIFECYCLE_TABLES {
            cells.extend(cells_of(&db, table).await);
        }
        let rows_of = |table: &str| {
            let owners = cells
                .iter()
                .filter(|cell| cell.table == table && cell.column == "user_id");
            owners.count()
        };
        assert_eq!(LIFECYCLE_TABLES.map(rows_of), [1, 1]);
        let ciphertext: Vec<u8> = sqlx::query_scalar("SELECT ciphertext FROM attachments")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(
            columns_holding(&cells, &ciphertext),
            [("attachments", "ciphertext")]
        );
        assert_eq!(
            columns_holding(&cells, id.as_bytes()),
            [("attachments", "id"), ("instance_requests", "envelope")]
        );
        assert_eq!(
            columns_holding(&cells, b"vault:"),
            [("attachments", "wrapped_key")]
        );
        let digest = Sha256::digest(&content);
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            columns_holding(&cells, hex.as_bytes()),
            [("instance_requests", "envelope")]
        );

        assert_eq!(columns_holding(&cells, &content), [], "the content");
        assert_eq!(columns_holding(&cells, &digest), [], "the raw digest");
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_fetched_attachment_is_deleted_and_a_second_fetch_is_not_found() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db).await;
        let desktop = a.paired(&db).await;
        let base = serve_state(state_over(db.clone(), AttachmentSettings::default())).await;
        let id = a.sent(&base, &desktop, &content()).await;

        assert_eq!(desktop.fetch(&base, &id).await, (200, content()));
        assert_eq!(attachment_rows(&db).await, 0);
        assert_eq!(desktop.fetch(&base, &id).await, not_found());
    })
    .await;
}

/// Both fetches can read and open the row before either deletes it; only the
/// one whose `DELETE … RETURNING` took it may answer the content.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn of_two_fetches_at_once_exactly_one_hands_the_content_over() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db).await;
        let desktop = a.paired(&db).await;
        let base = serve_state(state_over(db.clone(), AttachmentSettings::default())).await;

        for _ in 0..8 {
            let id = a.sent(&base, &desktop, &content()).await;
            let (first, second) =
                tokio::join!(desktop.fetch(&base, &id), desktop.fetch(&base, &id));
            let mut answers = [first, second];
            answers.sort();
            assert_eq!(answers, [(200, content()), not_found()]);
        }
        assert_eq!(attachment_rows(&db).await, 0);
    })
    .await;
}

/// The row survives every refusal, so the owner's own fetch after them shows
/// the refusals were not a fetch that had already taken it.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn another_instance_or_another_tenant_fetching_an_attachment_is_not_found() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db).await;
        let b = Tenant::new(&db).await;
        let (own, sibling) = (a.paired(&db).await, a.paired(&db).await);
        let foreign = b.paired(&db).await;
        let base = serve_state(state_over(db.clone(), AttachmentSettings::default())).await;
        let id = a.sent(&base, &own, &content()).await;

        assert_eq!(sibling.fetch(&base, &id).await, not_found(), "same tenant");
        assert_eq!(foreign.fetch(&base, &id).await, not_found(), "other tenant");
        assert_eq!(attachment_rows(&db).await, 1);

        assert_eq!(own.fetch(&base, &id).await, (200, content()));
    })
    .await;
}

fn limited_to(max_bytes: u64) -> AttachmentSettings {
    AttachmentSettings {
        max_bytes,
        ..AttachmentSettings::default()
    }
}

/// An over-limit body to an instance that is not the tenant's is answered
/// `413`, where the same body at the limit is `404`: the extractor refused it
/// before the use-case could look the instance up, so that `413` is the
/// route's `DefaultBodyLimit`, and it is the configured size.
///
/// The limit is small because an over-limit body is never read: a large one
/// left unread can reset the connection before the client reads the answer.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_upload_past_the_configured_size_is_refused_by_the_route_before_the_use_case() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db).await;
        let desktop = a.paired(&db).await;
        let base = serve_state(state_over(db.clone(), limited_to(64))).await;
        let nobodys = format!("/v1/instances/{}/attachments", Uuid::new_v4());
        let own = desktop.at("/attachments");
        let too_large = (413, json!({"error": "attachment_too_large"}));

        assert_eq!(
            post_bytes_as(&base, &nobodys, &a.session, vec![7; 65]).await,
            too_large
        );
        assert_eq!(
            post_bytes_as(&base, &nobodys, &a.session, vec![7; 64]).await,
            (404, json!({"error": "not_found"}))
        );
        assert_eq!(
            post_bytes_as(&base, &own, &a.session, vec![7; 65]).await,
            too_large
        );
        assert_eq!(attachment_rows(&db).await, 0);
    })
    .await;
}

/// axum's own limit is 2 MiB, so a body past it being stored shows the
/// route's `DefaultBodyLimit` replaced it rather than sat under it.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_upload_past_axum_s_default_limit_and_at_the_configured_one_is_stored() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db).await;
        let desktop = a.paired(&db).await;
        let max_bytes = 3 * 1024 * 1024;
        let base = serve_state(state_over(db.clone(), limited_to(max_bytes))).await;
        let at_limit = vec![7; usize::try_from(max_bytes).unwrap()];

        let path = desktop.at("/attachments");
        let (status, staged) = post_bytes_as(&base, &path, &a.session, at_limit).await;

        assert_eq!(status, 201, "{staged}");
        assert_eq!(staged["size"], max_bytes);
        assert_eq!(attachment_rows(&db).await, 1);
    })
    .await;
}

/// Both dispatches can find the row staged before either binds it; the bind's
/// `request_id IS NULL` is re-read under the row lock, so the one that waited
/// binds nothing and its request is rolled back with it.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn of_two_dispatches_naming_one_attachment_at_once_exactly_one_is_queued() {
    in_fresh_schema(|db| async move {
        let a = Tenant::new(&db).await;
        let desktop = a.paired(&db).await;
        let base = serve_state(state_over(db.clone(), AttachmentSettings::default())).await;

        for round in 1..=8 {
            let id = a.upload(&base, &desktop, &content()).await;
            let (first, second) = tokio::join!(
                a.dispatch(&base, &desktop, &id),
                a.dispatch(&base, &desktop, &id)
            );
            let mut statuses = [first.0, second.0];
            statuses.sort_unstable();
            assert_eq!(statuses, [202, 409], "{first:?} {second:?}");
            let refused = if first.0 == 409 { first.1 } else { second.1 };
            assert_eq!(refused, json!({"error": "attachment_unavailable"}));
            assert_eq!(request_rows(&db).await, round);
        }
        let bound = count(
            &db,
            "SELECT count(*) FROM attachments WHERE request_id IS NOT NULL",
        )
        .await;
        assert_eq!(bound, 8);
    })
    .await;
}

fn staged(instance: InstanceId, expires_at: u64) -> NewAttachment {
    NewAttachment {
        id: Uuid::new_v4(),
        instance_id: instance,
        ciphertext: vec![1, 2, 3],
        wrapped_key: WrappedKey::parse("vault:v1:wrapped").unwrap(),
        sealed_digest: vec![9; 60],
        expires_at,
    }
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_sweep_deletes_every_tenant_s_expired_attachments_and_keeps_the_live_ones() {
    in_fresh_schema(|db| async move {
        let now = unix_now();
        let mut live = Vec::new();
        for _ in 0..2 {
            let tenant = Tenant::new(&db).await;
            let desktop = tenant.paired(&db).await;
            for expires_at in [now - 1, now] {
                let lapsed = staged(desktop.instance, expires_at);
                insert_attachment(tenant.user, &db, &lapsed).await.unwrap();
            }
            let waiting = staged(desktop.instance, now + 1);
            insert_attachment(tenant.user, &db, &waiting).await.unwrap();
            live.push(waiting.id);
        }
        let store = PostgresAttachments::new(db.clone());

        assert_eq!(sweep_once(&store, now).await.unwrap(), 4);

        let mut left: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM attachments")
            .fetch_all(&db)
            .await
            .unwrap();
        left.sort_unstable();
        live.sort_unstable();
        assert_eq!(left, live);
        assert_eq!(sweep_once(&store, now).await.unwrap(), 0);
    })
    .await;
}
