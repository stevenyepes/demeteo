//! `POST /unpair` over the real router and real rows: an instance's own
//! access token and proof remove that instance and no other, and once
//! removed it is nobody. What the removal writes is held to its rows in
//! `tests/removal.rs`.
//!
//! The tests are ignored for the reason `crates/demeteo-hub/tests/pairing.rs`
//! gives in its header, and an ignored run shows nothing of them.

mod support;

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::attachments::AttachmentSettings;
use demeteo_hub::auth::challenge::base64url_encode;
use demeteo_hub::pairing::grant::FreshPair;
use demeteo_hub::pairing::jwk::DeviceJwk;
use demeteo_hub::pairing::testing::{claims, SoftDeviceKey};
use demeteo_hub::routes::AppState;
use demeteo_hub::store::device_keys::NewDeviceKey;
use demeteo_hub::store::instances::{pair_instance, InstanceKind, NewInstanceLabels, NewPairing};
use demeteo_hub::store::pool::create_user;
use demeteo_hub::store::{Db, InstanceId, UserId};
use demeteo_hub_protocol::Scope;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use support::http::{post_with_dpop, rp, serve_state, unix_now};
use support::in_fresh_schema;
use uuid::Uuid;

fn state_over(db: Db) -> AppState {
    AppState::new(
        db,
        rp(),
        Arc::new(MemoryKeys::with_keys(["signing", "transit"])),
        "signing".to_owned(),
        "transit".to_owned(),
        AttachmentSettings::default(),
    )
    .unwrap()
}

struct Paired {
    instance: InstanceId,
    key: SoftDeviceKey,
    access_token: String,
}

impl Paired {
    async fn new(db: &Db, user: UserId) -> Self {
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
            kind: InstanceKind::Runner,
            instance: NewInstanceLabels {
                name: "runner".to_owned(),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                version: "1.2.0".to_owned(),
            },
            device_key,
            scopes: vec![Scope::Read],
            family_id,
            access: rows.access,
            refresh: rows.refresh,
        };
        let instance = pair_instance(user, db, &new).await.unwrap().instance_id;
        Self {
            instance,
            key,
            access_token: pair.access.text(),
        }
    }

    async fn unpair_at(&self, base: &str, path: &str) -> u16 {
        post_with_dpop(base, path, &self.key, &self.access_token, &json!({}))
            .await
            .0
    }
}

async fn removed(db: &Db, instance: InstanceId) -> bool {
    sqlx::query_scalar("SELECT removed_at IS NOT NULL FROM instances WHERE id = $1")
        .bind(instance.0)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn live_keys(db: &Db, instance: InstanceId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM device_keys WHERE instance_id = $1 AND revoked_at IS NULL",
    )
    .bind(instance.0)
    .fetch_one(db)
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn an_instance_unpairs_itself_and_no_other_and_is_then_nobody() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let leaving = Paired::new(&db, user).await;
        let staying = Paired::new(&db, user).await;
        let base = serve_state(state_over(db.clone())).await;

        assert_eq!(leaving.unpair_at(&base, "/unpair").await, 204);

        assert!(removed(&db, leaving.instance).await);
        assert_eq!(live_keys(&db, leaving.instance).await, 0);
        assert!(!removed(&db, staying.instance).await);
        assert_eq!(live_keys(&db, staying.instance).await, 1);
        assert_eq!(
            leaving.unpair_at(&base, "/unpair").await,
            401,
            "a removed instance is refused like any revoked key"
        );
    })
    .await;
}

#[tokio::test]
#[ignore = "needs Postgres: see deploy/hub/README.md; set DEMETEO_HUB_TEST_DATABASE_URL"]
async fn a_proof_made_for_another_route_unpairs_nothing() {
    in_fresh_schema(|db| async move {
        let user = create_user(&db).await.unwrap();
        let paired = Paired::new(&db, user).await;
        let base = serve_state(state_over(db.clone())).await;

        let htu = format!("{}/git-token", rp().origin);
        let mut claims = claims("POST", &htu, unix_now(), &Uuid::new_v4().to_string());
        claims["ath"] = json!(base64url_encode(&Sha256::digest(
            paired.access_token.as_bytes()
        )));
        let proof = paired.key.proof_with(&paired.key.header(), &claims);
        let status = reqwest::Client::new()
            .post(format!("{base}/unpair"))
            .header("authorization", format!("DPoP {}", paired.access_token))
            .header("dpop", proof)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16();

        assert_eq!(status, 401);
        assert!(!removed(&db, paired.instance).await);
    })
    .await;
}
