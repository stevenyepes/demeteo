#![cfg(target_os = "linux")]
//! The desktop's half of pairing, judged by the Hub's own code.
//!
//! `src-tauri/tests/infrastructure/hub.rs` checks the desktop against fixed
//! RFC vectors on every OS; this file hands what the desktop produces to
//! `demeteo-hub`, which is a dev-dependency of this package on Linux only
//! and must never become one of `demeteo-core` (`hub_layout.rs`).

use async_trait::async_trait;
use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::hub::device_key::DeviceKey;
use demeteo_core::adapters::hub::loopback::CallbackListener;
use demeteo_core::adapters::hub::testing::MemorySecretStore;
use demeteo_core::adapters::hub::HubPairing;
use demeteo_core::domain::hub_pairing::pkce::s256_challenge;
use demeteo_core::domain::hub_pairing::{
    authorize_url, hub_origin, token_refusal, token_url, AuthorizeRequest, HubPairingError,
    PairingAnswer, SCOPES,
};
use demeteo_core::ports::hub::{BrowserLauncher, HubPinRepository, HubTokenEndpoint};
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::testing::{answered, lazy_state_with_keys, SoftAuthenticator};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::authorize::{self, AuthorizeParams};
use demeteo_hub::pairing::dpop::{verify, Expected};
use demeteo_hub::pairing::handlers::{approved_pairing, redeemed, started_pairing};
use demeteo_hub::pairing::jwk;
use demeteo_hub::routes::AppState;
use demeteo_hub::store::instances::Paired;
use demeteo_hub::store::{InstanceId, UserId};
use reqwest::Url;
use serde_json::{json, Value};
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HUB_URL: &str = "https://Hub.Example:443/";

/// Every query member of `url`, in the shape the Hub's `/authorize`
/// extractor reads it.
fn authorize_params(url: &Url) -> AuthorizeParams {
    serde_json::from_value(members(url).into()).expect("the Hub's query shape")
}

fn members(url: &Url) -> serde_json::Map<String, Value> {
    url.query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned().into()))
        .collect()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock set after 1970")
        .as_secs()
}

#[test]
fn the_hub_accepts_a_device_key_proof_for_post_token_bound_to_its_thumbprint() {
    let htu = token_url(&hub_origin(HUB_URL).expect("a valid Hub URL"));
    let key = DeviceKey::generate().expect("a P-256 key");
    let now = now();

    let proof = key
        .dpop_proof(&htu, now, "desktop-jti-1")
        .expect("a signed proof");
    let verified = verify(
        &proof,
        &Expected {
            htm: "POST",
            htu: &htu,
        },
        now,
    )
    .expect("the Hub verifies the desktop's proof");

    assert_eq!(verified.jkt, key.jkt());
    assert_eq!(verified.jti, "desktop-jti-1");
    assert_eq!(verified.iat, now);
}

#[test]
fn the_desktop_fingerprint_is_the_one_the_hub_shows_for_its_thumbprint() {
    let key = DeviceKey::generate().expect("a P-256 key");
    let htu = token_url(&hub_origin(HUB_URL).expect("a valid Hub URL"));
    let now = now();
    let proof = key.dpop_proof(&htu, now, "j").expect("a signed proof");
    let verified = verify(
        &proof,
        &Expected {
            htm: "POST",
            htu: &htu,
        },
        now,
    )
    .expect("the Hub verifies the desktop's proof");

    assert_eq!(key.fingerprint(), jwk::fingerprint(&verified.jkt));
}

