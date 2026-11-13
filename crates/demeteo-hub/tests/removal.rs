//! Removing an instance from the Hub, against a real Postgres: what
//! `remove_instance` writes in its one transaction, and what pairing the same
//! key does afterwards. Needs `DEMETEO_HUB_TEST_DATABASE_URL`, as
//! `tests/tenancy.rs` does.

mod support;

use demeteo_hub::pairing::grant::{refresh_change, FreshPair, RefreshRefusal, TokenSecret};
use demeteo_hub::pairing::jwk::DeviceJwk;
use demeteo_hub::pairing::testing::SoftDeviceKey;
use demeteo_hub::store::device_keys::{instance_of_key, NewDeviceKey};
use demeteo_hub::store::instances::{
    find_instance, list_instances, pair_instance, record_hello, remove_instance, InstanceKind,
    NewInstanceLabels, NewPairing,
};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::requests::{find_request, insert_request, NewRequest, RequestStatus};
use demeteo_hub::store::run_mirror::{mirror_event, mirror_of, NewEvent};
use demeteo_hub::store::tokens::{find_token_by_hash, rotate_family};
use demeteo_hub::store::{Db, InstanceId, UserId};
use demeteo_hub_protocol::{
    CancelFeature, Hello, RequestPayload, Scope, SignedRequest, PROTOCOL_VERSION,
};
use support::in_fresh_schema;
use time::OffsetDateTime;
use uuid::Uuid;

fn unix_now() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).unwrap()
}

fn device_key(key: &SoftDeviceKey) -> NewDeviceKey {
    let jwk = DeviceJwk::parse(&key.jwk()).unwrap();
    NewDeviceKey {
        public_jwk: jwk.canonical_json(),
        jkt: jwk.thumbprint(),
    }
}

fn new_pairing(key: &NewDeviceKey, name: &str) -> (NewPairing, FreshPair) {
    let family_id = Uuid::new_v4();
    let pair = FreshPair::generate().unwrap();
    let rows = pair.rows(family_id, &key.jkt, unix_now()).unwrap();
    let new = NewPairing {
        kind: InstanceKind::Desktop,
        instance: NewInstanceLabels {
            name: name.to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            version: "1.2.0".to_owned(),
        },
        device_key: key.clone(),
        scopes: vec![Scope::Read],
        family_id,
        access: rows.access,
        refresh: rows.refresh,
    };
    (new, pair)
}

async fn is_live(db: &Db, token: &TokenSecret) -> bool {
    let found = find_token_by_hash(db, &token.hash())
        .await
        .unwrap()
        .unwrap();
    found.retired_at.is_none()
}

