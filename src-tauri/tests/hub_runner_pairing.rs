//! A `demeteo-runner`'s half of Hub pairing as `demeteo-core` builds it: the
//! device-code grant, its poll decision, and the sealed store that stands in
//! for the keyring on a runner host (docs/HUB.md §4 *Pairing a runner by
//! device code*, decision 64).
//!
//! Here rather than in `demeteo-core` because a Demeteo run validates Rust
//! with `cd src-tauri && cargo test`, as `tests/infrastructure/hub.rs` says.
//! Every double answers only what it was scripted with.

use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::hub::sealed_store::{load_store_key, SealedSecretStore};
use demeteo_core::adapters::hub::socket::Sleeper;
use demeteo_core::adapters::hub::testing::{MemorySecretStore, ScriptedDeviceEndpoint};
use demeteo_core::adapters::hub::HubPairing;
use demeteo_core::domain::hub_pairing::device::{
    after_device_poll, device_authorization_form_body, device_authorization_url,
    device_grant_refusal, device_token_form_body, DeviceAuthorization, DeviceGrantEnd,
    DevicePairing, DevicePollStep, DEVICE_CODE_GRANT,
};
use demeteo_core::domain::hub_pairing::keyring_names::hub_keyring_account;
use demeteo_core::domain::hub_pairing::sealed::{store_key, store_key_path, SEALED_BACKEND};
use demeteo_core::domain::hub_pairing::{
    consent_machine, HubPairingError, HubPairingState, HubPins, HubSecret, PairingAnswer,
};
use demeteo_core::error::AppError;
use demeteo_core::ports::hub::{
    HubPinRepository, HubSealedSecretRepository, HubSecretStore, HubTokenEndpoint,
};
use serde_json::{json, Value};

const HUB_URL: &str = "https://Hub.Example.com/";
const ORIGIN: &str = "https://hub.example.com";
const DEVICE_CODE: &str = "dc-0123456789abcdefghijklmnopqrstuvwxyzABCDEFG";
const USER_CODE: &str = "BCDF-GHJK";
const ACCESS_TOKEN: &str = "dha_access-0123456789abcdefghijklmnopqrstuv";
const REFRESH_TOKEN: &str = "dhr_refresh-0123456789abcdefghijklmnopqrstu";

// ---- the poll decision -------------------------------------------------

#[test]
fn pending_and_an_unanswered_poll_wait_the_same_interval_until_expiry() {
    for end in [DeviceGrantEnd::Pending, DeviceGrantEnd::Unanswered] {
        assert_eq!(
            after_device_poll(end.clone(), 5, 100, 700),
            DevicePollStep::Wait { interval_secs: 5 },
            "{end:?}"
        );
        assert_eq!(
            after_device_poll(end.clone(), 5, 700, 700),
            DevicePollStep::Fail(HubPairingError::DeviceCodeExpired),
            "{end:?} at expiry"
        );
    }
}

#[test]
fn slow_down_adds_five_seconds_each_time() {
    assert_eq!(
        after_device_poll(DeviceGrantEnd::SlowDown, 5, 100, 700),
        DevicePollStep::Wait { interval_secs: 10 }
    );
    assert_eq!(
        after_device_poll(DeviceGrantEnd::SlowDown, 10, 100, 700),
        DevicePollStep::Wait { interval_secs: 15 }
    );
    assert_eq!(
        after_device_poll(DeviceGrantEnd::SlowDown, 10, 701, 700),
        DevicePollStep::Fail(HubPairingError::DeviceCodeExpired)
    );
}

#[test]
fn denied_expired_and_any_other_refusal_end_the_pairing() {
    let cases = [
        (DeviceGrantEnd::Denied, HubPairingError::Denied),
        (DeviceGrantEnd::Expired, HubPairingError::DeviceCodeExpired),
        (
            DeviceGrantEnd::Refused(HubPairingError::ClockSkew),
            HubPairingError::ClockSkew,
        ),
    ];
    for (end, error) in cases {
        assert_eq!(
            after_device_poll(end, 5, 100, 700),
            DevicePollStep::Fail(error)
        );
    }
}

