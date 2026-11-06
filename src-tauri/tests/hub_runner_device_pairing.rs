#![cfg(target_os = "linux")]
//! The runner's device-code pairing (`docs/HUB.md` §4, *Pairing a runner by
//! device code*), with `demeteo-core`'s client on one side and the Hub's own
//! code on the other, as `hub_desktop_pairing.rs` does for the desktop's
//! grant. Each half was built to the spec without the other; these are the
//! only tests in which the bytes one sends are the bytes the other reads.
//!
//! `/device_authorization` goes over loopback HTTP into the Hub's real router.
//! The polls cannot: the approved one ends in an instance the router would
//! write to Postgres, and the router reads the wall clock, which a poll loop
//! driven by an instant sleeper never advances past `slow_down`. So each poll
//! is read by the Hub's own `/token` grant dispatch and handed to `polled`,
//! at a Hub clock that moves only when the client's sleeper is woken.

use async_trait::async_trait;
use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::hub::socket::Sleeper;
use demeteo_core::adapters::hub::testing::MemorySecretStore;
use demeteo_core::adapters::hub::token_client::ReqwestTokenEndpoint;
use demeteo_core::adapters::hub::HubPairing;
use demeteo_core::domain::hub_pairing::device::{
    device_grant_refusal, DeviceAuthorization, DeviceGrantEnd, DevicePairing, DeviceTicket,
};
use demeteo_core::domain::hub_pairing::keyring_names::hub_keyring_account;
use demeteo_core::domain::hub_pairing::{
    HubPairingError, HubPairingState, HubSecret, PairingAnswer,
};
use demeteo_core::ports::hub::{HubDeviceEndpoint, HubPinRepository, HubSecretStore};
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::testing::{answered, lazy_state_with_keys, serve, SoftAuthenticator};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::handlers::{
    approved_device, denied_device, device_challenge, looked_up_device, polled, DeviceApproval,
    DeviceChallengeRequest, DeviceLookup, Grant,
};
use demeteo_hub::protocol::Scope;
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::store::instances::{InstanceKind, NewPairing, Paired};
use demeteo_hub::store::passkeys::Passkey;
use demeteo_hub::store::{InstanceId, UserId};
use reqwest::Url;
use serde_json::{json, Value};
use std::future::ready;
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

const SIGNING_KEY: &str = "demeteo-hub-signing";

/// How long a step the test waits on may take before it is a hang.
const PATIENCE: Duration = Duration::from_secs(10);

fn mutex<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

fn wall_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock set after 1970")
        .as_secs()
}

/// The client's sleeper and the Hub's clock in one: a sleep records what it
/// was asked for and returns only when the test wakes it, moving the Hub's
/// clock on by what the test says passed. The client's DPoP proofs are dated
/// by the wall clock, so the Hub's may run ahead of it by less than the
/// Hub's proof window, which a handful of polls stays well inside.
struct HubClock {
    ahead: AtomicU64,
    asked: Mutex<Vec<u64>>,
    wakes: tokio::sync::Mutex<UnboundedReceiver<u64>>,
}