/// The tenant's `device_keys` rows as `(instance_id, jkt, revoked)`.
async fn device_keys_of(db: &Db, user: UserId) -> Vec<(InstanceId, String, bool)> {
    sqlx::query_as(
        "SELECT instance_id, jkt, revoked_at IS NOT NULL \
         FROM device_keys WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .unwrap()
}

fn queued_cancel(id: [u8; 16], instance: InstanceId, expires_at: u64) -> NewRequest {
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

fn hello() -> Hello {
    Hello {
        v: PROTOCOL_VERSION,
        install_id: "install".to_owned(),
        app_version: "1.5.0".to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        scopes: vec![Scope::Read],
    }
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn removing_an_instance_revokes_its_key_retires_its_tokens_expires_its_queue_and_keeps_its_history(
) {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let kept_key = device_key(&SoftDeviceKey::new());
        let (pairing, pair) = new_pairing(&key, "laptop");
        let (kept_pairing, kept_pair) = new_pairing(&kept_key, "desktop");
        let instance = pair_instance(user, &db, &pairing)
            .await
            .unwrap()
            .instance_id;
        let kept = pair_instance(user, &db, &kept_pairing)
            .await
            .unwrap()
            .instance_id;
        let now = unix_now();
        let request = [3; 16];
        insert_request(
            user,
            &db,
            instance,
            &queued_cancel(request, instance, now + 600),
        )
        .await
        .unwrap();
        let event = NewEvent {
            feature_id: "f-1".to_owned(),
            event_offset: 1,
            event: "{}".to_owned(),
        };
        mirror_event(user, &db, instance, &event, |_, offset| offset)
            .await
            .unwrap();
        record_hello(user, &db, instance, &hello(), Uuid::new_v4(), now)
            .await
            .unwrap();

        assert!(remove_instance(user, &db, instance, now).await.unwrap());

        let removed = find_instance(user, &db, instance).await.unwrap().unwrap();
        assert!(removed.is_removed());
        assert_eq!(removed.connection_id, None);
        assert_eq!(list_instances(user, &db).await.unwrap().len(), 2);
        assert_eq!(instance_of_key(user, &db, &key.jkt).await.unwrap(), None);
        assert_eq!(
            device_keys_of(&db, user).await,
            [
                (instance, key.jkt.clone(), true),
                (kept, kept_key.jkt.clone(), false)
            ]
        );
        assert!(!is_live(&db, &pair.access).await);
        assert!(!is_live(&db, &pair.refresh).await);
        let record = find_request(user, &db, instance, &request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.status, RequestStatus::Expired);
        assert_eq!(mirror_of(user, &db, instance, None).await.unwrap().len(), 1);

        let untouched = find_instance(user, &db, kept).await.unwrap().unwrap();
        assert!(!untouched.is_removed());
        assert!(is_live(&db, &kept_pair.refresh).await);
        assert_eq!(
            instance_of_key(user, &db, &kept_key.jkt).await.unwrap(),
            Some(kept)
        );

        assert!(!remove_instance(user, &db, instance, now).await.unwrap());
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn removing_another_users_instance_writes_nothing() {
    in_fresh_schema(|db| async move {
        let owner = create_user(&db).await.unwrap();
        let stranger = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (pairing, pair) = new_pairing(&key, "laptop");
        let instance = pair_instance(owner, &db, &pairing)
            .await
            .unwrap()
            .instance_id;

        assert!(!remove_instance(stranger, &db, instance, unix_now())
            .await
            .unwrap());

        let row = find_instance(owner, &db, instance).await.unwrap().unwrap();
        assert!(!row.is_removed());
        assert!(is_live(&db, &pair.refresh).await);
        assert_eq!(
            instance_of_key(owner, &db, &key.jkt).await.unwrap(),
            Some(instance)
        );
    })
    .await;
}

/// The refresh token a removal retired is presented again, as an instance
/// that missed its own removal would: it ends its family and nothing is
/// issued.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_refresh_after_removal_issues_nothing() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (pairing, pair) = new_pairing(&key, "laptop");
        let instance = pair_instance(user, &db, &pairing)
            .await
            .unwrap()
            .instance_id;
        assert!(remove_instance(user, &db, instance, unix_now())
            .await
            .unwrap());

        let fresh = FreshPair::generate().unwrap();
        let hash = pair.refresh.hash();
        let written = rotate_family(&db, &hash, |family| {
            let decision = refresh_change(family, &hash, &key.jkt, unix_now(), &fresh).unwrap();
            (decision.change, decision.verdict)
        })
        .await
        .unwrap();

        assert!(matches!(written.answer, Err(RefreshRefusal::Reused)));
        assert!(written.inserted.is_empty());
        let issued = find_token_by_hash(&db, &fresh.refresh.hash())
            .await
            .unwrap();
        assert_eq!(issued, None);
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn pairing_a_key_whose_instance_was_removed_makes_a_new_instance_and_leaves_the_old_removed()
{
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (first, _) = new_pairing(&key, "laptop");
        let removed = pair_instance(user, &db, &first).await.unwrap().instance_id;
        assert!(remove_instance(user, &db, removed, unix_now())
            .await
            .unwrap());

        let (again, again_pair) = new_pairing(&key, "laptop again");
        let paired = pair_instance(user, &db, &again).await.unwrap().instance_id;

        assert_ne!(paired, removed);
        let old = find_instance(user, &db, removed).await.unwrap().unwrap();
        assert!(old.is_removed());
        let new = find_instance(user, &db, paired).await.unwrap().unwrap();
        assert!(!new.is_removed());
        assert_eq!(new.name, "laptop again");
        assert_eq!(
            device_keys_of(&db, user).await,
            [(paired, key.jkt.clone(), false)]
        );
        assert_eq!(
            instance_of_key(user, &db, &key.jkt).await.unwrap(),
            Some(paired)
        );
        assert!(is_live(&db, &again_pair.refresh).await);
    })
    .await;
}

/// A socket admitted just before the removal committed sends its `hello`
/// after: the removed row is not marked present again.
#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_hello_for_a_removed_instance_writes_nothing() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let key = device_key(&SoftDeviceKey::new());
        let (pairing, _) = new_pairing(&key, "laptop");
        let instance = pair_instance(user, &db, &pairing)
            .await
            .unwrap()
            .instance_id;
        let now = unix_now();
        assert!(remove_instance(user, &db, instance, now).await.unwrap());

        let greeted = record_hello(user, &db, instance, &hello(), Uuid::new_v4(), now)
            .await
            .unwrap();

        assert_eq!(greeted, None);
        let row = find_instance(user, &db, instance).await.unwrap().unwrap();
        assert_eq!(row.connection_id, None);
    })
    .await;
}
