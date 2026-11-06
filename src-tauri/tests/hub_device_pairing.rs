#![cfg(target_os = "linux")]
//! A runner pairing with the Hub by device code (`docs/HUB.md` §4, *Pairing a
//! runner by device code*), asserted from the desktop package for the reason
//! `hub_pairing.rs` gives: a Demeteo run builds the Hub's tests from here and
//! nowhere else.
//!
//! The runner's two requests are sent to the real router where nothing past
//! them needs a database, and handed to the grant past its extractors where
//! the answer is an instance the router would write to Postgres. The approval
//! routes are their bodies, with the tenant's passkeys handed in.

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::ceremonies::Ceremony;
use demeteo_hub::auth::challenge::base64url_encode;
use demeteo_hub::auth::handlers::ApiError;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::session::{self, SessionSecret};
use demeteo_hub::auth::testing::{answered, lazy_state_with_keys, serve, SoftAuthenticator};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::handlers::{
    approved_device, denied_device, device_challenge, looked_up_device, polled, started_device,
    DeviceApproval, DeviceChallengeRequest, DeviceLookup, DevicePoll,
};
use demeteo_hub::pairing::jwk::fingerprint;
use demeteo_hub::pairing::testing::SoftDeviceKey;
use demeteo_hub::pairing::{
    DEVICE_CODE_TTL_SECS, DEVICE_POLL_INTERVAL_SECS, MAX_DEVICE_LOOKUP_MISSES,
};
use demeteo_hub::protocol::{Scope, WebAuthnAssertion};
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::store::instances::{InstanceKind, NewPairing, Paired};
use demeteo_hub::store::passkeys::Passkey;
use demeteo_hub::store::{InstanceId, StoreError, UserId};
use serde::Serialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::future::{ready, Ready};
use std::net::Ipv4Addr;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const HUB_ORIGIN: &str = "https://hub.example";
const SIGNING_KEY: &str = "demeteo-hub-signing";
const NOW: u64 = 1_700_000_000;

fn rp() -> RelyingParty {
    RelyingParty {
        rp_id: "hub.example".to_owned(),
        origin: HUB_ORIGIN.to_owned(),
    }
}

/// A Hub whose pool never connects and whose key service holds the signing
/// key. Must be made inside a Tokio runtime.
fn hub_state() -> AppState {
    let keys = Arc::new(MemoryKeys::with_keys([SIGNING_KEY]));
    lazy_state_with_keys("postgres://hub@db.invalid/hub", rp(), keys, SIGNING_KEY)
        .expect("a parsable database URL")
}

async fn serve_state(state: AppState) -> String {
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a free loopback port");
    let addr = listener.local_addr().expect("the bound address");
    serve(listener, routes::router(state, &web_dir));
    format!("http://{addr}")
}

fn wall_clock() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock set after 1970");
    since_epoch.as_secs()
}

fn device_url() -> String {
    format!("{HUB_ORIGIN}/device_authorization")
}

fn token_url() -> String {
    format!("{HUB_ORIGIN}/token")
}

/// What a runner sends to start: `read` and `gates`, and its labels.
fn runner_form() -> Value {
    json!({
        "scope": "read gates",
        "instance_name": "build-01",
        "os": "linux",
        "arch": "x86_64",
        "app_version": "1.4.0",
    })
}

fn form_body(members: &Value) -> String {
    let Value::Object(members) = members else {
        panic!("{members} is not an object");
    };
    let encoded = |value: &str| -> String {
        value
            .bytes()
            .map(|byte| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    char::from(byte).to_string()
                }
                _ => format!("%{byte:02X}"),
            })
            .collect()
    };
    let pairs: Vec<String> = members
        .iter()
        .filter_map(|(name, value)| Some(format!("{name}={}", encoded(value.as_str()?))))
        .collect();
    pairs.join("&")
}

struct Answer {
    outcome: (u16, Value),
    cache_control: Option<String>,
}

