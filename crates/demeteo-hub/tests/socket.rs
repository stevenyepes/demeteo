//! What the instance socket asks of Postgres, against real rows.
//!
//! `src-tauri/tests/hub_socket.rs` holds every rule of a session to its
//! verdict over a store double, which applies what it is handed and so cannot
//! show that a statement does. What is left for this file is what only
//! Postgres can show: that a write's `WHERE` matches the row it names and no
//! other, and that a value read back is the one written.
//!
//! Every test here is ignored on the terms the header of
//! `crates/demeteo-hub/tests/pairing.rs` states, and shows as little when it
//! is.

mod support;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::pairing::grant::FreshPair;
use demeteo_hub::pairing::jwk::DeviceJwk;
use demeteo_hub::pairing::signing_key::SigningKey;
use demeteo_hub::pairing::testing::{verifies_ed25519, SoftDeviceKey};
use demeteo_hub::ports::KeyService;
use demeteo_hub::socket::connections::{ConnectionId, Connections};
use demeteo_hub::socket::handler::Admission;
use demeteo_hub::socket::postgres::PostgresStore;
use demeteo_hub::socket::requests::{enqueued, Enqueued};
use demeteo_hub::socket::session::{self, Ended, Limits};
use demeteo_hub::socket::testing::FakeInstance;
use demeteo_hub::store::device_keys::{instance_of_key, NewDeviceKey};
use demeteo_hub::store::instances::{
    granted_scopes, list_instances, pair_instance, record_departure, record_hello, Greeted,
    Instance, NewInstanceLabels, NewPairing,
};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::requests::{
    insert_request, list_queued_requests, mark_request_expired, mark_request_sent, record_outcome,
    NewRequest, QueuedEnvelope, QueuedRequest, RequestId, RequestState, RequestStatus,
};
use demeteo_hub::store::run_mirror::{mirror_event, NewEvent};
use demeteo_hub::store::snapshots::replace_snapshot;
use demeteo_hub::store::{Db, InstanceId, StoreError, UserId};
use demeteo_hub_protocol::{
    AttachmentRef, CancelFeature, Effort, Hello, ProjectSnapshot, RefusalReason, RequestOutcome,
    RequestPayload, RequestResult, RunEvent, RunEventKind, RunEventPayload, Scope, SignedRequest,
    Snapshot, StartFeature, StepOverride, PROTOCOL_VERSION,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use support::in_fresh_schema;
use time::OffsetDateTime;
use tokio::task::JoinHandle;
use uuid::Uuid;

const NOW: u64 = 1_700_000_000;
const SIGNING_KEY: &str = "request-signing";
const CHECK_VIOLATION: &str = "23514";
const UNIQUE_VIOLATION: &str = "23505";
const FOREIGN_KEY_VIOLATION: &str = "23503";

fn at(unix_seconds: u64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(i64::try_from(unix_seconds).unwrap()).unwrap()
}

/// A desktop paired as `laptop` on Linux at 1.2.0 with `read` and `spend`
/// left checked: the row a `hello` finds, and the thumbprint of its key.
async fn paired(db: &Db, user: UserId) -> (InstanceId, String) {
    let (instance, jkt, _) = paired_with_tokens(db, user).await;
    (instance, jkt)
}

/// As [`paired`], with the token pair the desktop was sent: the one place
/// their text exists once the grant has answered.
async fn paired_with_tokens(db: &Db, user: UserId) -> (InstanceId, String, FreshPair) {
    let jwk = DeviceJwk::parse(&SoftDeviceKey::new().jwk()).unwrap();
    let key = NewDeviceKey {
        public_jwk: jwk.canonical_json(),
        jkt: jwk.thumbprint(),
    };
    let family_id = Uuid::new_v4();
    let pair = FreshPair::generate().unwrap();
    let rows = pair.rows(family_id, &key.jkt, NOW).unwrap();
    let new = NewPairing {
        instance: NewInstanceLabels {
            name: "laptop".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            version: "1.2.0".to_owned(),
        },
        device_key: key,
        scopes: vec![Scope::Read, Scope::Spend],
        family_id,
        access: rows.access,
        refresh: rows.refresh,
    };
    let instance = pair_instance(user, db, &new).await.unwrap().instance_id;
    (instance, new.device_key.jkt, pair)
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

async fn the_instance(db: &Db, user: UserId) -> Instance {
    let [instance] = list_instances(user, db).await.unwrap().try_into().unwrap();
    instance
}

fn event(feature_id: &str, offset: u64, kind: RunEventKind) -> RunEvent {
    RunEvent {
        v: PROTOCOL_VERSION,
        feature_id: feature_id.to_owned(),
        offset,
        kind,
        occurred_at: NOW,
        payload: RunEventPayload {
            step_id: Some("s-implement".to_owned()),
            ..RunEventPayload::default()
        },
    }
}

fn as_mirrored(event: &RunEvent) -> NewEvent {
    NewEvent {
        feature_id: event.feature_id.clone(),
        event_offset: event.offset,
        event: serde_json::to_string(event).unwrap(),
    }
}

/// The instance's mirror rows as `(feature_id, event_offset, event)`, in key
/// order.
async fn mirror_of(db: &Db, user: UserId, instance: InstanceId) -> Vec<(String, i64, RunEvent)> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT feature_id, event_offset, event::text FROM run_mirror \
         WHERE user_id = $1 AND instance_id = $2 ORDER BY feature_id, event_offset",
    )
    .bind(user)
    .bind(instance)
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(feature_id, offset, event)| {
            (feature_id, offset, serde_json::from_str(&event).unwrap())
        })
        .collect()
}

fn snapshot(project_names: &[&str]) -> Snapshot {
    Snapshot {
        v: PROTOCOL_VERSION,
        projects: project_names
            .iter()
            .map(|name| ProjectSnapshot {
                id: format!("p-{name}"),
                name: (*name).to_owned(),
                remote_url: None,
            })
            .collect(),
        workflows: Vec::new(),
        run_shape: Vec::new(),
    }
}