/// The whole query, not only `redirect_uri`, goes through the Hub's
/// `validate`: it refuses a request missing any parameter, so this also
/// catches one the desktop forgot to send.
#[test]
fn the_hub_accepts_the_desktops_authorize_query_with_its_loopback_redirect() {
    let origin = hub_origin(HUB_URL).expect("a valid Hub URL");
    let (_listener, redirect_uri) = CallbackListener::bind().expect("a loopback bind");
    let key = DeviceKey::generate().expect("a P-256 key");
    let request = AuthorizeRequest {
        code_challenge: s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        redirect_uri: redirect_uri.clone(),
        dpop_jkt: key.jkt(),
        scope: SCOPES.into(),
        state: "Wm9vbS1zdGF0ZS0zMi1yYW5kb20tYnl0ZXMtYmFzZTY0dQ".into(),
        instance_name: "Ana's laptop".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        app_version: "1.2.3".into(),
    };

    let url = Url::parse(&authorize_url(&origin, &request).expect("an authorize URL"))
        .expect("a parseable URL");
    let params = authorize_params(&url);
    let accepted = authorize::validate(&params).expect("the Hub accepts the desktop's query");

    assert_eq!(url.path(), "/authorize");
    assert_eq!(accepted.redirect_uri, redirect_uri);
    assert_eq!(accepted.dpop_jkt, request.dpop_jkt);
    assert_eq!(accepted.code_challenge, request.code_challenge);
    assert_eq!(accepted.state, request.state);
}

/// The name the Hub below knows its request-signing key by.
const SIGNING_KEY: &str = "demeteo-hub-signing";

/// A Hub served at a `localhost` origin, the one plain-HTTP origin both the
/// Hub and the desktop accept. Nothing listens there: the doubles below hand
/// the desktop's requests to the Hub's handlers directly, so the port only
/// has to agree between the two.
const HUB_ORIGIN: &str = "http://localhost:47123";

fn mutex<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The user's browser: the Hub's `/authorize` and consent bodies, approving
/// every requested scope for `user`, then the redirect they answer followed
/// to the desktop's loopback listener. A refusal panics, since `pair` would
/// otherwise report it only as a browser that failed to open.
struct ApprovingBrowser {
    hub: AppState,
    user: UserId,
}

impl BrowserLauncher for ApprovingBrowser {
    fn open(&self, url: &str) -> Result<(), String> {
        let url = Url::parse(url).expect("the desktop opens a URL");
        assert_eq!(url.origin().ascii_serialization(), HUB_ORIGIN);
        assert_eq!(url.path(), "/authorize");
        let request =
            authorize::validate(&authorize_params(&url)).expect("the Hub accepts the query");
        let scopes: Vec<&str> = SCOPES.split(' ').collect();

        let start = serde_json::from_value(json!({ "request": authorize::encode(&request) }))
            .expect("a start body");
        let Ok(prompt) = started_pairing(&self.hub, self.user, start, now()) else {
            panic!("the Hub refused to start the desktop's pairing");
        };
        let prompt = serde_json::to_value(prompt).expect("a prompt that is JSON");
        let approval = serde_json::from_value(json!({
            "pairing_id": prompt["pairing_id"],
            "scopes": scopes,
        }))
        .expect("an approval body");
        let Ok(consent) = approved_pairing(&self.hub, self.user, approval, now()) else {
            panic!("the Hub refused the approval");
        };
        let consent = serde_json::to_value(consent).expect("a consent answer that is JSON");
        let redirect = consent["redirect"]
            .as_str()
            .expect("an approval redirects")
            .to_owned();
        tokio::spawn(async move { reqwest::get(redirect).await });
        Ok(())
    }
}

/// The Hub's `POST /token` past its extractors: the desktop's form body and
/// its one `DPoP` header, unchanged, into `redeemed`, and the Hub's answer
/// serialized back as the JSON the desktop parses. A refusal is answered as
/// the route answers it. The store is a recording closure answering `paired`,
/// since the Hub's own is a database that is not there.
struct HubToken {
    hub: AppState,
    paired: Mutex<Option<Paired>>,
    handed: Mutex<Vec<UserId>>,
    answer: Mutex<Option<Value>>,
}