async fn post_form(base: &str, path: &str, proofs: &[&str], form: &Value) -> Answer {
    let mut request = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form_body(form));
    for proof in proofs {
        request = request.header("dpop", *proof);
    }
    let response = request.send().await.expect("the loopback listener answers");
    let cache_control = response
        .headers()
        .get("cache-control")
        .map(|value| value.to_str().expect("an ASCII header").to_owned());
    let status = response.status().as_u16();
    let body = response.text().await.expect("a readable body");
    Answer {
        outcome: (status, serde_json::from_str(&body).unwrap_or(Value::Null)),
        cache_control,
    }
}

async fn sent<T: Serialize>(outcome: Result<T, ApiError>) -> (u16, Value) {
    match outcome {
        Ok(answer) => (
            200,
            serde_json::to_value(answer).expect("an answer that is JSON"),
        ),
        Err(refusal) => answered(refusal).await,
    }
}

fn refused(code: &str) -> (u16, Value) {
    (400, json!({ "error": code }))
}

/// `POST /device_authorization` past its extractors, by `key` at `now`: the
/// device code and the user code it answered.
fn start(state: &AppState, key: &SoftDeviceKey, now: u64, jti: &str) -> (String, String) {
    let form = serde_json::from_value(runner_form()).expect("a device form");
    let proof = key.proof("POST", &device_url(), now, jti);
    let started = started_device(state, &form, Some(&proof), now).expect("a started pairing");
    let started = serde_json::to_value(started).expect("an answer that is JSON");
    let text = |member: &str| started[member].as_str().expect("a string").to_owned();
    (text("device_code"), text("user_code"))
}

type Handed = Rc<RefCell<Vec<(UserId, NewPairing)>>>;

fn recording(
    handed: Handed,
    paired: Paired,
) -> impl FnOnce(UserId, NewPairing) -> Ready<Result<Paired, StoreError>> {
    move |user, new| {
        handed.borrow_mut().push((user, new));
        ready(Ok(paired))
    }
}

/// One device-code poll by `key` at `now`, to a store that records what it
/// is handed and answers an instance of a tenant holding `passkeys`.
async fn poll(
    state: &AppState,
    key: &SoftDeviceKey,
    device_code: &str,
    now: u64,
    jti: &str,
    handed: &Handed,
    paired: Paired,
) -> (u16, Value) {
    let proof = key.proof("POST", &token_url(), now, jti);
    let pair = recording(handed.clone(), paired);
    sent(polled(state, DevicePoll::new(device_code), Some(&proof), now, pair).await).await
}

async fn poll_at(
    state: &AppState,
    key: &SoftDeviceKey,
    device_code: &str,
    now: u64,
) -> (u16, Value) {
    let paired = Paired {
        instance_id: InstanceId::generate(),
        passkeys: Vec::new(),
    };
    let jti = format!("poll-{now}");
    poll(
        state,
        key,
        device_code,
        now,
        &jti,
        &Handed::default(),
        paired,
    )
    .await
}

/// A tenant of one trusted passkey, and that passkey's row.
fn tenant(state: &AppState) -> (SoftAuthenticator, Passkey) {
    let authenticator = SoftAuthenticator::new();
    let passkey = authenticator.enrolled(state, Status::Trusted);
    (authenticator, passkey)
}

fn holder(passkey: &Passkey) -> Vec<u8> {
    passkey.credential_id.clone()
}

/// A challenge held for `passkey`'s session over `user_code`, approving
/// `scopes`: its ceremony id and the digest a passkey is asked to sign.
fn challenged(
    state: &AppState,
    passkey: &Passkey,
    user_code: &str,
    scopes: Vec<Scope>,
    now: u64,
) -> (String, [u8; 32]) {
    let request = DeviceChallengeRequest::new(user_code, scopes);
    let challenge = device_challenge(
        state,
        passkey.user_id,
        holder(passkey),
        request,
        std::slice::from_ref(passkey),
        now,
    )
    .expect("a challenge");
    let digest = challenge.challenge().expect("options carrying a challenge");
    (challenge.ceremony_id().to_owned(), digest)
}