/// The instance's snapshot rows as `(kind, body, updated_at)`, in kind order.
async fn snapshots_of(
    db: &Db,
    user: UserId,
    instance: InstanceId,
) -> Vec<(String, Snapshot, OffsetDateTime)> {
    let rows: Vec<(String, String, OffsetDateTime)> = sqlx::query_as(
        "SELECT kind, body::text, updated_at FROM snapshots \
         WHERE user_id = $1 AND instance_id = $2 ORDER BY kind",
    )
    .bind(user)
    .bind(instance)
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(kind, body, updated_at)| (kind, serde_json::from_str(&body).unwrap(), updated_at))
        .collect()
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_hello_records_every_field_it_reported_and_returns_the_stored_cursor() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        sqlx::query("UPDATE instances SET event_cursor = 41 WHERE user_id = $1 AND id = $2")
            .bind(user)
            .bind(id)
            .execute(&db)
            .await
            .unwrap();

        let as_paired = the_instance(&db, user).await;
        assert_eq!(as_paired.install_id, None);
        assert_eq!(as_paired.connection_id, None);
        assert_eq!(as_paired.connected_at, None);
        assert_eq!(as_paired.last_seen, None);
        assert_eq!(
            granted_scopes(user, &db, id).await.unwrap(),
            Some(vec![Scope::Read, Scope::Spend])
        );

        let first = Uuid::new_v4();
        let greeted = record_hello(
            user,
            &db,
            id,
            &hello("install-1", &[Scope::Gates]),
            first,
            NOW,
        )
        .await
        .unwrap();
        assert_eq!(
            greeted,
            Some(Greeted {
                event_cursor: 41,
                previous_install_id: None,
            })
        );
        let written = the_instance(&db, user).await;
        assert_eq!(written.id, id);
        assert_eq!(written.install_id.as_deref(), Some("install-1"));
        assert_eq!(written.version, "1.5.0");
        assert_eq!(written.os, "macos");
        assert_eq!(written.arch, "aarch64");
        assert_eq!(written.granted_scopes, ["gates"]);
        assert_eq!(written.connection_id, Some(first));
        assert_eq!(written.connected_at, Some(at(NOW)));
        assert_eq!(written.last_seen, Some(at(NOW)));
        assert_eq!(written.event_cursor, 41);
        assert_eq!(written.name, "laptop");
        assert_eq!(written.created_at, as_paired.created_at);
        assert_eq!(
            granted_scopes(user, &db, id).await.unwrap(),
            Some(vec![Scope::Gates])
        );

        let second = Uuid::new_v4();
        let reinstalled = hello("install-2", &[]);
        let greeted = record_hello(user, &db, id, &reinstalled, second, NOW + 60)
            .await
            .unwrap();
        assert_eq!(
            greeted,
            Some(Greeted {
                event_cursor: 41,
                previous_install_id: Some("install-1".to_owned()),
            })
        );
        let rewritten = the_instance(&db, user).await;
        assert_eq!(rewritten.install_id.as_deref(), Some("install-2"));
        assert_eq!(rewritten.connection_id, Some(second));
        assert_eq!(rewritten.connected_at, Some(at(NOW + 60)));
        assert!(rewritten.granted_scopes.is_empty());
        assert_eq!(granted_scopes(user, &db, id).await.unwrap(), Some(vec![]));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_hello_for_an_instance_that_was_never_paired_inserts_nothing() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let unpaired = InstanceId::generate();

        let greeted = record_hello(user, &db, unpaired, &hello("i", &[]), Uuid::new_v4(), NOW)
            .await
            .unwrap();
        assert_eq!(greeted, None);
        assert_eq!(granted_scopes(user, &db, unpaired).await.unwrap(), None);

        let only = the_instance(&db, user).await;
        assert_eq!(only.id, id);
        assert_eq!(only.install_id, None);
        assert_eq!(only.connection_id, None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_departure_by_a_displaced_connection_leaves_the_instance_present() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let (stale, current) = (Uuid::new_v4(), Uuid::new_v4());
        for (connection, now) in [(stale, NOW), (current, NOW + 10)] {
            let greeted = record_hello(user, &db, id, &hello("i", &[]), connection, now)
                .await
                .unwrap();
            assert!(greeted.is_some());
        }

        assert!(!record_departure(user, &db, id, stale, NOW + 20)
            .await
            .unwrap());
        let present = the_instance(&db, user).await;
        assert_eq!(present.connection_id, Some(current));
        assert_eq!(present.connected_at, Some(at(NOW + 10)));
        assert_eq!(present.last_seen, Some(at(NOW + 10)));

        assert!(record_departure(user, &db, id, current, NOW + 30)
            .await
            .unwrap());
        let absent = the_instance(&db, user).await;
        assert_eq!(absent.connection_id, None);
        assert_eq!(absent.connected_at, None);
        assert_eq!(absent.last_seen, Some(at(NOW + 30)));
        assert_eq!(absent.install_id.as_deref(), Some("i"));

        assert!(!record_departure(user, &db, id, current, NOW + 40)
            .await
            .unwrap());
        assert_eq!(
            the_instance(&db, user).await.last_seen,
            Some(at(NOW + 30)),
            "a connection that already left is as stale as a displaced one"
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn the_instance_of_a_key_is_found_until_the_key_is_revoked() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, jkt) = paired(&db, user).await;

        assert_eq!(instance_of_key(user, &db, &jkt).await.unwrap(), Some(id));
        let unpaired = SoftDeviceKey::new().thumbprint();
        assert_eq!(instance_of_key(user, &db, &unpaired).await.unwrap(), None);

        sqlx::query("UPDATE device_keys SET revoked_at = now() WHERE user_id = $1 AND jkt = $2")
            .bind(user)
            .bind(&jkt)
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(instance_of_key(user, &db, &jkt).await.unwrap(), None);
    })
    .await;
}

/// The turnstile is taken here the way a rotation or a re-pairing takes it,
/// and held. A lookup that asked for the row lock would wait behind it for
/// as long as this transaction stays open.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn the_instance_of_a_key_is_read_while_a_rotation_holds_the_keys_row() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, jkt) = paired(&db, user).await;

        let mut rotation = db.begin().await.unwrap();
        sqlx::query("SELECT id FROM device_keys WHERE user_id = $1 AND jkt = $2 FOR UPDATE")
            .bind(user)
            .bind(&jkt)
            .fetch_one(&mut *rotation)
            .await
            .unwrap();

        let read = tokio::time::timeout(Duration::from_secs(5), instance_of_key(user, &db, &jkt))
            .await
            .expect("the lookup waited for the row lock");
        assert_eq!(read.unwrap(), Some(id));
        rotation.rollback().await.unwrap();
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_stored_scope_name_the_protocol_does_not_have_fails_the_read() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        sqlx::query(
            "UPDATE instances SET granted_scopes = ARRAY['read', 'logs'] \
             WHERE user_id = $1 AND id = $2",
        )
        .bind(user)
        .bind(id)
        .execute(&db)
        .await
        .unwrap();

        let err = granted_scopes(user, &db, id).await.unwrap_err();
        assert!(
            matches!(&err, StoreError::UnknownScope(name) if name == "logs"),
            "{err}"
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_time_past_the_column_is_refused_before_any_row_is_written() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let connection = Uuid::new_v4();

        let err = record_hello(user, &db, id, &hello("i", &[]), connection, u64::MAX)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::TimeTooLarge(u64::MAX)), "{err}");
        assert_eq!(the_instance(&db, user).await.install_id, None);

        record_hello(user, &db, id, &hello("i", &[]), connection, NOW)
            .await
            .unwrap();
        let err = record_departure(user, &db, id, connection, u64::MAX)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::TimeTooLarge(u64::MAX)), "{err}");
        assert_eq!(
            the_instance(&db, user).await.connection_id,
            Some(connection)
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_event_the_mirror_already_holds_is_dropped_and_the_first_copy_kept() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let to_the_offset = |_before: u64, offset: u64| offset;

        let first = event("f-1", 7, RunEventKind::StepStarted);
        let stored = mirror_event(user, &db, id, &as_mirrored(&first), to_the_offset)
            .await
            .unwrap();
        assert_eq!(stored, Some(7));
        assert_eq!(
            mirror_of(&db, user, id).await,
            [("f-1".to_owned(), 7, first.clone())]
        );

        let resent_differently = event("f-1", 7, RunEventKind::StepFailed);
        let again = mirror_event(
            user,
            &db,
            id,
            &as_mirrored(&resent_differently),
            to_the_offset,
        )
        .await
        .unwrap();
        assert_eq!(again, Some(7));
        assert_eq!(
            mirror_of(&db, user, id).await,
            [("f-1".to_owned(), 7, first.clone())]
        );

        let of_another_feature = event("f-2", 7, RunEventKind::FeatureStarted);
        mirror_event(
            user,
            &db,
            id,
            &as_mirrored(&of_another_feature),
            to_the_offset,
        )
        .await
        .unwrap();
        assert_eq!(
            mirror_of(&db, user, id).await,
            [
                ("f-1".to_owned(), 7, first),
                ("f-2".to_owned(), 7, of_another_feature),
            ]
        );
    })
    .await;
}

