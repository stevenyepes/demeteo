// Tests extracted from `crates/demeteo-runner/src/rpc/hub.rs` (mirrored-tests convention). `super` = that module.
//
// Every pairing here is over the runner's real sealed store and a database
// of its own; only the Hub is a double, and it answers only what it was
// scripted with.

use super::dispatch_hub;
use crate::hub::HubControl;
use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::hub::sealed_store::SealedSecretStore;
use demeteo_core::adapters::hub::socket::Sleeper;
use demeteo_core::adapters::hub::testing::ScriptedDeviceEndpoint;
use demeteo_core::adapters::hub::HubPairing;
use demeteo_core::domain::hub_pairing::device::DeviceGrantEnd;
use demeteo_core::domain::hub_pairing::sealed::SEALED_BACKEND;
use demeteo_core::domain::hub_pairing::{HubPairingError, PairingAnswer};
use demeteo_core::ports::hub::{HubSecretStore, HubTokenEndpoint};
use demeteo_core::test_dir::TestDir;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const HUB_URL: &str = "https://hub.example.com";
const DEVICE_CODE: &str = "dc-0123456789abcdefghijklmnopqrstuvwxyzABCDEFG";
const USER_CODE: &str = "BCDF-GHJK";
const NO_KEY: &str = "CREDENTIALS_DIRECTORY is not set: the unit loads no hub-store-key credential";

struct NoRedemption;

#[async_trait::async_trait]
impl HubTokenEndpoint for NoRedemption {
    async fn exchange(
        &self,
        _: &str,
        _: String,
        _: String,
    ) -> Result<PairingAnswer, HubPairingError> {
        Err(HubPairingError::Transport("unscripted exchange".into()))
    }
}

/// Returns at once, so a scripted poll sequence runs to its end.
struct InstantSleeper;

#[async_trait::async_trait]
impl Sleeper for InstantSleeper {
    async fn sleep(&self, _: Duration) {
        tokio::task::yield_now().await;
    }
}

/// Never returns, so a pairing stays pending until cancelled.
struct ParkedSleeper;

#[async_trait::async_trait]
impl Sleeper for ParkedSleeper {
    async fn sleep(&self, _: Duration) {
        std::future::pending::<()>().await;
    }
}

struct Rig {
    _dir: TestDir,
    device: Arc<ScriptedDeviceEndpoint>,
    hub: HubControl,
}