async fn approve(
    state: &AppState,
    passkey: &Passkey,
    ceremony_id: &str,
    assertion: WebAuthnAssertion,
    now: u64,
) -> (u16, Value) {
    let approval = DeviceApproval::new(ceremony_id, assertion);
    let passkeys = std::slice::from_ref(passkey);
    let answer = approved_device(
        state,
        passkey.user_id,
        &passkey.credential_id,
        approval,
        passkeys,
        now,
    );
    sent(answer).await
}

#[tokio::test]
async fn a_runner_is_paired_once_a_passkey_approves_and_is_handed_the_pairing_response() {
    let state = hub_state();
    let key = SoftDeviceKey::new();
    let (authenticator, passkey) = tenant(&state);
    let (device_code, user_code) = start(&state, &key, NOW, "start");

    assert_eq!(
        poll_at(&state, &key, &device_code, NOW).await,
        refused("authorization_pending")
    );

    let (status, prompt) = sent(looked_up_device(
        &state,
        passkey.user_id,
        DeviceLookup::new(&user_code.to_lowercase()),
        NOW + 1,
    ))
    .await;
    assert_eq!(
        (status, prompt),
        (
            200,
            json!({
                "user_code": user_code,
                "fingerprint": fingerprint(&key.thumbprint()),
                "instance_name": "build-01",
                "os": "linux",
                "arch": "x86_64",
                "app_version": "1.4.0",
                "scopes": ["read", "gates"],
                "expires_at": NOW + DEVICE_CODE_TTL_SECS,
            })
        )
    );

    let (ceremony_id, challenge) =
        challenged(&state, &passkey, &user_code, vec![Scope::Gates], NOW + 2);
    let assertion = authenticator.assert(&rp(), &challenge);
    assert_eq!(
        approve(&state, &passkey, &ceremony_id, assertion, NOW + 3).await,
        (200, json!({ "answer": "approved" }))
    );

    let handed = Handed::default();
    let instance_id = InstanceId::generate();
    let paired = Paired {
        instance_id,
        passkeys: vec![passkey.clone()],
    };
    let at = NOW + DEVICE_POLL_INTERVAL_SECS;
    let (status, response) = poll(&state, &key, &device_code, at, "paired", &handed, paired).await;

    assert_eq!(status, 200, "{response}");
    assert_eq!(response["instance_id"], json!(instance_id.0.to_string()));
    assert_eq!(response["scope"], json!("gates"));
    assert_eq!(response["token_type"], json!("DPoP"));
    assert_eq!(response["rp_id"], json!("hub.example"));
    assert_eq!(
        response["passkeys"],
        json!([{
            "credential_id": base64url_encode(&passkey.credential_id),
            "public_key": base64url_encode(&passkey.public_key),
        }])
    );
    let handed = handed.take();
    let [(user, new)] = handed.as_slice() else {
        panic!("the store was handed {} pairings", handed.len());
    };
    assert_eq!(*user, passkey.user_id);
    assert_eq!(new.kind, InstanceKind::Runner);
    assert_eq!(new.scopes, [Scope::Gates]);
    assert_eq!(new.device_key.jkt, key.thumbprint());
    assert_eq!(new.instance.name, "build-01");

    let again = poll_at(&state, &key, &device_code, at + DEVICE_POLL_INTERVAL_SECS).await;
    assert_eq!(again, refused("invalid_grant"));
}

#[tokio::test]
async fn a_poll_sooner_than_the_interval_after_the_last_is_slow_down() {
    let state = hub_state();
    let key = SoftDeviceKey::new();
    let (device_code, _) = start(&state, &key, NOW, "start");
    assert_eq!(
        poll_at(&state, &key, &device_code, NOW).await,
        refused("authorization_pending")
    );

    let early = NOW + DEVICE_POLL_INTERVAL_SECS - 1;
    assert_eq!(
        poll_at(&state, &key, &device_code, early).await,
        refused("slow_down")
    );

    let waited = early + DEVICE_POLL_INTERVAL_SECS;
    assert_eq!(
        poll_at(&state, &key, &device_code, waited).await,
        refused("authorization_pending")
    );
}