/// Each answer is one a cursor rule would never give: below the offset just
/// stored, far above it, then backwards. A store that compared offsets
/// itself would write something else for at least one of them.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn the_cursor_written_is_what_decide_answered_about_the_cursor_it_was_shown() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;

        let asked_and_answered = [(9, 0, 3), (2, 3, 40), (9, 40, 5), (11, 5, 5)];
        for (offset, shown_before, answer) in asked_and_answered {
            let new = as_mirrored(&event("f-1", offset, RunEventKind::StepCompleted));
            let mut shown = None;
            let written = mirror_event(user, &db, id, &new, |before, offset| {
                shown = Some((before, offset));
                answer
            })
            .await
            .unwrap();
            assert_eq!(shown, Some((shown_before, offset)));
            assert_eq!(written, Some(answer));
            assert_eq!(
                the_instance(&db, user).await.event_cursor,
                i64::try_from(answer).unwrap()
            );
        }
        let offsets_stored: Vec<i64> = mirror_of(&db, user, id)
            .await
            .into_iter()
            .map(|(_, offset, _)| offset)
            .collect();
        assert_eq!(offsets_stored, [2, 9, 11]);
    })
    .await;
}

/// Both halves fail after the other was reached: the cursor after the row
/// was inserted, and the row after the instance was locked for its cursor.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_event_and_its_cursor_are_written_together_or_not_at_all() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let kept = event("f-1", 4, RunEventKind::StepStarted);
        mirror_event(user, &db, id, &as_mirrored(&kept), |_, offset| offset)
            .await
            .unwrap();
        let as_it_was = [("f-1".to_owned(), 4, kept)];

        let new = as_mirrored(&event("f-1", 5, RunEventKind::StepCompleted));
        let err = mirror_event(user, &db, id, &new, |_, _| u64::MAX)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::OffsetTooLarge(u64::MAX)), "{err}");
        assert_eq!(mirror_of(&db, user, id).await, as_it_was);
        assert_eq!(the_instance(&db, user).await.event_cursor, 4);

        let not_json = NewEvent {
            event: "{".to_owned(),
            ..new.clone()
        };
        let err = mirror_event(user, &db, id, &not_json, |_, _| {
            panic!("asked about an event that was not stored")
        })
        .await
        .unwrap_err();
        assert!(matches!(err, StoreError::Database(_)), "{err}");
        assert_eq!(mirror_of(&db, user, id).await, as_it_was);
        assert_eq!(the_instance(&db, user).await.event_cursor, 4);

        let stored = mirror_event(user, &db, id, &new, |_, offset| offset)
            .await
            .unwrap();
        assert_eq!(stored, Some(5));
        assert_eq!(mirror_of(&db, user, id).await.len(), 2);
        assert_eq!(the_instance(&db, user).await.event_cursor, 5);
    })
    .await;
}

/// Counting is the one answer that shows a lost turn: every event adds one to
/// the cursor it was shown, so two that were shown the same cursor leave the
/// total one short.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn events_of_one_instance_are_each_shown_the_cursor_the_one_before_wrote() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;

        let at_once: Vec<_> = (1..=16)
            .map(|offset| {
                let db = db.clone();
                let new = as_mirrored(&event("f-1", offset, RunEventKind::StepStarted));
                tokio::spawn(async move {
                    mirror_event(user, &db, id, &new, |before, _| before + 1)
                        .await
                        .unwrap()
                })
            })
            .collect();
        let mut written = Vec::new();
        for storing in at_once {
            written.push(storing.await.unwrap().unwrap());
        }
        written.sort_unstable();

        assert_eq!(written, (1..=16).collect::<Vec<u64>>());
        assert_eq!(the_instance(&db, user).await.event_cursor, 16);
        assert_eq!(mirror_of(&db, user, id).await.len(), 16);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_event_with_no_instance_or_an_offset_past_the_column_is_not_stored() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let never_asked = |_: u64, _: u64| -> u64 { panic!("asked about an event not stored") };

        let new = as_mirrored(&event("f-1", 1, RunEventKind::FeatureStarted));
        let unpaired = InstanceId::generate();
        let stored = mirror_event(user, &db, unpaired, &new, never_asked)
            .await
            .unwrap();
        assert_eq!(stored, None);
        assert!(mirror_of(&db, user, unpaired).await.is_empty());

        let past_the_column = NewEvent {
            event_offset: u64::MAX,
            ..new
        };
        let err = mirror_event(user, &db, id, &past_the_column, never_asked)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::OffsetTooLarge(u64::MAX)), "{err}");
        assert!(mirror_of(&db, user, id).await.is_empty());
        assert_eq!(the_instance(&db, user).await.event_cursor, 0);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_second_snapshot_leaves_one_row_holding_the_last_body() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        assert!(snapshots_of(&db, user, id).await.is_empty());

        let first = snapshot(&["demeteo", "notes"]);
        let body = serde_json::to_string(&first).unwrap();
        assert!(replace_snapshot(user, &db, id, "full", &body, NOW)
            .await
            .unwrap());
        assert_eq!(
            snapshots_of(&db, user, id).await,
            [("full".to_owned(), first, at(NOW))]
        );

        let second = snapshot(&["notes"]);
        let body = serde_json::to_string(&second).unwrap();
        assert!(replace_snapshot(user, &db, id, "full", &body, NOW + 60)
            .await
            .unwrap());
        assert_eq!(
            snapshots_of(&db, user, id).await,
            [("full".to_owned(), second.clone(), at(NOW + 60))]
        );

        let err = replace_snapshot(user, &db, id, "full", "{}", u64::MAX)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::TimeTooLarge(u64::MAX)), "{err}");

        let of_another_kind = snapshot(&[]);
        let body = serde_json::to_string(&of_another_kind).unwrap();
        assert!(replace_snapshot(user, &db, id, "empty", &body, NOW + 90)
            .await
            .unwrap());
        assert_eq!(
            snapshots_of(&db, user, id).await,
            [
                ("empty".to_owned(), of_another_kind, at(NOW + 90)),
                ("full".to_owned(), second, at(NOW + 60)),
            ]
        );
    })
    .await;
}

fn cancel(feature_id: &str) -> RequestPayload {
    RequestPayload::CancelFeature(CancelFeature {
        feature_id: feature_id.to_owned(),
    })
}

/// A request for `instance` with every column taken from the one envelope,
/// as whoever enqueues one builds it. Unsigned: only the test that verifies
/// a signature puts one in.
fn request(
    id: RequestId,
    instance: InstanceId,
    payload: RequestPayload,
    expires_at: u64,
) -> NewRequest {
    let kind = serde_json::to_value(&payload).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_owned();
    NewRequest {
        id,
        kind,
        envelope: SignedRequest {
            v: PROTOCOL_VERSION,
            request_id: URL_SAFE_NO_PAD.encode(id),
            instance_id: instance.0.to_string(),
            issued_at: NOW,
            expires_at,
            payload,
            signature: String::new(),
        },
        expires_at,
    }
}