impl HubClock {
    fn now(&self) -> u64 {
        wall_clock() + self.ahead.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Sleeper for HubClock {
    async fn sleep(&self, duration: Duration) {
        mutex(&self.asked).push(duration.as_secs());
        let Some(passed) = self.wakes.lock().await.recv().await else {
            return std::future::pending().await;
        };
        self.ahead.fetch_add(passed, Ordering::SeqCst);
    }
}

/// The Hub's two device-code endpoints as the runner's client reaches them.
/// Every poll's answer is reported, by its `error` code or `paired`, so the
/// test can step the pairing one poll at a time. The store is a recording
/// closure answering `paired`, since the Hub's own is a database that is not
/// there.
struct HubDevice {
    http: ReqwestTokenEndpoint,
    hub: AppState,
    clock: Arc<HubClock>,
    paired: Mutex<Option<Paired>>,
    handed: Mutex<Vec<(UserId, NewPairing)>>,
    answer: Mutex<Option<Value>>,
    answers: UnboundedSender<String>,
}

impl HubDevice {
    fn report(&self, answer: &str) {
        let _ = self.answers.send(answer.to_owned());
    }
}

#[async_trait]
impl HubDeviceEndpoint for HubDevice {
    async fn authorize_device(
        &self,
        url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<DeviceAuthorization, HubPairingError> {
        self.http.authorize_device(url, form_body, dpop).await
    }

    async fn poll_device(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<PairingAnswer, DeviceGrantEnd> {
        assert_eq!(token_url, format!("{}/token", self.hub.rp().origin));
        let mut form = Url::parse("http://form.invalid/").expect("a URL");
        form.set_query(Some(&form_body));
        let form: serde_json::Map<String, Value> = form
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned().into()))
            .collect();
        let grant = serde_json::from_value(form.into());
        let Ok(Grant::DeviceCode(poll)) = grant else {
            self.report("not a device-code grant");
            return Err(device_grant_refusal(
                br#"{"error":"unsupported_grant_type"}"#,
            ));
        };

        let pair = |user, new| {
            mutex(&self.handed).push((user, new));
            let paired = mutex(&self.paired).take().expect("one approved poll");
            ready(Ok(paired))
        };
        match polled(&self.hub, poll, Some(&dpop), self.clock.now(), pair).await {
            Ok(response) => {
                self.report("paired");
                let body = serde_json::to_vec(&response).expect("an answer that is JSON");
                *mutex(&self.answer) = serde_json::from_slice(&body).ok();
                serde_json::from_slice(&body).map_err(|e| {
                    DeviceGrantEnd::Refused(HubPairingError::MalformedAnswer(format!(
                        "the runner's mirror: {e}"
                    )))
                })
            }
            Err(refusal) => {
                let (_, body) = answered(refusal).await;
                self.report(body["error"].as_str().unwrap_or("no error code"));
                Err(device_grant_refusal(
                    &serde_json::to_vec(&body).expect("a refusal that is JSON"),
                ))
            }
        }
    }
}

/// A Hub served on loopback at a `localhost` origin, the one plain-HTTP
/// origin both sides accept, and a runner's pairing pointed at it.
struct Rig {
    origin: String,
    rp: RelyingParty,
    hub: AppState,
    clock: Arc<HubClock>,
    wake: UnboundedSender<u64>,
    answers: UnboundedReceiver<String>,
    device: Arc<HubDevice>,
    db: Arc<SqliteAdapter>,
    secrets: Arc<MemorySecretStore>,
    pairing: Arc<HubPairing>,
    _dir: demeteo_core::test_dir::TestDir,
}

impl Rig {
    async fn new(name: &str) -> Self {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("a free loopback port");
        let port = listener.local_addr().expect("the bound address").port();
        let origin = format!("http://localhost:{port}");
        let rp = RelyingParty {
            rp_id: "localhost".to_owned(),
            origin: origin.clone(),
        };
        let keys = Arc::new(MemoryKeys::with_keys([SIGNING_KEY]));
        let hub = lazy_state_with_keys(
            "postgres://hub@db.invalid/hub",
            rp.clone(),
            keys,
            SIGNING_KEY,
        )
        .expect("a parsable database URL");
        let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
        serve(listener, routes::router(hub.clone(), &web_dir));

        let (wake, wakes) = unbounded_channel();
        let clock = Arc::new(HubClock {
            ahead: AtomicU64::new(0),
            asked: Mutex::default(),
            wakes: tokio::sync::Mutex::new(wakes),
        });
        let (reports, answers) = unbounded_channel();
        let device = Arc::new(HubDevice {
            http: ReqwestTokenEndpoint::new().expect("an HTTP client"),
            hub: hub.clone(),
            clock: clock.clone(),
            paired: Mutex::default(),
            handed: Mutex::default(),
            answer: Mutex::default(),
            answers: reports,
        });

        let dir = demeteo_core::test_dir::TestDir::new(name);
        let conn = rusqlite::Connection::open(dir.path().join("demeteo.db")).expect("a scratch DB");
        let db = Arc::new(SqliteAdapter::new(conn).expect("a migrated DB"));
        let secrets = Arc::new(MemorySecretStore::new(
            &db.install_id().expect("an install id"),
        ));
        let token = Arc::new(ReqwestTokenEndpoint::new().expect("an HTTP client"));
        let pairing = HubPairing::new(db.clone(), secrets.clone(), token, db.clone())
            .with_device_endpoint(device.clone())
            .with_device_sleeper(clock.clone());

        Self {
            origin,
            rp,
            hub,
            clock,
            wake,
            answers,
            device,
            db,
            secrets,
            pairing: Arc::new(pairing),
            _dir: dir,
        }
    }

    async fn begin(&self) -> DeviceTicket {
        let ticket = self
            .pairing
            .begin_device_pairing(&format!("{}/", self.origin))
            .await;
        ticket.expect("the Hub starts the runner's pairing")
    }

    /// Ends the client's current sleep with `passed` seconds gone, and
    /// returns the Hub's answer to the poll that follows it.
    async fn poll_after(&mut self, passed: u64) -> String {
        self.wake.send(passed).expect("the poll task is waiting");
        tokio::time::timeout(PATIENCE, self.answers.recv())
            .await
            .expect("the client polled")
            .expect("the endpoint is alive")
    }

    /// The pairing once its poll task has stopped polling.
    async fn settled(&self) -> DevicePairing {
        let settled = async {
            loop {
                match self.pairing.device_pairing() {
                    DevicePairing::Pending(_) => tokio::task::yield_now().await,
                    ended => return ended,
                }
            }
        };
        tokio::time::timeout(PATIENCE, settled)
            .await
            .expect("the pairing settled")
    }

    fn asked(&self) -> Vec<u64> {
        mutex(&self.clock.asked).clone()
    }

    /// A tenant whose one trusted passkey will answer.
    fn tenant(&self) -> (SoftAuthenticator, Passkey) {
        let authenticator = SoftAuthenticator::new();
        let passkey = authenticator.enrolled(&self.hub, Status::Trusted);
        (authenticator, passkey)
    }

    /// What hub-web shows beside the code the runner printed.
    async fn look_up(&self, passkey: &Passkey, user_code: &str) -> Value {
        let lookup = DeviceLookup::new(user_code);
        match looked_up_device(&self.hub, passkey.user_id, lookup, self.clock.now()) {
            Ok(prompt) => serde_json::to_value(prompt).expect("a prompt that is JSON"),
            Err(refusal) => panic!("the lookup was refused: {:?}", answered(refusal).await),
        }
    }

    /// The human approves `scopes` with `passkey`, by an assertion over the
    /// challenge the Hub issued for this one pairing.
    async fn approve(
        &self,
        authenticator: &SoftAuthenticator,
        passkey: &Passkey,
        user_code: &str,
        scopes: Vec<Scope>,
    ) {
        let passkeys = std::slice::from_ref(passkey);
        let request = DeviceChallengeRequest::new(user_code, scopes);
        let challenge = device_challenge(
            &self.hub,
            passkey.user_id,
            passkey.credential_id.clone(),
            request,
            passkeys,
            self.clock.now(),
        );
        let challenge = match challenge {
            Ok(challenge) => challenge,
            Err(refusal) => panic!("no challenge: {:?}", answered(refusal).await),
        };
        let digest = challenge.challenge().expect("options carrying a challenge");
        let assertion = authenticator.assert(&self.rp, &digest);
        let approval = DeviceApproval::new(challenge.ceremony_id(), assertion);
        let approved = approved_device(
            &self.hub,
            passkey.user_id,
            &passkey.credential_id,
            approval,
            passkeys,
            self.clock.now(),
        );
        match approved {
            Ok(answer) => assert_eq!(json!(answer), json!({ "answer": "approved" })),
            Err(refusal) => panic!("the approval was refused: {:?}", answered(refusal).await),
        }
    }

    fn stored_refresh_token(&self) -> Option<String> {
        let install_id = self.db.install_id().expect("an install id");
        let account = hub_keyring_account(&install_id, HubSecret::RefreshToken);
        self.secrets.get(&account).expect("a readable store")
    }
}

#[tokio::test]
async fn a_runner_pairs_through_the_hubs_own_device_and_approval_handlers() {
    let mut rig = Rig::new("hub_runner_device_pairing").await;
    let (authenticator, passkey) = rig.tenant();
    let other = SoftAuthenticator::new().enrolled(&rig.hub, Status::Trusted);
    let instance_id = InstanceId::generate();
    *mutex(&rig.device.paired) = Some(Paired {
        instance_id,
        passkeys: vec![passkey.clone(), other],
    });

    let ticket = rig.begin().await;
    assert_eq!(ticket.verification_uri, format!("{}/#/device", rig.origin));
    assert_eq!(
        ticket.verification_uri_complete,
        format!("{}/#/device/{}", rig.origin, ticket.user_code)
    );

    let prompt = rig.look_up(&passkey, &ticket.user_code).await;
    assert_eq!(prompt["fingerprint"], json!(ticket.fingerprint));
    assert_eq!(prompt["scopes"], json!(["read", "spend", "gates"]));
    assert_eq!(
        prompt["instance_name"],
        json!(rig.pairing.machine().instance_name)
    );

    assert_eq!(rig.poll_after(5).await, "authorization_pending");
    rig.approve(
        &authenticator,
        &passkey,
        &ticket.user_code,
        vec![Scope::Read, Scope::Gates],
    )
    .await;
    assert_eq!(rig.poll_after(5).await, "paired");

    assert_eq!(rig.settled().await, DevicePairing::Idle);
    assert_eq!(rig.asked(), [5, 5]);
    let status = rig.pairing.status().expect("a readable status");
    assert_eq!(status.state, HubPairingState::Paired);
    assert_eq!(
        status.fingerprint.as_deref(),
        Some(ticket.fingerprint.as_str())
    );

    let answer = mutex(&rig.device.answer).take().expect("the Hub answered");
    let pins = rig
        .db
        .load()
        .expect("readable pins")
        .expect("pins were written");
    assert_eq!(pins.hub_origin, rig.origin);
    assert_eq!(json!(pins.instance_id), json!(instance_id.0.to_string()));
    assert_eq!(json!(pins.instance_id), answer["instance_id"]);
    assert_eq!(
        json!(pins.request_signing_key),
        answer["request_signing_key"]
    );
    assert_eq!(json!(pins.rp_id), answer["rp_id"]);
    assert_eq!(pins.scopes, ["read", "gates"]);
    let pinned: Vec<Value> = pins
        .passkeys
        .iter()
        .map(|p| json!({ "credential_id": p.credential_id, "public_key": p.public_key }))
        .collect();
    assert_eq!(answer["passkeys"].as_array().map(Vec::len), Some(2));
    assert_eq!(json!(pinned), answer["passkeys"]);
    assert_eq!(
        rig.stored_refresh_token().map(Value::from),
        Some(answer["refresh_token"].clone())
    );

    let handed = std::mem::take(&mut *mutex(&rig.device.handed));
    let [(user, new)] = handed.as_slice() else {
        panic!("the store was handed {} pairings", handed.len());
    };
    assert_eq!(*user, passkey.user_id);
    assert_eq!(new.kind, InstanceKind::Runner);
    assert_eq!(new.scopes, [Scope::Read, Scope::Gates]);
    assert_eq!(new.device_key.jkt, pins.device_jkt);
}

#[tokio::test]
async fn a_denial_at_the_hub_fails_the_runners_pairing_and_pins_nothing() {
    let mut rig = Rig::new("hub_runner_device_denial").await;
    let (_, passkey) = rig.tenant();
    let ticket = rig.begin().await;

    assert_eq!(rig.poll_after(5).await, "authorization_pending");
    let denied = denied_device(
        &rig.hub,
        passkey.user_id,
        DeviceLookup::new(&ticket.user_code),
        rig.clock.now(),
    );
    assert!(denied.is_ok(), "the Hub refused the denial");
    assert_eq!(rig.poll_after(5).await, "access_denied");

    assert_eq!(
        rig.settled().await,
        DevicePairing::Failed(HubPairingError::Denied)
    );
    assert!(rig.db.load().expect("readable pins").is_none());
    assert_eq!(rig.stored_refresh_token(), None);
    assert!(mutex(&rig.device.handed).is_empty());
}

/// A sleep that ends early is the one way a client that keeps its interval
/// polls too soon. What it does with the `slow_down` that earns is what the
/// Hub must then accept, through to an approval.
#[tokio::test]
async fn after_a_slow_down_the_runner_polls_at_an_interval_the_hub_accepts() {
    let mut rig = Rig::new("hub_runner_device_slow_down").await;
    let (authenticator, passkey) = rig.tenant();
    *mutex(&rig.device.paired) = Some(Paired {
        instance_id: InstanceId::generate(),
        passkeys: vec![passkey.clone()],
    });
    let ticket = rig.begin().await;

    assert_eq!(rig.poll_after(5).await, "authorization_pending");
    assert_eq!(rig.poll_after(4).await, "slow_down");
    assert_eq!(rig.poll_after(10).await, "authorization_pending");
    rig.approve(
        &authenticator,
        &passkey,
        &ticket.user_code,
        vec![Scope::Gates],
    )
    .await;
    assert_eq!(rig.poll_after(10).await, "paired");

    assert_eq!(rig.settled().await, DevicePairing::Idle);
    assert_eq!(rig.asked(), [5, 5, 10, 10]);
    let status = rig.pairing.status().expect("a readable status");
    assert_eq!(status.state, HubPairingState::Paired);
    assert_eq!(status.scopes, ["gates"]);
}