#[tokio::test]
async fn a_denial_is_access_denied_once_and_then_the_code_is_spent() {
    let state = hub_state();
    let key = SoftDeviceKey::new();
    let (_, passkey) = tenant(&state);
    let (device_code, user_code) = start(&state, &key, NOW, "start");

    let (status, answer) = sent(denied_device(
        &state,
        passkey.user_id,
        DeviceLookup::new(&user_code),
        NOW + 1,
    ))
    .await;
    assert_eq!((status, answer), (200, json!({ "answer": "denied" })));

    assert_eq!(
        poll_at(&state, &key, &device_code, NOW + 2).await,
        refused("access_denied")
    );
    let later = NOW + 2 + DEVICE_POLL_INTERVAL_SECS;
    assert_eq!(
        poll_at(&state, &key, &device_code, later).await,
        refused("invalid_grant")
    );
}

#[tokio::test]
async fn a_pairing_left_unanswered_for_ten_minutes_is_expired_token() {
    assert_eq!(DEVICE_CODE_TTL_SECS, 600);
    let state = hub_state();
    let key = SoftDeviceKey::new();
    let (device_code, _) = start(&state, &key, NOW, "start");

    let last = NOW + DEVICE_CODE_TTL_SECS;
    assert_eq!(
        poll_at(&state, &key, &device_code, last).await,
        refused("authorization_pending")
    );
    let past = last + DEVICE_POLL_INTERVAL_SECS;
    assert_eq!(
        poll_at(&state, &key, &device_code, past).await,
        refused("expired_token")
    );
}

#[tokio::test]
async fn a_poll_by_a_key_other_than_the_one_that_started_is_invalid_grant() {
    let state = hub_state();
    let (key, another) = (SoftDeviceKey::new(), SoftDeviceKey::new());
    let (authenticator, passkey) = tenant(&state);
    let (device_code, user_code) = start(&state, &key, NOW, "start");
    let (ceremony_id, challenge) = challenged(&state, &passkey, &user_code, vec![Scope::Read], NOW);
    let assertion = authenticator.assert(&rp(), &challenge);
    assert_eq!(
        approve(&state, &passkey, &ceremony_id, assertion, NOW)
            .await
            .0,
        200
    );

    assert_eq!(
        poll_at(&state, &another, &device_code, NOW + 1).await,
        refused("invalid_grant")
    );
    assert_eq!(
        poll_at(&state, &another, &device_code, NOW + 2).await,
        refused("invalid_grant")
    );

    let (status, response) = poll_at(&state, &key, &device_code, NOW + 3).await;
    assert_eq!(status, 200, "{response}");
}

#[tokio::test]
async fn an_approval_without_a_verifying_assertion_is_refused_and_leaves_the_pairing_pending() {
    let state = hub_state();
    let key = SoftDeviceKey::new();
    let (authenticator, passkey) = tenant(&state);
    let (device_code, user_code) = start(&state, &key, NOW, "start");
    let challenge_for = |scopes| challenged(&state, &passkey, &user_code, scopes, NOW);

    let without: Result<DeviceApproval, _> =
        serde_json::from_value(json!({ "ceremony_id": challenge_for(vec![Scope::Read]).0 }));
    assert!(without.is_err());

    let (ceremony_id, _) = challenge_for(vec![Scope::Read]);
    let over_another = authenticator.assert(&rp(), &[7; 32]);
    assert_eq!(
        approve(&state, &passkey, &ceremony_id, over_another, NOW).await,
        (403, json!({ "error": "assertion_invalid" }))
    );

    let (ceremony_id, challenge) = challenge_for(vec![Scope::Read]);
    let stranger = SoftAuthenticator::new();
    assert_eq!(
        approve(
            &state,
            &passkey,
            &ceremony_id,
            stranger.assert(&rp(), &challenge),
            NOW
        )
        .await,
        (403, json!({ "error": "assertion_invalid" }))
    );

    let (ceremony_id, challenge) = challenge_for(vec![Scope::Read]);
    let unverified = authenticator.assert_with("hub.example", HUB_ORIGIN, 0x01, &challenge);
    assert_eq!(
        approve(&state, &passkey, &ceremony_id, unverified, NOW).await,
        (403, json!({ "error": "assertion_invalid" }))
    );

    let (ceremony_id, challenge) = challenge_for(vec![Scope::Read]);
    let good = authenticator.assert(&rp(), &challenge);
    assert_eq!(
        approve(&state, &passkey, &ceremony_id, good.clone(), NOW)
            .await
            .0,
        200
    );
    assert_eq!(
        approve(&state, &passkey, &ceremony_id, good, NOW).await,
        refused("ceremony_expired")
    );
    let (status, _) = poll_at(&state, &key, &device_code, NOW + 1).await;
    assert_eq!(status, 200);
}