fn as_queued(new: &NewRequest, deliveries: u32) -> QueuedRequest {
    QueuedRequest {
        id: new.id,
        envelope: QueuedEnvelope::Readable(Box::new(new.envelope.clone())),
        expires_at: new.expires_at,
        deliveries,
    }
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct StoredRequest {
    id: Vec<u8>,
    kind: String,
    status: String,
    expires_at: OffsetDateTime,
    deliveries: i32,
    last_sent_at: Option<OffsetDateTime>,
    outcome: Option<String>,
    refusal_reason: Option<String>,
    answered_at: Option<OffsetDateTime>,
}

impl StoredRequest {
    /// The row [`insert_request`] leaves for `new`.
    fn as_inserted(new: &NewRequest) -> Self {
        Self {
            id: new.id.to_vec(),
            kind: new.kind.clone(),
            status: "queued".to_owned(),
            expires_at: at(new.expires_at),
            deliveries: 0,
            last_sent_at: None,
            outcome: None,
            refusal_reason: None,
            answered_at: None,
        }
    }
}

/// The instance's request rows of every status, in queue order.
async fn requests_of(db: &Db, user: UserId, instance: InstanceId) -> Vec<StoredRequest> {
    sqlx::query_as(
        "SELECT id, kind, status, expires_at, deliveries, last_sent_at, outcome, \
                refusal_reason, answered_at \
         FROM instance_requests WHERE user_id = $1 AND instance_id = $2 ORDER BY queue_seq",
    )
    .bind(user)
    .bind(instance)
    .fetch_all(db)
    .await
    .unwrap()
}

fn never_asked(state: RequestState) -> Option<RequestOutcome> {
    panic!("asked about a request that was not found: {state:?}")
}

/// The ids fall in the reverse of the order they are queued in, and so do
/// the first two expiries, so a list ordered by the primary key or by
/// `expires_at` comes back wrong. The second request is already overdue at
/// `NOW`: a statement that judged expiry would leave it out.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn queued_requests_come_back_in_the_order_they_were_queued_from_a_pool_that_never_wrote_them()
{
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let (other, _) = paired(&db, user).await;
        assert!(list_queued_requests(user, &db, id)
            .await
            .unwrap()
            .is_empty());

        let queued = [
            request([0xc0; 16], id, cancel("f-first"), NOW + 300),
            request([0x60; 16], id, cancel("f-second"), NOW - 1),
            request([0x10; 16], id, cancel("f-third"), NOW + 100),
        ];
        let of_the_other = request([0xc0; 16], other, cancel("f-other"), NOW + 300);
        insert_request(user, &db, id, &queued[0]).await.unwrap();
        insert_request(user, &db, other, &of_the_other)
            .await
            .unwrap();
        for new in &queued[1..] {
            insert_request(user, &db, id, new).await.unwrap();
        }
        assert_eq!(
            requests_of(&db, user, id).await,
            queued
                .iter()
                .map(StoredRequest::as_inserted)
                .collect::<Vec<_>>()
        );

        let options = db.connect_options().as_ref().clone();
        db.close().await;
        let reconnected = PgPoolOptions::new().connect_with(options).await.unwrap();
        let in_queue_order: Vec<_> = queued.iter().map(|new| as_queued(new, 0)).collect();
        assert_eq!(
            list_queued_requests(user, &reconnected, id).await.unwrap(),
            in_queue_order
        );
        assert_eq!(
            list_queued_requests(user, &reconnected, other)
                .await
                .unwrap(),
            [as_queued(&of_the_other, 0)]
        );
        reconnected.close().await;
    })
    .await;
}

/// `JSONB` hands back other text than it was given, which the test checks
/// first: were the text kept, a signature that verifies afterwards would
/// say nothing about the round trip.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_signed_request_read_back_from_jsonb_still_verifies_under_the_key_that_signed_it() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let keys = MemoryKeys::with_keys([SIGNING_KEY]);

        let payload = RequestPayload::StartFeature(StartFeature {
            project_id: "p-1".to_owned(),
            workflow_id: "wf-default".to_owned(),
            title: "Größe — 見出し \"quoted\"".to_owned(),
            description: "line one\nline two \\ {\"not\": \"json\"}".to_owned(),
            agent_kind: Some("claude-code".to_owned()),
            model: None,
            effort: Some(Effort::Xhigh),
            step_overrides: vec![StepOverride {
                step_id: "s-implement".to_owned(),
                agent_kind: None,
                model: Some("opus".to_owned()),
                effort: Some(Effort::Low),
            }],
            max_budget_cents: Some(u64::MAX),
            max_wall_clock_secs: None,
            attachments: vec![AttachmentRef {
                id: "a-1".to_owned(),
                size: u64::MAX - 1,
                sha256: "ab".repeat(32),
            }],
        });
        let mut new = request([0x5a; 16], id, payload, NOW + 300);
        let signed = new.envelope.signing_bytes().unwrap();
        let signature = keys.sign(SIGNING_KEY, &signed).await.unwrap();
        new.envelope.signature = URL_SAFE_NO_PAD.encode(signature);
        insert_request(user, &db, id, &new).await.unwrap();

        let stored: String = sqlx::query_scalar(
            "SELECT envelope::text FROM instance_requests WHERE user_id = $1 AND instance_id = $2",
        )
        .bind(user)
        .bind(id)
        .fetch_one(&db)
        .await
        .unwrap();
        assert_ne!(stored, serde_json::to_string(&new.envelope).unwrap());

        let options = db.connect_options().as_ref().clone();
        db.close().await;
        let reconnected = PgPoolOptions::new().connect_with(options).await.unwrap();
        let [read] = list_queued_requests(user, &reconnected, id)
            .await
            .unwrap()
            .try_into()
            .unwrap();
        reconnected.close().await;
        assert_eq!(read, as_queued(&new, 0));
        let QueuedEnvelope::Readable(envelope) = read.envelope else {
            panic!("the envelope just stored did not read back");
        };
        assert_eq!(envelope.request_id, URL_SAFE_NO_PAD.encode(read.id));

        let public_key = keys.verifying_key(SIGNING_KEY).await.unwrap();
        let signature = URL_SAFE_NO_PAD.decode(&envelope.signature).unwrap();
        let rebuilt = envelope.signing_bytes().unwrap();
        assert!(verifies_ed25519(&public_key, &rebuilt, &signature));

        let mut rewritten = envelope.clone();
        rewritten.expires_at += 1;
        let rewritten = rewritten.signing_bytes().unwrap();
        assert!(!verifies_ed25519(&public_key, &rewritten, &signature));
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_request_that_cannot_be_queued_stores_nothing() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let kept = request([1; 16], id, cancel("f-1"), NOW + 300);
        insert_request(user, &db, id, &kept).await.unwrap();
        let as_it_was = [StoredRequest::as_inserted(&kept)];

        let under_the_same_id = request([1; 16], id, cancel("f-2"), NOW + 600);
        let unpaired = InstanceId::generate();
        let for_no_instance = request([2; 16], unpaired, cancel("f-1"), NOW + 300);
        let with_a_nul = request([3; 16], id, cancel("f-\u{0}"), NOW + 300);
        for (instance, new, code) in [
            (id, &under_the_same_id, Some(UNIQUE_VIOLATION)),
            (unpaired, &for_no_instance, Some(FOREIGN_KEY_VIOLATION)),
            (id, &with_a_nul, None),
        ] {
            let err = insert_request(user, &db, instance, new).await.unwrap_err();
            let StoreError::Database(err) = err else {
                panic!("{err}");
            };
            if let Some(code) = code {
                let reported = err.as_database_error().and_then(|e| e.code());
                assert_eq!(reported.as_deref(), Some(code), "{err}");
            }
        }

        let past_the_column = request([4; 16], id, cancel("f-1"), u64::MAX);
        let err = insert_request(user, &db, id, &past_the_column)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::TimeTooLarge(u64::MAX)), "{err}");

        assert_eq!(requests_of(&db, user, id).await, as_it_was);
        assert!(requests_of(&db, user, unpaired).await.is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn sending_counts_a_delivery_and_expiring_takes_a_request_off_the_queue_and_keeps_its_row() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let (other, _) = paired(&db, user).await;
        let sent = request([1; 16], id, cancel("f-1"), NOW + 300);
        let overdue = request([2; 16], id, cancel("f-2"), NOW - 1);
        for new in [&sent, &overdue] {
            insert_request(user, &db, id, new).await.unwrap();
        }
        let under_the_same_ids = [
            request(sent.id, other, cancel("f-1"), NOW + 300),
            request(overdue.id, other, cancel("f-2"), NOW - 1),
        ];
        for new in &under_the_same_ids {
            insert_request(user, &db, other, new).await.unwrap();
        }

        for (now, deliveries) in [(NOW + 5, 1), (NOW + 9, 2)] {
            assert!(mark_request_sent(user, &db, id, &sent.id, now)
                .await
                .unwrap());
            assert_eq!(
                list_queued_requests(user, &db, id).await.unwrap(),
                [as_queued(&sent, deliveries), as_queued(&overdue, 0)]
            );
        }
        let sent_twice = StoredRequest {
            deliveries: 2,
            last_sent_at: Some(at(NOW + 9)),
            ..StoredRequest::as_inserted(&sent)
        };
        assert_eq!(
            requests_of(&db, user, id).await,
            [sent_twice.clone(), StoredRequest::as_inserted(&overdue)]
        );

        assert!(mark_request_expired(user, &db, id, &overdue.id)
            .await
            .unwrap());
        assert_eq!(
            list_queued_requests(user, &db, id).await.unwrap(),
            [as_queued(&sent, 2)]
        );
        let expired = StoredRequest {
            status: "expired".to_owned(),
            ..StoredRequest::as_inserted(&overdue)
        };
        assert_eq!(
            requests_of(&db, user, id).await,
            [sent_twice.clone(), expired.clone()]
        );
        assert!(
            !mark_request_expired(user, &db, id, &overdue.id)
                .await
                .unwrap(),
            "a request is taken off the queue once"
        );

        assert!(mark_request_sent(user, &db, id, &overdue.id, NOW + 20)
            .await
            .unwrap());
        let expired_and_sent = StoredRequest {
            deliveries: 1,
            last_sent_at: Some(at(NOW + 20)),
            ..expired
        };

        let unknown = [9; 16];
        assert!(!mark_request_sent(user, &db, id, &unknown, NOW + 30)
            .await
            .unwrap());
        assert!(!mark_request_expired(user, &db, id, &unknown).await.unwrap());
        let err = mark_request_sent(user, &db, id, &sent.id, u64::MAX)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::TimeTooLarge(u64::MAX)), "{err}");
        assert_eq!(
            requests_of(&db, user, id).await,
            [sent_twice.clone(), expired_and_sent.clone()]
        );

        let accepted = |_: RequestState| Some(RequestOutcome::Accepted);
        assert!(record_outcome(user, &db, id, &sent.id, NOW + 40, accepted)
            .await
            .unwrap());
        assert!(
            !mark_request_expired(user, &db, id, &sent.id).await.unwrap(),
            "an answered request keeps its outcome"
        );
        let answered = StoredRequest {
            status: "answered".to_owned(),
            outcome: Some("accepted".to_owned()),
            answered_at: Some(at(NOW + 40)),
            ..sent_twice
        };
        assert_eq!(
            requests_of(&db, user, id).await,
            [answered, expired_and_sent]
        );
        assert_eq!(
            requests_of(&db, user, other).await,
            under_the_same_ids
                .iter()
                .map(StoredRequest::as_inserted)
                .collect::<Vec<_>>(),
            "an id names a request of one instance"
        );
    })
    .await;
}