#[async_trait]
impl HubTokenEndpoint for HubToken {
    async fn exchange(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<PairingAnswer, HubPairingError> {
        assert_eq!(token_url, format!("{}/token", self.hub.rp().origin));
        let mut form = Url::parse("http://form.invalid/").expect("a URL");
        form.set_query(Some(&form_body));
        let mut form = members(&form);
        assert_eq!(form.remove("grant_type"), Some(json!("authorization_code")));
        let form = serde_json::from_value(form.into()).expect("the Hub's redemption form");

        let paired = mutex(&self.paired).take().expect("one redemption");
        let pair = |user, _| {
            mutex(&self.handed).push(user);
            ready(Ok(paired))
        };
        match redeemed(&self.hub, form, Some(&dpop), now(), pair).await {
            Ok(response) => {
                let body = serde_json::to_vec(&response).expect("an answer that is JSON");
                *mutex(&self.answer) = serde_json::from_slice(&body).ok();
                serde_json::from_slice(&body).map_err(|e| {
                    HubPairingError::MalformedAnswer(format!("the desktop's mirror: {e}"))
                })
            }
            Err(refusal) => {
                let (_, body) = answered(refusal).await;
                Err(token_refusal(
                    &serde_json::to_vec(&body).expect("a refusal that is JSON"),
                ))
            }
        }
    }
}

/// The whole pairing, with the Hub's own code on the other side of every
/// request the desktop makes. Signing the `/token` proof with any key but the
/// one whose thumbprint went to `/authorize` as `dpop_jkt` fails it: the Hub
/// binds the code to that thumbprint.
#[tokio::test]
async fn the_desktop_pairs_through_the_hubs_own_consent_and_token_handlers() {
    let rp = RelyingParty {
        rp_id: "localhost".to_owned(),
        origin: HUB_ORIGIN.to_owned(),
    };
    let keys = Arc::new(MemoryKeys::with_keys([SIGNING_KEY]));
    let hub = lazy_state_with_keys("postgres://hub@db.invalid/hub", rp, keys, SIGNING_KEY)
        .expect("a parsable database URL");
    let user = SoftAuthenticator::new()
        .enrolled(&hub, Status::Trusted)
        .user_id;
    let passkeys = [SoftAuthenticator::new(), SoftAuthenticator::new()]
        .map(|authenticator| authenticator.enrolled(&hub, Status::Trusted));
    let token = Arc::new(HubToken {
        hub: hub.clone(),
        paired: Mutex::new(Some(Paired {
            instance_id: InstanceId::generate(),
            passkeys: passkeys.to_vec(),
        })),
        handed: Mutex::default(),
        answer: Mutex::default(),
    });

    let dir = demeteo_core::test_dir::TestDir::new("hub_desktop_full_pairing");
    let conn = rusqlite::Connection::open(dir.path().join("demeteo.db")).expect("a scratch DB");
    let db = Arc::new(SqliteAdapter::new(conn).expect("a migrated DB"));
    let secrets = Arc::new(MemorySecretStore::new(
        &db.install_id().expect("an install id"),
    ));
    let pairing = HubPairing::new(db.clone(), secrets, token.clone(), db.clone());
    let browser = ApprovingBrowser { hub, user };

    let paired = pairing
        .pair(HUB_ORIGIN, &browser, Duration::from_secs(10))
        .await;

    assert!(paired.is_ok(), "{paired:?}");
    assert_eq!(*mutex(&token.handed), [user]);
    let answer = mutex(&token.answer).take().expect("the Hub answered");
    let pins = db
        .load()
        .expect("readable pins")
        .expect("pins were written");
    assert_eq!(pins.hub_origin, HUB_ORIGIN);
    assert_eq!(
        json!(pins.request_signing_key),
        answer["request_signing_key"]
    );
    assert_eq!(json!(pins.rp_id), answer["rp_id"]);
    assert_eq!(json!(pins.instance_id), answer["instance_id"]);
    let pinned: Vec<Value> = pins
        .passkeys
        .iter()
        .map(|p| json!({ "credential_id": p.credential_id, "public_key": p.public_key }))
        .collect();
    assert_eq!(answer["passkeys"].as_array().map(Vec::len), Some(2));
    assert_eq!(json!(pinned), answer["passkeys"]);
}