/// What the challenge was computed over is held with it, so the approval
/// lands on that key with those scopes whatever the finish says; and a
/// challenge is never held for a scope the runner did not ask for.
#[tokio::test]
async fn the_challenge_is_held_for_the_device_key_and_the_scopes_approved() {
    let state = hub_state();
    let key = SoftDeviceKey::new();
    let (_, passkey) = tenant(&state);
    let (_, user_code) = start(&state, &key, NOW, "start");

    let (ceremony_id, challenge) =
        challenged(&state, &passkey, &user_code, vec![Scope::Gates], NOW);

    let Some(Ceremony::Device {
        dpop_jkt,
        scopes,
        challenge: held,
        ..
    }) = state.ceremonies().take(NOW, &ceremony_id)
    else {
        panic!("no device ceremony was held");
    };
    assert_eq!(dpop_jkt, key.thumbprint());
    assert_eq!(scopes, [Scope::Gates]);
    assert_eq!(held, challenge);

    let widened = DeviceChallengeRequest::new(&user_code, vec![Scope::Spend]);
    let passkeys = std::slice::from_ref(&passkey);
    let refusal = device_challenge(
        &state,
        passkey.user_id,
        holder(&passkey),
        widened,
        passkeys,
        NOW,
    );
    assert_eq!(sent(refusal).await, refused("invalid_scope"));
}

#[tokio::test]
async fn a_tenant_whose_lookups_miss_too_often_is_refused_even_the_right_code() {
    let state = hub_state();
    let key = SoftDeviceKey::new();
    let (_, user_code) = start(&state, &key, NOW, "start");
    let (guesser, other) = (tenant(&state).1.user_id, tenant(&state).1.user_id);
    let look =
        |user, code: &str| sent(looked_up_device(&state, user, DeviceLookup::new(code), NOW));

    assert_eq!(
        look(guesser, "not a code").await,
        refused("invalid_request")
    );
    for _ in 0..MAX_DEVICE_LOOKUP_MISSES {
        assert_eq!(look(guesser, "ZZZZ-ZZZZ").await, refused("pairing_expired"));
    }

    assert_eq!(
        look(guesser, &user_code).await,
        (429, json!({ "error": "too_many_lookups" }))
    );
    assert_eq!(look(other, &user_code).await.0, 200);
}