/// The second instance holds one request under an id the first does not
/// have and one under an id it does, so a read or a write that looked past
/// the instance named has a row to find either way.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_outcome_is_what_decide_answered_about_the_row_of_this_instance_and_no_other() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let (other, _) = paired(&db, user).await;
        let asked = request([1; 16], id, cancel("f-1"), NOW + 300);
        let overdue = request([2; 16], id, cancel("f-2"), NOW - 1);
        let only_the_others = request([3; 16], other, cancel("f-3"), NOW + 300);
        let under_the_same_id = request(asked.id, other, cancel("f-4"), NOW + 300);
        for (instance, new) in [
            (id, &asked),
            (id, &overdue),
            (other, &only_the_others),
            (other, &under_the_same_id),
        ] {
            insert_request(user, &db, instance, new).await.unwrap();
        }
        let untouched = [
            StoredRequest::as_inserted(&only_the_others),
            StoredRequest::as_inserted(&under_the_same_id),
        ];

        let found = record_outcome(user, &db, id, &only_the_others.id, NOW, never_asked)
            .await
            .unwrap();
        assert!(!found);
        assert_eq!(requests_of(&db, user, other).await, untouched);

        let mut shown = None;
        let found = record_outcome(user, &db, id, &asked.id, NOW + 10, |state| {
            shown = Some(state);
            None
        })
        .await
        .unwrap();
        assert!(found);
        let queued = RequestState {
            status: RequestStatus::Queued,
            deliveries: 0,
            outcome: None,
        };
        assert_eq!(shown, Some(queued));
        assert_eq!(
            requests_of(&db, user, id).await[0],
            StoredRequest::as_inserted(&asked)
        );

        mark_request_sent(user, &db, id, &asked.id, NOW + 20)
            .await
            .unwrap();
        let refused = RequestOutcome::Refused {
            reason: RefusalReason::OverRunCeiling,
        };
        let found = record_outcome(user, &db, id, &asked.id, NOW + 30, |state| {
            shown = Some(state);
            Some(refused)
        })
        .await
        .unwrap();
        assert!(found);
        assert_eq!(
            shown,
            Some(RequestState {
                deliveries: 1,
                ..queued
            })
        );
        let answered = StoredRequest {
            status: "answered".to_owned(),
            deliveries: 1,
            last_sent_at: Some(at(NOW + 20)),
            outcome: Some("refused".to_owned()),
            refusal_reason: Some("over_run_ceiling".to_owned()),
            answered_at: Some(at(NOW + 30)),
            ..StoredRequest::as_inserted(&asked)
        };
        assert_eq!(requests_of(&db, user, id).await[0], answered);
        assert!(list_queued_requests(user, &db, id)
            .await
            .unwrap()
            .iter()
            .all(|queued| queued.id != asked.id));

        let found = record_outcome(user, &db, id, &asked.id, NOW + 40, |state| {
            shown = Some(state);
            None
        })
        .await
        .unwrap();
        assert!(found);
        let as_answered = RequestState {
            status: RequestStatus::Answered,
            deliveries: 1,
            outcome: Some(refused),
        };
        assert_eq!(shown, Some(as_answered));
        assert_eq!(requests_of(&db, user, id).await[0], answered);

        let accepted = |_: RequestState| Some(RequestOutcome::Accepted);
        assert!(record_outcome(user, &db, id, &asked.id, NOW + 50, accepted)
            .await
            .unwrap());
        let overwritten = StoredRequest {
            outcome: Some("accepted".to_owned()),
            refusal_reason: None,
            answered_at: Some(at(NOW + 50)),
            ..answered
        };
        assert_eq!(requests_of(&db, user, id).await[0], overwritten);

        mark_request_expired(user, &db, id, &overdue.id)
            .await
            .unwrap();
        let found = record_outcome(user, &db, id, &overdue.id, NOW + 60, |state| {
            shown = Some(state);
            Some(RequestOutcome::Refused {
                reason: RefusalReason::Expired,
            })
        })
        .await
        .unwrap();
        assert!(found);
        assert_eq!(
            shown,
            Some(RequestState {
                status: RequestStatus::Expired,
                ..queued
            })
        );
        let answered_late = StoredRequest {
            status: "answered".to_owned(),
            outcome: Some("refused".to_owned()),
            refusal_reason: Some("expired".to_owned()),
            answered_at: Some(at(NOW + 60)),
            ..StoredRequest::as_inserted(&overdue)
        };
        assert_eq!(
            requests_of(&db, user, id).await,
            [overwritten.clone(), answered_late.clone()]
        );

        let err = record_outcome(user, &db, id, &asked.id, u64::MAX, never_asked)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::TimeTooLarge(u64::MAX)), "{err}");
        assert_eq!(
            requests_of(&db, user, id).await,
            [overwritten, answered_late]
        );
        assert_eq!(requests_of(&db, user, other).await, untouched);
    })
    .await;
}