impl Rig {
    fn new(tag: &str, key: Result<[u8; 32], &'static str>, sleeper: Arc<dyn Sleeper>) -> Self {
        let dir = TestDir::new(tag);
        let conn = demeteo_core::db::init_db(dir.path().to_path_buf()).unwrap();
        let db = Arc::new(SqliteAdapter::new(conn).unwrap());
        let secrets: Arc<dyn HubSecretStore> =
            Arc::new(SealedSecretStore::new(db.clone(), key, SEALED_BACKEND));
        let device = Arc::new(ScriptedDeviceEndpoint::new());
        let pairing = HubPairing::new(
            db.clone(),
            secrets.clone(),
            Arc::new(NoRedemption),
            db.clone(),
        )
        .with_device_endpoint(device.clone())
        .with_device_sleeper(sleeper);
        Self {
            _dir: dir,
            device,
            hub: HubControl {
                pairing: Some(Arc::new(pairing)),
                secrets,
                policy: db.clone(),
                policy_writer: db,
            },
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        dispatch_hub(&self.hub, method, &params)
            .await
            .unwrap_or_else(|| panic!("{method} is a hub method"))
    }

    /// The pairing state once its poll task has ended, or a failed test.
    async fn settled(&self) -> Value {
        for _ in 0..5_000 {
            let state = self.call("hub_pair_state", json!({})).await.unwrap();
            if state["state"] != "pending" {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("the device pairing never settled");
    }
}

#[tokio::test]
async fn without_the_sealed_key_pairing_is_refused_with_how_to_fix_it_before_any_hub_call() {
    let rig = Rig::new("runner_hub_no_key", Err(NO_KEY), Arc::new(InstantSleeper));
    rig.device.authorize(HUB_URL, DEVICE_CODE, USER_CODE, 5);

    let error = rig
        .call("hub_pair_begin", json!({ "hub_url": HUB_URL }))
        .await
        .expect_err("a runner without its key does not pair");

    assert!(error.starts_with("cannot pair: "), "{error}");
    assert!(error.contains(NO_KEY), "{error}");
    assert!(error.contains("demeteo-runner hub seal"), "{error}");
    assert!(rig.device.authorize_calls().is_empty(), "no Hub call");
}

#[tokio::test]
async fn status_without_the_key_says_why_and_how() {
    let rig = Rig::new(
        "runner_hub_status_no_key",
        Err(NO_KEY),
        Arc::new(InstantSleeper),
    );
    let view = rig.call("hub_status", json!({})).await.unwrap();
    assert_eq!(view["status"]["state"], "unpaired");
    assert_eq!(view["status"]["keyring_backend"], SEALED_BACKEND);
    assert_eq!(view["status"]["keyring_available"], false);
    let why = view["store_unavailable"].as_str().expect("a reason");
    assert!(why.contains(NO_KEY) && why.contains("hub seal"), "{why}");
    assert_eq!(view["connection"]["state"], "not_running");
}

#[tokio::test]
async fn with_the_key_a_pairing_answers_its_ticket_and_stays_pending_until_cancelled() {
    let rig = Rig::new("runner_hub_pending", Ok([7; 32]), Arc::new(ParkedSleeper));
    rig.device.authorize(HUB_URL, DEVICE_CODE, USER_CODE, 5);

    let ticket = rig
        .call(
            "hub_pair_begin",
            json!({ "hub_url": HUB_URL, "client_id": "laptop-1" }),
        )
        .await
        .expect("a ticket");
    assert_eq!(ticket["user_code"], USER_CODE);
    assert_eq!(ticket["verification_uri"], format!("{HUB_URL}/#/device"));
    assert!(ticket["fingerprint"]
        .as_str()
        .is_some_and(|f| f.starts_with("SHA256:")));

    let state = rig.call("hub_pair_state", json!({})).await.unwrap();
    assert_eq!(state["state"], "pending");
    assert_eq!(state["ticket"], ticket);
    let view = rig.call("hub_status", json!({})).await.unwrap();
    assert_eq!(view["store_unavailable"], Value::Null);

    let cancelled = rig.call("hub_pair_cancel", json!({})).await.unwrap();
    assert_eq!(cancelled, json!({ "state": "idle" }));
    assert_eq!(rig.device.unscripted(), 0);
}

#[tokio::test]
async fn a_denied_pairing_reads_failed_with_the_hubs_reason() {
    let rig = Rig::new("runner_hub_denied", Ok([7; 32]), Arc::new(InstantSleeper));
    rig.device.authorize(HUB_URL, DEVICE_CODE, USER_CODE, 5);
    rig.device.poll_ends(DeviceGrantEnd::Pending);
    rig.device.poll_ends(DeviceGrantEnd::Denied);

    rig.call("hub_pair_begin", json!({ "hub_url": HUB_URL }))
        .await
        .expect("a ticket");

    assert_eq!(
        rig.settled().await,
        json!({ "state": "failed", "error": HubPairingError::Denied.to_string() })
    );
    let view = rig.call("hub_status", json!({})).await.unwrap();
    assert_eq!(view["status"]["state"], "unpaired");
    assert_eq!(rig.device.unscripted(), 0);
}

#[tokio::test]
async fn the_policy_round_trips_and_answers_what_is_stored() {
    let rig = Rig::new("runner_hub_policy", Err(NO_KEY), Arc::new(InstantSleeper));
    let stored = rig.call("hub_policy_get", json!({})).await.unwrap();
    assert_eq!(stored["read_enabled"], true);
    assert_eq!(stored["spend_enabled"], false);

    let wanted = json!({
        "read_enabled": false,
        "spend_enabled": true,
        "gates_enabled": true,
        "run_ceiling_cents": 250,
        "daily_ceiling_cents": null,
    });
    let saved = rig
        .call(
            "hub_policy_set",
            json!({ "policy": wanted, "client_id": "laptop-1" }),
        )
        .await
        .unwrap();
    assert_eq!(saved, wanted);
    assert_eq!(rig.call("hub_policy_get", json!({})).await.unwrap(), wanted);
}

#[tokio::test]
async fn malformed_params_are_refused_and_other_methods_fall_through() {
    let rig = Rig::new("runner_hub_params", Ok([7; 32]), Arc::new(InstantSleeper));
    for (method, params) in [
        ("hub_pair_begin", json!({})),
        (
            "hub_policy_set",
            json!({ "policy": { "spend_enabled": "yes" } }),
        ),
    ] {
        let error = rig.call(method, params).await.expect_err(method);
        assert!(error.starts_with("invalid params"), "{method}: {error}");
    }
    assert!(dispatch_hub(&rig.hub, "get_status", &json!({}))
        .await
        .is_none());
    assert!(rig.device.authorize_calls().is_empty());
}

#[tokio::test]
async fn disconnect_on_an_unpaired_runner_reads_unpaired() {
    let rig = Rig::new(
        "runner_hub_disconnect",
        Ok([7; 32]),
        Arc::new(InstantSleeper),
    );
    let status = rig.call("hub_disconnect", json!({})).await.unwrap();
    assert_eq!(status["state"], "unpaired");
}