/// The route itself: a form and one proof for this URL, answered `no-store`
/// with the six members of the contract.
#[tokio::test]
async fn device_authorization_answers_the_codes_and_where_to_approve_them() {
    let state = hub_state();
    let base = serve_state(state.clone()).await;
    let key = SoftDeviceKey::new();
    let proof = key.proof("POST", &device_url(), wall_clock(), "route");

    let answer = post_form(&base, "/device_authorization", &[&proof], &runner_form()).await;

    let (status, body) = answer.outcome;
    assert_eq!(status, 200, "{body}");
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"));
    let user_code = body["user_code"].as_str().expect("a user code").to_owned();
    let device_code = body["device_code"].as_str().expect("a device code");
    assert_eq!(device_code.len(), 43);
    let (first, second) = user_code.split_once('-').expect("two groups");
    for group in [first, second] {
        assert_eq!(group.len(), 4);
        assert!(
            group.bytes().all(|b| b"BCDFGHJKLMNPQRSTVWXZ".contains(&b)),
            "{user_code}"
        );
    }
    assert_eq!(
        body,
        json!({
            "device_code": device_code,
            "user_code": user_code,
            "verification_uri": format!("{HUB_ORIGIN}/#/device"),
            "verification_uri_complete": format!("{HUB_ORIGIN}/#/device/{user_code}"),
            "expires_in": 600,
            "interval": 5,
        })
    );

    let poll = json!({
        "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        "device_code": device_code,
    });
    let pending = key.proof("POST", &token_url(), wall_clock(), "poll-1");
    let polled = post_form(&base, "/token", &[&pending], &poll).await;
    assert_eq!(polled.outcome, refused("authorization_pending"));
    assert_eq!(polled.cache_control.as_deref(), Some("no-store"));
    let soon = key.proof("POST", &token_url(), wall_clock(), "poll-2");
    let polled = post_form(&base, "/token", &[&soon], &poll).await;
    assert_eq!(polled.outcome, refused("slow_down"));
}

#[tokio::test]
async fn device_authorization_refuses_a_bad_proof_or_form_and_starts_nothing() {
    let state = hub_state();
    let base = serve_state(state.clone()).await;
    let key = SoftDeviceKey::new();
    let at = wall_clock();
    let proof = |htu: &str, jti: &str| key.proof("POST", htu, at, jti);

    let cases = [
        (vec![], runner_form(), "invalid_dpop_proof"),
        (
            vec![proof(&token_url(), "for-token")],
            runner_form(),
            "invalid_dpop_proof",
        ),
        (
            vec![proof(&device_url(), "two-a"), proof(&device_url(), "two-b")],
            runner_form(),
            "invalid_dpop_proof",
        ),
        (
            vec![proof(&device_url(), "scope")],
            json!({ "scope": "read admin", "instance_name": "a", "os": "a", "arch": "a", "app_version": "a" }),
            "invalid_request",
        ),
        (
            vec![proof(&device_url(), "label")],
            json!({ "scope": "read", "os": "a", "arch": "a", "app_version": "a" }),
            "invalid_request",
        ),
    ];
    for (proofs, form, error) in cases {
        let proofs: Vec<&str> = proofs.iter().map(String::as_str).collect();
        let answer = post_form(&base, "/device_authorization", &proofs, &form).await;
        assert_eq!(
            answer.outcome,
            refused(error),
            "{form} with {} proofs",
            proofs.len()
        );
        assert_eq!(answer.cache_control.as_deref(), Some("no-store"));
    }

    let accepted = post_form(
        &base,
        "/device_authorization",
        &[&proof(&device_url(), "ok")],
        &runner_form(),
    )
    .await;
    assert_eq!(accepted.outcome.0, 200);
    let replayed = post_form(
        &base,
        "/device_authorization",
        &[&proof(&device_url(), "ok")],
        &runner_form(),
    )
    .await;
    assert_eq!(replayed.outcome, refused("invalid_dpop_proof"));
}

/// The approval routes are behind the session like every `/v1` route: the
/// `401` is the extractor's, before the pool that cannot connect is asked.
#[tokio::test]
async fn the_device_approval_routes_answer_401_without_a_session() {
    let base = serve_state(hub_state()).await;
    let token = SessionSecret::generate()
        .expect("the system random source")
        .csrf_token();
    for path in ["lookup", "challenge", "approve", "deny"] {
        let response = reqwest::Client::new()
            .post(format!("{base}/v1/pairing/device/{path}"))
            .header("origin", HUB_ORIGIN)
            .header(session::CSRF_HEADER, token.as_str())
            .send()
            .await
            .expect("the loopback listener answers");
        let status = response.status().as_u16();
        let body = response.text().await.expect("a readable body");
        let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        assert_eq!(
            (status, body),
            (401, json!({ "error": "unauthenticated" })),
            "{path}"
        );
    }
}