/// Every refused statement breaks exactly one constraint of migration 0006,
/// so the name reported is the one the case is about.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_status_outcome_and_reason_that_disagree_are_refused_by_the_table() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let new = request([1; 16], id, cancel("f-1"), NOW + 300);
        insert_request(user, &db, id, &new).await.unwrap();
        let as_inserted = [StoredRequest::as_inserted(&new)];

        let set = |assignments: &'static str| {
            let db = db.clone();
            async move {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "UPDATE instance_requests SET {assignments} \
                     WHERE user_id = $1 AND instance_id = $2"
                )))
                .bind(user)
                .bind(id)
                .execute(&db)
                .await
            }
        };

        let refused = [
            ("status = 'sent'", "instance_requests_status_check"),
            ("status = 'answered'", "instance_requests_answered_check"),
            ("outcome = 'accepted'", "instance_requests_answered_check"),
            (
                "status = 'expired', outcome = 'accepted'",
                "instance_requests_answered_check",
            ),
            (
                "status = 'answered', outcome = 'maybe'",
                "instance_requests_outcome_check",
            ),
            (
                "status = 'answered', outcome = 'refused'",
                "instance_requests_reason_check",
            ),
            (
                "status = 'answered', outcome = 'accepted', refusal_reason = 'expired'",
                "instance_requests_reason_check",
            ),
            (
                "refusal_reason = 'expired'",
                "instance_requests_reason_check",
            ),
            ("deliveries = -1", "instance_requests_deliveries_check"),
        ];
        for (assignments, constraint) in refused {
            let err = set(assignments)
                .await
                .expect_err("an inconsistent row was stored");
            let reported = err.as_database_error().unwrap();
            assert_eq!(
                reported.code().as_deref(),
                Some(CHECK_VIOLATION),
                "{assignments}: {err}"
            );
            assert_eq!(
                reported.constraint(),
                Some(constraint),
                "{assignments}: {err}"
            );
            assert_eq!(requests_of(&db, user, id).await, as_inserted);
        }

        for consistent in [
            "status = 'expired'",
            "status = 'answered', outcome = 'accepted'",
            "status = 'answered', outcome = 'refused', refusal_reason = 'expired'",
            "status = 'queued', outcome = NULL, refusal_reason = NULL",
        ] {
            let done = set(consistent).await.unwrap();
            assert_eq!(done.rows_affected(), 1, "{consistent}");
        }
    })
    .await;
}

/// Both rows pass every `CHECK`: the table holds the envelope as any JSON
/// and the reason as any text, and only the read knows better. An envelope
/// is one row's fact and an outcome the whole answer, so the two end apart.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_stored_envelope_the_protocol_does_not_have_is_unreadable_and_a_reason_fails_the_read() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let new = request([1; 16], id, cancel("f-1"), NOW + 300);
        insert_request(user, &db, id, &new).await.unwrap();

        sqlx::query(
            "UPDATE instance_requests SET envelope = jsonb_set(envelope, '{extra}', '1') \
             WHERE user_id = $1 AND instance_id = $2",
        )
        .bind(user)
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
        assert_eq!(
            list_queued_requests(user, &db, id).await.unwrap(),
            [QueuedRequest {
                envelope: QueuedEnvelope::Unreadable,
                ..as_queued(&new, 0)
            }]
        );

        sqlx::query(
            "UPDATE instance_requests \
             SET status = 'answered', outcome = 'refused', refusal_reason = 'out_of_patience' \
             WHERE user_id = $1 AND instance_id = $2",
        )
        .bind(user)
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
        let err = record_outcome(user, &db, id, &new.id, NOW, never_asked)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, StoreError::UnknownOutcome(stored) if stored == "refused out_of_patience"),
            "{err}"
        );
        let [row] = requests_of(&db, user, id).await.try_into().unwrap();
        assert_eq!(row.refusal_reason.as_deref(), Some("out_of_patience"));
        assert_eq!(row.answered_at, None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_envelope_that_is_no_signed_request_is_listed_unreadable_beside_one_that_reads() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let (id, _) = paired(&db, user).await;
        let spoiled = request([1; 16], id, cancel("f-1"), NOW + 300);
        let neighbour = request([2; 16], id, cancel("f-2"), NOW + 300);
        insert_request(user, &db, id, &spoiled).await.unwrap();
        insert_request(user, &db, id, &neighbour).await.unwrap();

        let done = sqlx::query(
            "UPDATE instance_requests SET envelope = '{}'::jsonb \
             WHERE user_id = $1 AND instance_id = $2 AND id = $3",
        )
        .bind(user)
        .bind(id)
        .bind(spoiled.id.as_slice())
        .execute(&db)
        .await
        .unwrap();
        assert_eq!(done.rows_affected(), 1);

        assert_eq!(
            list_queued_requests(user, &db, id).await.unwrap(),
            [
                QueuedRequest {
                    envelope: QueuedEnvelope::Unreadable,
                    ..as_queued(&spoiled, 0)
                },
                as_queued(&neighbour, 0),
            ]
        );
    })
    .await;
}

/// No ping is sent and no peer given up on within any wait in this file: how
/// long a session lives is `src-tauri/tests/hub_socket.rs`'s to show.
const UNHURRIED: Limits = Limits {
    ping_interval: Duration::from_secs(3600),
    pong_deadline: Duration::from_secs(3600),
};

/// How long a script waits on a session whose every step is a round trip to
/// Postgres. Measured against nothing, and spent only by a test that fails.
const PATIENCE: Duration = Duration::from_secs(10);

const TOKEN_EXPIRES_AT: u64 = NOW + 900;
const STORE_FAILED: u16 = 1011;

/// What the socket route gives a session, less the route: the Postgres store
/// of one instance, the registry its connections are seated in and a clock
/// the test moves.
struct Served {
    db: Db,
    user: UserId,
    instance: InstanceId,
    connections: Arc<Connections>,
    now: Arc<AtomicU64>,
}

impl Served {
    fn over(db: &Db, user: UserId, instance: InstanceId) -> Self {
        Self {
            db: db.clone(),
            user,
            instance,
            connections: Arc::new(Connections::default()),
            now: Arc::new(AtomicU64::new(NOW)),
        }
    }

    /// A new tenant's one instance as [`paired`] leaves it, and the tokens
    /// its desktop holds.
    async fn paired(db: &Db) -> (Self, FreshPair) {
        let user = create_user(db).await.unwrap();
        let (instance, _, tokens) = paired_with_tokens(db, user).await;
        (Self::over(db, user, instance), tokens)
    }

    fn store(&self) -> PostgresStore {
        PostgresStore::new(self.db.clone(), self.user, self.instance)
    }

    fn clock_reads(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }

    /// A session seated, run to its end and released, with the id it is
    /// seated under.
    fn connect(&self) -> (FakeInstance, ConnectionId, JoinHandle<Ended>) {
        let (peer, transport) = FakeInstance::connect();
        let registered = self.connections.register(self.instance).unwrap();
        let connection = registered.connection;
        let admission = Admission {
            user: self.user,
            instance: self.instance,
            token_expires_at: TOKEN_EXPIRES_AT,
        };
        let now = Arc::clone(&self.now);
        let clock = move || now.load(Ordering::SeqCst);
        let store = self.store();
        let connections = Arc::clone(&self.connections);
        let instance = self.instance;
        let session = tokio::spawn(async move {
            let ended =
                session::run(transport, store, registered, admission, clock, UNHURRIED).await;
            connections.release(instance, connection);
            ended
        });
        (peer.waiting_at_most(PATIENCE), connection, session)
    }

    /// A session whose `hello` was acked with `cursor`.
    async fn greeted(
        &self,
        hello: &Hello,
        cursor: u64,
    ) -> (FakeInstance, ConnectionId, JoinHandle<Ended>) {
        let (mut peer, connection, session) = self.connect();
        peer.send(hello);
        peer.expect_ack(cursor).await;
        (peer, connection, session)
    }

    /// Queues `payload` as a route would, signed by `keys`, at the time the
    /// clock reads.
    async fn enqueue(&self, keys: &MemoryKeys, payload: RequestPayload) -> Enqueued {
        enqueued(
            &self.store(),
            keys,
            &SigningKey::new(SIGNING_KEY),
            &self.connections,
            payload,
            self.now.load(Ordering::SeqCst),
        )
        .await
        .unwrap()
    }