#[test]
fn each_device_grant_error_code_means_what_rfc_8628_says() {
    let body = |code: &str| json!({ "error": code }).to_string().into_bytes();
    let cases = [
        ("authorization_pending", DeviceGrantEnd::Pending),
        ("slow_down", DeviceGrantEnd::SlowDown),
        ("access_denied", DeviceGrantEnd::Denied),
        ("expired_token", DeviceGrantEnd::Expired),
        (
            "invalid_grant",
            DeviceGrantEnd::Refused(HubPairingError::HubRefused {
                error: "invalid_grant".into(),
            }),
        ),
        (
            "invalid_dpop_proof",
            DeviceGrantEnd::Refused(HubPairingError::ClockSkew),
        ),
    ];
    for (code, end) in cases {
        assert_eq!(device_grant_refusal(&body(code)), end, "{code}");
    }
    assert!(matches!(
        device_grant_refusal(b"<html>bad gateway</html>"),
        DeviceGrantEnd::Refused(HubPairingError::MalformedAnswer(_))
    ));
}

fn form_members(body: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://x/?{body}"))
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[test]
fn the_device_forms_carry_exactly_the_members_the_hub_reads() {
    let machine = consent_machine("runner 1", "linux", "x86_64", "1.2.3");
    assert_eq!(
        form_members(&device_authorization_form_body(
            "read spend gates",
            &machine
        )),
        [
            ("scope", "read spend gates"),
            ("instance_name", "runner 1"),
            ("os", "linux"),
            ("arch", "x86_64"),
            ("app_version", "1.2.3"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
    );
    assert_eq!(
        form_members(&device_token_form_body("a+b/c=")),
        [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", "a+b/c="),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
    );
    assert_eq!(
        DEVICE_CODE_GRANT,
        "urn:ietf:params:oauth:grant-type:device_code"
    );
    assert_eq!(
        device_authorization_url(ORIGIN),
        "https://hub.example.com/device_authorization"
    );
}

fn authorization(overrides: Value) -> DeviceAuthorization {
    let mut body = json!({
        "device_code": DEVICE_CODE,
        "user_code": USER_CODE,
        "verification_uri": format!("{ORIGIN}/#/device"),
        "verification_uri_complete": format!("{ORIGIN}/#/device/{USER_CODE}"),
        "expires_in": 600,
        "interval": 5,
    });
    for (k, v) in overrides.as_object().unwrap() {
        body[k] = v.clone();
    }
    serde_json::from_value(body).unwrap()
}

#[test]
fn an_accepted_authorization_becomes_a_ticket_and_keeps_the_device_code_out_of_it() {
    let grant = authorization(json!({}))
        .accept(ORIGIN, "SHA256:abcd efgh ijkl mnop".into(), 1_000)
        .expect("the Hub's own answer");
    assert_eq!(grant.ticket.user_code, USER_CODE);
    assert_eq!(grant.ticket.expires_at, 1_600);
    assert_eq!(grant.ticket.fingerprint, "SHA256:abcd efgh ijkl mnop");
    assert_eq!(grant.interval_secs, 5);
    assert_eq!(grant.device_code.expose(), DEVICE_CODE);
    assert!(!serde_json::to_string(&grant.ticket)
        .unwrap()
        .contains(DEVICE_CODE));
}

#[test]
fn an_authorization_a_terminal_should_not_show_is_malformed() {
    let refused = [
        json!({ "user_code": "BCDF\u{1b}[2J" }),
        json!({ "user_code": "" }),
        json!({ "verification_uri": "https://evil.example/#/device" }),
        json!({ "verification_uri_complete": "https://hub.example.com.evil/x" }),
        json!({ "verification_uri": format!("{ORIGIN}/#/device\nrm") }),
        json!({ "expires_in": 0 }),
    ];
    for overrides in refused {
        let accepted = authorization(overrides.clone()).accept(ORIGIN, String::new(), 0);
        assert!(
            matches!(accepted, Err(HubPairingError::MalformedAnswer(_))),
            "{overrides}"
        );
    }
    let zero = authorization(json!({ "interval": 0 }))
        .accept(ORIGIN, String::new(), 0)
        .unwrap();
    assert_eq!(zero.interval_secs, 1, "an interval of 0 would spin");
}

// ---- the sealed store --------------------------------------------------

fn db(tag: &str) -> (demeteo_core::test_dir::TestDir, Arc<SqliteAdapter>) {
    let dir = demeteo_core::test_dir::TestDir::new(tag);
    let conn = rusqlite::Connection::open(dir.path().join("demeteo.db")).unwrap();
    (dir, Arc::new(SqliteAdapter::new(conn).unwrap()))
}

const KEY: [u8; 32] = [7; 32];
const ACCOUNT: &str = "hub_refresh_token:install-1";
const OTHER_ACCOUNT: &str = "hub_device_key:install-1";

fn sealed(db: &Arc<SqliteAdapter>, key: Result<[u8; 32], &'static str>) -> SealedSecretStore {
    SealedSecretStore::new(db.clone(), key, SEALED_BACKEND)
}

#[test]
fn a_sealed_value_round_trips_and_is_stored_as_ciphertext_only() {
    let (_dir, db) = db("hub_sealed_round_trip");
    let store = sealed(&db, Ok(KEY));

    assert_eq!(store.get(ACCOUNT).unwrap(), None);
    store.set(ACCOUNT, REFRESH_TOKEN).unwrap();
    assert_eq!(store.get(ACCOUNT).unwrap().as_deref(), Some(REFRESH_TOKEN));

    let row = db.load_sealed(ACCOUNT).unwrap().expect("a row");
    assert_eq!(row.nonce.len(), 12);
    assert_eq!(row.ciphertext.len(), REFRESH_TOKEN.len() + 16);
    assert!(!row
        .ciphertext
        .windows(REFRESH_TOKEN.len())
        .any(|w| w == REFRESH_TOKEN.as_bytes()));

    store.set(ACCOUNT, "rotated").unwrap();
    let rotated = db.load_sealed(ACCOUNT).unwrap().unwrap();
    assert_ne!(rotated.nonce, row.nonce, "a fresh nonce per write");
    assert_eq!(store.get(ACCOUNT).unwrap().as_deref(), Some("rotated"));

    store.delete(ACCOUNT).unwrap();
    store.delete(ACCOUNT).unwrap();
    assert_eq!(store.get(ACCOUNT).unwrap(), None);
    let status = store.status();
    assert!(status.available);
    assert_eq!(status.backend, "systemd-creds");
    assert_eq!(status.reason, None);
}

#[test]
fn a_sealed_value_does_not_open_under_another_key() {
    let (_dir, db) = db("hub_sealed_wrong_key");
    sealed(&db, Ok(KEY)).set(ACCOUNT, REFRESH_TOKEN).unwrap();

    let other = sealed(&db, Ok([8; 32])).get(ACCOUNT);

    let err = other.expect_err("another key opens nothing");
    assert!(!err.to_string().contains(REFRESH_TOKEN));
}

#[test]
fn a_tampered_ciphertext_or_nonce_does_not_open() {
    let (_dir, db) = db("hub_sealed_tampered");
    let store = sealed(&db, Ok(KEY));
    store.set(ACCOUNT, REFRESH_TOKEN).unwrap();
    let row = db.load_sealed(ACCOUNT).unwrap().unwrap();

    let mut ciphertext = row.clone();
    ciphertext.ciphertext[0] ^= 1;
    db.store_sealed(ACCOUNT, &ciphertext).unwrap();
    assert!(store.get(ACCOUNT).is_err(), "a flipped ciphertext bit");

    let mut nonce = row.clone();
    nonce.nonce[0] ^= 1;
    db.store_sealed(ACCOUNT, &nonce).unwrap();
    assert!(store.get(ACCOUNT).is_err(), "a flipped nonce bit");

    let mut short = row;
    short.nonce.pop();
    db.store_sealed(ACCOUNT, &short).unwrap();
    assert!(store.get(ACCOUNT).is_err(), "a short nonce");
}

#[test]
fn a_sealed_value_moved_to_another_account_does_not_open_there() {
    let (_dir, db) = db("hub_sealed_aad");
    let store = sealed(&db, Ok(KEY));
    store.set(ACCOUNT, REFRESH_TOKEN).unwrap();

    let row = db.load_sealed(ACCOUNT).unwrap().unwrap();
    db.store_sealed(OTHER_ACCOUNT, &row).unwrap();

    assert!(store.get(OTHER_ACCOUNT).is_err());
    assert_eq!(store.get(ACCOUNT).unwrap().as_deref(), Some(REFRESH_TOKEN));
}

#[test]
fn a_store_without_a_key_is_unavailable_says_why_and_touches_nothing() {
    let (_dir, db) = db("hub_sealed_no_key");
    let store = sealed(&db, Err("no hub-store-key credential was loaded"));

    let status = store.status();
    assert!(!status.available);
    assert_eq!(
        status.reason,
        Some("no hub-store-key credential was loaded")
    );
    assert_eq!(
        status.unavailable_error(),
        HubPairingError::SecretStoreUnavailable {
            backend: "systemd-creds".into(),
            reason: "no hub-store-key credential was loaded".into(),
        }
    );
    assert!(store.set(ACCOUNT, REFRESH_TOKEN).is_err());
    assert!(store.get(ACCOUNT).is_err());
    assert!(store.delete(ACCOUNT).is_err());
    assert_eq!(db.load_sealed(ACCOUNT).unwrap(), None);
}

#[test]
fn the_store_key_is_exactly_32_bytes_from_the_credentials_directory() {
    assert!(store_key_path(None).is_err());
    assert!(store_key_path(Some(OsStr::new(""))).is_err());
    assert_eq!(
        store_key_path(Some(OsStr::new("/run/credentials/demeteo-runner.service"))).unwrap(),
        std::path::Path::new("/run/credentials/demeteo-runner.service").join("hub-store-key")
    );
    assert_eq!(store_key(&KEY), Ok(KEY));
    assert!(store_key(&[7; 31]).is_err());
    assert!(store_key(&[7; 33]).is_err());
    assert!(store_key(b"").is_err());

    let dir = demeteo_core::test_dir::TestDir::new("hub_store_key_file");
    let creds = dir.path().as_os_str();
    assert_eq!(
        load_store_key(Some(creds)),
        Err("no hub-store-key credential was loaded")
    );
    std::fs::write(dir.path().join("hub-store-key"), [7u8; 33]).unwrap();
    assert_eq!(
        load_store_key(Some(creds)),
        Err("hub-store-key is not exactly 32 bytes")
    );
    std::fs::write(dir.path().join("hub-store-key"), KEY).unwrap();
    assert_eq!(load_store_key(Some(creds)), Ok(KEY));
    assert!(load_store_key(None).is_err());
}

// ---- the device flow on `HubPairing` ------------------------------------

fn hub_answer() -> PairingAnswer {
    serde_json::from_value(json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "DPoP",
        "expires_in": 900,
        "refresh_token": REFRESH_TOKEN,
        "scope": "read spend gates",
        "instance_id": "7f0c2a4e-1b3d-4c5e-8f90-123456789abc",
        "request_signing_key": "__________________________________________8",
        "rp_id": "hub.example.com",
        "passkeys": [{"credential_id": "Y3JlZC0x", "public_key": "cGstMQ"}]
    }))
    .unwrap()
}

/// Returns at once, recording each wait.
#[derive(Default)]
struct InstantSleeper(Mutex<Vec<Duration>>);

#[async_trait::async_trait]
impl Sleeper for InstantSleeper {
    async fn sleep(&self, duration: Duration) {
        self.0.lock().unwrap().push(duration);
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

/// The pins, recording at each save whether the refresh token was already
/// in the store.
struct OrderedPins {
    db: Arc<SqliteAdapter>,
    secrets: Arc<dyn HubSecretStore>,
    refresh_stored_at_save: Mutex<Vec<bool>>,
}

impl HubPinRepository for OrderedPins {
    fn install_id(&self) -> Result<String, AppError> {
        self.db.install_id()
    }
    fn load(&self) -> Result<Option<HubPins>, AppError> {
        HubPinRepository::load(&*self.db)
    }
    fn save(&self, pins: &HubPins) -> Result<(), AppError> {
        let account = hub_keyring_account(&self.db.install_id()?, HubSecret::RefreshToken);
        let stored = self.secrets.get(&account)?.is_some();
        self.refresh_stored_at_save.lock().unwrap().push(stored);
        self.db.save(pins)
    }
    fn clear(&self) -> Result<(), AppError> {
        self.db.clear()
    }
}

/// `/token`'s code redemption, which nothing here may reach.
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

struct Rig {
    _dir: demeteo_core::test_dir::TestDir,
    db: Arc<SqliteAdapter>,
    secrets: Arc<dyn HubSecretStore>,
    pins: Arc<OrderedPins>,
    device: Arc<ScriptedDeviceEndpoint>,
    hub: Arc<HubPairing>,
}

impl Rig {
    fn new(tag: &str, sleeper: Arc<dyn Sleeper>) -> Self {
        Self::with_store(tag, sleeper, |db| {
            Arc::new(MemorySecretStore::new(&db.install_id().unwrap()))
        })
    }

    fn with_store(
        tag: &str,
        sleeper: Arc<dyn Sleeper>,
        store: impl FnOnce(&Arc<SqliteAdapter>) -> Arc<dyn HubSecretStore>,
    ) -> Self {
        let (dir, db) = db(tag);
        let secrets = store(&db);
        let pins = Arc::new(OrderedPins {
            db: db.clone(),
            secrets: secrets.clone(),
            refresh_stored_at_save: Mutex::default(),
        });
        let device = Arc::new(ScriptedDeviceEndpoint::new());
        let hub = Arc::new(
            HubPairing::new(
                pins.clone(),
                secrets.clone(),
                Arc::new(NoRedemption),
                db.clone(),
            )
            .with_device_endpoint(device.clone())
            .with_device_sleeper(sleeper),
        );
        Self {
            _dir: dir,
            db,
            secrets,
            pins,
            device,
            hub,
        }
    }

    fn account(&self, secret: HubSecret) -> String {
        hub_keyring_account(&self.db.install_id().unwrap(), secret)
    }

    /// The state once the poll task has ended, or a failed test after 5 s.
    async fn settled(&self) -> DevicePairing {
        for _ in 0..5_000 {
            match self.hub.device_pairing() {
                DevicePairing::Pending(_) => tokio::time::sleep(Duration::from_millis(1)).await,
                settled => return settled,
            }
        }
        panic!("the device pairing never settled");
    }
}

/// Unpadded base64url, by hand, as `tests/infrastructure/hub.rs` decodes it:
/// this package has no `base64` dependency.
fn base64url_decode(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let (mut bits, mut len, mut out) = (0u32, 0, Vec::new());
    for byte in text.bytes() {
        let value = ALPHABET.iter().position(|&a| a == byte);
        bits = (bits << 6) | value.unwrap_or_else(|| panic!("{byte} in {text:?}")) as u32;
        len += 6;
        if len >= 8 {
            len -= 8;
            out.push((bits >> len) as u8);
        }
    }
    out
}

fn claims(proof: &str) -> Value {
    let claims = proof.split('.').nth(1).expect("a compact JWS");
    serde_json::from_slice(&base64url_decode(claims)).unwrap()
}

#[tokio::test]
async fn an_approved_device_pairing_stores_the_token_before_the_pins_and_reads_paired() {
    let sleeper = Arc::new(InstantSleeper::default());
    let rig = Rig::new("hub_device_approved", sleeper.clone());
    rig.device.authorize(ORIGIN, DEVICE_CODE, USER_CODE, 5);
    rig.device.poll_ends(DeviceGrantEnd::Pending);
    rig.device.poll_ends(DeviceGrantEnd::SlowDown);
    rig.device.poll_answers(hub_answer());

    let ticket = rig
        .hub
        .begin_device_pairing(HUB_URL)
        .await
        .expect("a ticket");

    assert_eq!(ticket.user_code, USER_CODE);
    assert_eq!(ticket.verification_uri, format!("{ORIGIN}/#/device"));
    assert_eq!(rig.settled().await, DevicePairing::Idle);
    let status = rig.hub.status().unwrap();
    assert_eq!(status.state, HubPairingState::Paired);
    assert_eq!(
        status.fingerprint.as_deref(),
        Some(ticket.fingerprint.as_str())
    );
    assert_eq!(*rig.pins.refresh_stored_at_save.lock().unwrap(), [true]);
    assert_eq!(
        rig.secrets
            .get(&rig.account(HubSecret::RefreshToken))
            .unwrap()
            .as_deref(),
        Some(REFRESH_TOKEN)
    );
    assert_eq!(
        rig.hub
            .access_token()
            .map(|t| t.expose().to_owned())
            .as_deref(),
        Some(ACCESS_TOKEN)
    );
    assert_eq!(
        *sleeper.0.lock().unwrap(),
        [5, 5, 10].map(Duration::from_secs),
        "the interval, again on pending, then five more after slow_down"
    );

    let [authorize] = &rig.device.authorize_calls()[..] else {
        panic!("one /device_authorization");
    };
    assert_eq!(authorize.url, format!("{ORIGIN}/device_authorization"));
    assert_eq!(claims(&authorize.dpop)["htu"], json!(authorize.url));
    let polls = rig.device.poll_calls();
    assert_eq!(polls.len(), 3);
    for poll in &polls {
        assert_eq!(poll.url, format!("{ORIGIN}/token"));
        assert_eq!(poll.form_body, device_token_form_body(DEVICE_CODE));
        assert_eq!(claims(&poll.dpop)["htu"], json!(poll.url));
    }
    assert_eq!(rig.device.unscripted(), 0);
}

async fn failed_pairing(tag: &str, end: DeviceGrantEnd) -> (DevicePairing, Rig) {
    let rig = Rig::new(tag, Arc::new(InstantSleeper::default()));
    rig.device.authorize(ORIGIN, DEVICE_CODE, USER_CODE, 5);
    rig.device.poll_ends(DeviceGrantEnd::Pending);
    rig.device.poll_ends(end);
    rig.hub
        .begin_device_pairing(HUB_URL)
        .await
        .expect("a ticket");
    (rig.settled().await, rig)
}

fn assert_left_unpaired(rig: &Rig) {
    assert!(
        rig.secrets
            .get(&rig.account(HubSecret::DeviceKey))
            .unwrap()
            .is_some(),
        "the device key is kept for the next attempt, as `pair` keeps it"
    );
    assert_eq!(
        rig.secrets
            .get(&rig.account(HubSecret::RefreshToken))
            .unwrap(),
        None
    );
    assert_eq!(HubPinRepository::load(&*rig.db).unwrap(), None);
    assert_eq!(rig.hub.status().unwrap().state, HubPairingState::Unpaired);
    assert_eq!(rig.device.unscripted(), 0);
}

#[tokio::test]
async fn a_denied_device_pairing_fails_denied_and_stores_no_token() {
    let (state, rig) = failed_pairing("hub_device_denied", DeviceGrantEnd::Denied).await;
    assert_eq!(state, DevicePairing::Failed(HubPairingError::Denied));
    assert_left_unpaired(&rig);
}

#[tokio::test]
async fn an_expired_device_pairing_fails_expired_and_stores_no_token() {
    let (state, rig) = failed_pairing("hub_device_expired", DeviceGrantEnd::Expired).await;
    assert_eq!(
        state,
        DevicePairing::Failed(HubPairingError::DeviceCodeExpired)
    );
    assert_left_unpaired(&rig);
}

#[tokio::test]
async fn an_unavailable_sealed_store_refuses_with_its_reason_before_any_network_call() {
    let rig = Rig::with_store("hub_device_no_key", Arc::new(ParkedSleeper), |db| {
        Arc::new(SealedSecretStore::new(
            db.clone(),
            Err("no hub-store-key credential was loaded"),
            SEALED_BACKEND,
        ))
    });

    let refused = rig.hub.begin_device_pairing(HUB_URL).await;

    assert_eq!(
        refused,
        Err(HubPairingError::SecretStoreUnavailable {
            backend: "systemd-creds".into(),
            reason: "no hub-store-key credential was loaded".into(),
        })
    );
    assert!(rig.device.authorize_calls().is_empty());
    assert_eq!(rig.hub.device_pairing(), DevicePairing::Idle);
    assert_eq!(HubPinRepository::load(&*rig.db).unwrap(), None);
}

#[tokio::test]
async fn a_device_pairing_through_the_sealed_store_reads_paired() {
    let rig = Rig::with_store(
        "hub_device_sealed",
        Arc::new(InstantSleeper::default()),
        |db| Arc::new(SealedSecretStore::new(db.clone(), Ok(KEY), SEALED_BACKEND)),
    );
    rig.device.authorize(ORIGIN, DEVICE_CODE, USER_CODE, 5);
    rig.device.poll_answers(hub_answer());

    rig.hub
        .begin_device_pairing(HUB_URL)
        .await
        .expect("a ticket");

    assert_eq!(rig.settled().await, DevicePairing::Idle);
    let status = rig.hub.status().unwrap();
    assert_eq!(status.state, HubPairingState::Paired);
    assert_eq!(status.keyring_backend, "systemd-creds");
}

#[tokio::test]
async fn a_second_begin_replaces_a_pending_pairing() {
    let rig = Rig::new("hub_device_replaced", Arc::new(ParkedSleeper));
    rig.device.authorize(ORIGIN, DEVICE_CODE, USER_CODE, 5);
    rig.device.authorize(ORIGIN, "dc-second", "MNPQ-RSTV", 5);

    let first = rig
        .hub
        .begin_device_pairing(HUB_URL)
        .await
        .expect("a ticket");
    assert_eq!(
        rig.hub.device_pairing(),
        DevicePairing::Pending(first.clone())
    );
    let second = rig
        .hub
        .begin_device_pairing(HUB_URL)
        .await
        .expect("a second ticket");

    assert_eq!(second.user_code, "MNPQ-RSTV");
    assert_eq!(second.fingerprint, first.fingerprint, "the same device key");
    assert_eq!(rig.hub.device_pairing(), DevicePairing::Pending(second));
    assert!(rig.device.poll_calls().is_empty());
    assert_eq!(rig.device.unscripted(), 0);
}

#[tokio::test]
async fn disconnect_cancels_a_pending_device_pairing() {
    let rig = Rig::new("hub_device_disconnect", Arc::new(ParkedSleeper));
    rig.device.authorize(ORIGIN, DEVICE_CODE, USER_CODE, 5);
    rig.hub
        .begin_device_pairing(HUB_URL)
        .await
        .expect("a ticket");

    let status = tokio::time::timeout(Duration::from_secs(5), rig.hub.disconnect())
        .await
        .expect("disconnect is not refused or held by the pending pairing")
        .expect("a disconnect");

    assert_eq!(status.state, HubPairingState::Unpaired);
    assert_eq!(rig.hub.device_pairing(), DevicePairing::Idle);
    assert!(rig
        .secrets
        .get(&rig.account(HubSecret::DeviceKey))
        .unwrap()
        .is_none());
    assert!(rig.device.poll_calls().is_empty());
}