    async fn row(&self) -> Instance {
        the_instance(&self.db, self.user).await
    }

    async fn requests(&self) -> Vec<StoredRequest> {
        requests_of(&self.db, self.user, self.instance).await
    }
}

async fn ended(session: JoinHandle<Ended>) -> Ended {
    tokio::time::timeout(PATIENCE, session)
        .await
        .expect("the session to end")
        .expect("the session not to panic")
}

/// Whether `envelope` carries the signature `keys` makes over what it says.
async fn verifies(keys: &MemoryKeys, envelope: &SignedRequest) -> bool {
    let public_key = keys.verifying_key(SIGNING_KEY).await.unwrap();
    let signature = URL_SAFE_NO_PAD.decode(&envelope.signature).unwrap();
    let signed = envelope.signing_bytes().unwrap();
    verifies_ed25519(&public_key, &signed, &signature)
}

fn result(request: &SignedRequest, outcome: RequestOutcome) -> RequestResult {
    RequestResult {
        v: PROTOCOL_VERSION,
        request_id: request.request_id.clone(),
        outcome,
    }
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_session_over_postgres_writes_what_its_hello_reported_and_marks_the_instance_present() {
    in_fresh_schema(|db| async move {
        let (served, _) = Served::paired(&db).await;
        let as_paired = served.row().await;
        assert_eq!(as_paired.install_id, None);
        assert_eq!(as_paired.connection_id, None);

        let said = hello("install-a", &[Scope::Gates]);
        let (peer, connection, session) = served.greeted(&said, 0).await;

        let present = served.row().await;
        assert_eq!(present.id, served.instance);
        assert_eq!(present.install_id.as_deref(), Some("install-a"));
        assert_eq!(
            [&present.version, &present.os, &present.arch],
            ["1.5.0", "macos", "aarch64"]
        );
        assert_eq!(present.granted_scopes, ["gates"]);
        assert_eq!(present.name, "laptop");
        assert_eq!(present.connection_id, Some(connection.as_uuid()));
        assert_eq!(present.connected_at, Some(at(NOW)));
        assert_eq!(present.last_seen, Some(at(NOW)));

        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_hello_through_a_store_of_no_paired_instance_ends_the_session_and_inserts_nothing() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let served = Served::over(&db, user, InstanceId::generate());

        let (mut peer, _, session) = served.connect();
        peer.send(&hello("install-a", &[Scope::Read]));
        peer.expect_close(STORE_FAILED).await;
        assert_eq!(ended(session).await, Ended::StoreFailed);
        peer.finish().await;

        assert!(list_instances(user, &db).await.unwrap().is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_resent_event_leaves_one_mirror_row_and_every_ack_names_the_cursor_the_row_holds() {
    in_fresh_schema(|db| async move {
        let (served, _) = Served::paired(&db).await;
        let said = hello("install-a", &[Scope::Read]);
        let (mut peer, _, session) = served.greeted(&said, 0).await;

        let first = event("f-1", 5, RunEventKind::StepStarted);
        peer.send(&first);
        peer.expect_ack(5).await;
        assert_eq!(served.row().await.event_cursor, 5);

        let resent_differently = event("f-1", 5, RunEventKind::StepFailed);
        peer.send(&resent_differently);
        peer.expect_ack(5).await;
        assert_eq!(served.row().await.event_cursor, 5);

        let from_below = event("f-1", 3, RunEventKind::FeatureStarted);
        peer.send(&from_below);
        peer.expect_ack(5).await;
        assert_eq!(served.row().await.event_cursor, 5);

        let of_another_feature = event("f-2", 9, RunEventKind::FeatureStarted);
        peer.send(&of_another_feature);
        peer.expect_ack(9).await;
        assert_eq!(served.row().await.event_cursor, 9);

        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);
        assert_eq!(
            mirror_of(&db, served.user, served.instance).await,
            [
                ("f-1".to_owned(), 3, from_below),
                ("f-1".to_owned(), 5, first),
                ("f-2".to_owned(), 9, of_another_feature),
            ]
        );

        let (peer, _, session) = served.greeted(&said, 9).await;
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);
        assert_eq!(served.row().await.event_cursor, 9);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_request_enqueued_while_disconnected_is_delivered_on_the_next_connection_and_verifies() {
    in_fresh_schema(|db| async move {
        let (served, _) = Served::paired(&db).await;
        let keys = MemoryKeys::with_keys([SIGNING_KEY]);

        let first = served.enqueue(&keys, cancel("f-1")).await;
        let second = served.enqueue(&keys, cancel("f-2")).await;
        let waiting = served.requests().await;
        assert_eq!(
            waiting.iter().map(|row| &row.id).collect::<Vec<_>>(),
            [&first.id.to_vec(), &second.id.to_vec()]
        );
        for row in &waiting {
            assert_eq!(
                (row.status.as_str(), row.deliveries, row.last_sent_at),
                ("queued", 0, None)
            );
            assert_eq!(row.kind, "cancel_feature");
            assert_eq!(row.expires_at, at(first.expires_at));
        }

        let said = hello("install-a", &[Scope::Spend]);
        let (mut peer, _, session) = served.greeted(&said, 0).await;
        let delivered = [peer.expect_request().await, peer.expect_request().await];
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);

        for (envelope, queued) in delivered.iter().zip([first, second]) {
            assert_eq!(envelope.request_id, URL_SAFE_NO_PAD.encode(queued.id));
            assert_eq!(envelope.instance_id, served.instance.0.to_string());
            assert_eq!(envelope.expires_at, queued.expires_at);
            assert!(verifies(&keys, envelope).await);

            let mut rewritten = envelope.clone();
            rewritten.expires_at += 1;
            assert!(!verifies(&keys, &rewritten).await);
            let another_hub = MemoryKeys::with_keys([SIGNING_KEY]);
            assert!(!verifies(&another_hub, envelope).await);
        }
        assert_eq!(delivered[0].payload, cancel("f-1"));
        assert_eq!(delivered[1].payload, cancel("f-2"));

        let unanswered = list_queued_requests(served.user, &db, served.instance)
            .await
            .unwrap();
        let held: Vec<_> = unanswered
            .iter()
            .map(|queued| (&queued.envelope, queued.deliveries))
            .collect();
        let readable = delivered.map(|envelope| QueuedEnvelope::Readable(Box::new(envelope)));
        assert_eq!(held, [(&readable[0], 1), (&readable[1], 1)]);
        for row in served.requests().await {
            assert_eq!(row.last_sent_at, Some(at(NOW)));
        }
    })
    .await;
}

/// What a desktop reports after its database was lost under a keyring that
/// survived. Whether the cursor should be kept is undecided; that it is kept
/// until someone decides is what this holds.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_hello_under_another_install_id_is_written_and_the_cursor_is_kept() {
    in_fresh_schema(|db| async move {
        let (served, _) = Served::paired(&db).await;
        let (mut peer, _, session) = served.greeted(&hello("install-a", &[Scope::Read]), 0).await;
        peer.send(&event("f-1", 41, RunEventKind::StepStarted));
        peer.expect_ack(41).await;
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);

        let (mut peer, _, session) = served
            .greeted(&hello("install-b", &[Scope::Read]), 41)
            .await;
        let reinstalled = served.row().await;
        assert_eq!(reinstalled.install_id.as_deref(), Some("install-b"));
        assert_eq!(reinstalled.event_cursor, 41);

        let restarted = event("f-2", 1, RunEventKind::FeatureStarted);
        peer.send(&restarted);
        peer.expect_ack(41).await;
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);
        assert_eq!(served.row().await.event_cursor, 41);
        let mirrored = mirror_of(&db, served.user, served.instance).await;
        assert_eq!(mirrored.len(), 2);
        assert_eq!(mirrored[1], ("f-2".to_owned(), 1, restarted));
    })
    .await;
}

/// The first request expires at `NOW + 600`, spelled here and not read from
/// the constant, and the session is opened at exactly that second.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_request_overdue_when_its_instance_connects_is_marked_expired_and_never_sent() {
    in_fresh_schema(|db| async move {
        let (served, _) = Served::paired(&db).await;
        let keys = MemoryKeys::with_keys([SIGNING_KEY]);
        let overdue = served.enqueue(&keys, cancel("f-1")).await;
        assert_eq!(overdue.expires_at, NOW + 600);
        served.clock_reads(NOW + 600);
        let in_time = served.enqueue(&keys, cancel("f-2")).await;

        let said = hello("install-a", &[Scope::Spend]);
        let (mut peer, _, session) = served.greeted(&said, 0).await;
        let delivered = peer.expect_request().await;
        assert_eq!(delivered.request_id, URL_SAFE_NO_PAD.encode(in_time.id));
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);

        let rows: Vec<_> = served
            .requests()
            .await
            .into_iter()
            .map(|row| (row.id, row.status, row.deliveries, row.last_sent_at))
            .collect();
        assert_eq!(
            rows,
            [
                (overdue.id.to_vec(), "expired".to_owned(), 0, None),
                (
                    in_time.id.to_vec(),
                    "queued".to_owned(),
                    1,
                    Some(at(NOW + 600))
                ),
            ]
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_snapshot_sent_over_the_socket_replaces_the_one_row_the_instance_has() {
    in_fresh_schema(|db| async move {
        let (served, _) = Served::paired(&db).await;
        let said = hello("install-a", &[Scope::Read]);
        let (peer, _, session) = served.greeted(&said, 0).await;

        peer.send(&snapshot(&["demeteo", "notes"]));
        served.clock_reads(NOW + 45);
        let latest = snapshot(&["demeteo"]);
        peer.send(&latest);
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);

        assert_eq!(
            snapshots_of(&db, served.user, served.instance).await,
            [("full".to_owned(), latest, at(NOW + 45))]
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_refusal_sent_over_the_socket_is_stored_with_its_reason() {
    in_fresh_schema(|db| async move {
        let (served, _) = Served::paired(&db).await;
        let keys = MemoryKeys::with_keys([SIGNING_KEY]);
        served.enqueue(&keys, cancel("f-1")).await;
        served.enqueue(&keys, cancel("f-2")).await;
        served.enqueue(&keys, cancel("f-3")).await;

        let said = hello("install-a", &[Scope::Spend]);
        let (mut peer, _, session) = served.greeted(&said, 0).await;
        let over_ceiling = peer.expect_request().await;
        let done = peer.expect_request().await;
        let unknown = peer.expect_request().await;

        served.clock_reads(NOW + 30);
        let refused = |reason| RequestOutcome::Refused { reason };
        peer.send(&result(
            &over_ceiling,
            refused(RefusalReason::OverRunCeiling),
        ));
        peer.send(&result(&done, RequestOutcome::Accepted));
        peer.send(&result(&unknown, refused(RefusalReason::UnknownFeature)));
        peer.send(&result(&done, refused(RefusalReason::Replayed)));
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);

        let answers: Vec<_> = served
            .requests()
            .await
            .into_iter()
            .map(|row| (row.status, row.outcome, row.refusal_reason, row.answered_at))
            .collect();
        let answered = |outcome: &str, reason: Option<&str>| {
            (
                "answered".to_owned(),
                Some(outcome.to_owned()),
                reason.map(str::to_owned),
                Some(at(NOW + 30)),
            )
        };
        assert_eq!(
            answers,
            [
                answered("refused", Some("over_run_ceiling")),
                answered("accepted", None),
                answered("refused", Some("unknown_feature")),
            ]
        );
        assert!(list_queued_requests(served.user, &db, served.instance)
            .await
            .unwrap()
            .is_empty());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_disconnect_clears_presence_and_sets_last_seen_to_the_time_it_happened() {
    in_fresh_schema(|db| async move {
        let (served, _) = Served::paired(&db).await;
        let said = hello("install-a", &[Scope::Read]);
        let (peer, connection, session) = served.greeted(&said, 0).await;
        let present = served.row().await;
        assert_eq!(present.connection_id, Some(connection.as_uuid()));
        assert_eq!(present.last_seen, Some(at(NOW)));

        served.clock_reads(NOW + 120);
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);

        let gone = served.row().await;
        assert_eq!(gone.connection_id, None);
        assert_eq!(gone.connected_at, None);
        assert_eq!(gone.last_seen, Some(at(NOW + 120)));
        assert_eq!(gone.install_id.as_deref(), Some("install-a"));
    })
    .await;
}

const SOCKET_TABLES: [&str; 4] = ["instances", "run_mirror", "snapshots", "instance_requests"];

/// One stored value: a `BYTEA` as its bytes and anything else as the text
/// Postgres writes it as.
struct Cell {
    table: &'static str,
    column: String,
    bytes: Vec<u8>,
}

/// Every non-null value of every row of `table`, read as `cells_of` in
/// `crates/demeteo-hub/tests/pairing.rs` reads them and for the reason given
/// there.
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

/// A session is never handed a token, so this passes for as long as nothing
/// starts handing it one: the search is first watched find a value of each
/// table, and a `BYTEA` among them, and only then asked for the tokens. Each
/// is looked for as the text the desktop was sent and as the bytes that text
/// encodes.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn no_socket_table_holds_a_token_the_instance_was_paired_with() {
    in_fresh_schema(|db| async move {
        let (served, tokens) = Served::paired(&db).await;
        let keys = MemoryKeys::with_keys([SIGNING_KEY]);
        let queued = served.enqueue(&keys, cancel("f-1")).await;

        let said = hello("install-a", &[Scope::Read, Scope::Spend]);
        let (mut peer, _, session) = served.greeted(&said, 0).await;
        let request = peer.expect_request().await;
        peer.send(&event("f-9", 1, RunEventKind::StepStarted));
        peer.expect_ack(1).await;
        peer.send(&snapshot(&["demeteo"]));
        let refused = RequestOutcome::Refused {
            reason: RefusalReason::ScopeOff,
        };
        peer.send(&result(&request, refused));
        peer.finish().await;
        assert_eq!(ended(session).await, Ended::PeerGone);

        let mut cells = Vec::new();
        for table in SOCKET_TABLES {
            cells.extend(cells_of(&db, table).await);
        }
        let rows_of = |table: &str| {
            let owners = cells
                .iter()
                .filter(|cell| cell.table == table && cell.column == "user_id");
            owners.count()
        };
        assert_eq!(SOCKET_TABLES.map(rows_of), [1, 1, 1, 1]);
        assert_eq!(
            columns_holding(&cells, b"install-a"),
            [("instances", "install_id")]
        );
        assert_eq!(
            columns_holding(&cells, b"s-implement"),
            [("run_mirror", "event")]
        );
        assert_eq!(
            columns_holding(&cells, b"p-demeteo"),
            [("snapshots", "body")]
        );
        assert_eq!(
            columns_holding(&cells, request.request_id.as_bytes()),
            [("instance_requests", "envelope")]
        );
        assert_eq!(
            columns_holding(&cells, &queued.id),
            [("instance_requests", "id")]
        );
        assert_eq!(
            columns_holding(&cells, b"scope_off"),
            [("instance_requests", "refusal_reason")]
        );

        let secrets = [
            ("access token", tokens.access.text()),
            ("refresh token", tokens.refresh.text()),
        ];
        for (name, secret) in secrets {
            assert_eq!(
                columns_holding(&cells, secret.as_bytes()),
                [],
                "the {name}, as text"
            );
            let raw = URL_SAFE_NO_PAD.decode(&secret).unwrap();
            assert_eq!(columns_holding(&cells, &raw), [], "the {name}, as bytes");
        }
    })
    .await;
}
