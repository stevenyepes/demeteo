#![cfg(target_os = "linux")]
//! What the Hub demands of a desktop that pairs with it, asserted from the
//! desktop package.
//!
//! A Demeteo run validates Rust with `cd src-tauri && cargo test` and nothing
//! wider. That command builds `demeteo-hub` because this package lists it as a
//! Linux dev-dependency, and then runs this package's tests only, so a unit
//! test beside the Hub's pairing code is compiled by nobody and passes for
//! nobody during a run. Each pairing rule a run must be able to watch hold is
//! therefore driven from this file, through the Hub's public API and without
//! a database.

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::auth::challenge::{base64url_decode, base64url_encode};
use demeteo_hub::auth::handlers::ApiError;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::session::{self, SessionSecret};
use demeteo_hub::auth::testing::{
    answered, lazy_state, lazy_state_with_keys, serve, SoftAuthenticator,
};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::authorize::{self, AuthorizeParams, AuthorizeRefusal, AuthorizeRequest};
use demeteo_hub::pairing::grant::{
    access_verdict, refresh_change, refresh_response, AccessRefusal, AuthorizationCode,
    FamilyChange, FreshPair, RefreshDecision, RefreshRefusal, TokenSecret,
};
use demeteo_hub::pairing::handlers::{
    approved_pairing, denied_pairing, redeemed, refreshed, started_pairing, Rotation,
};
use demeteo_hub::pairing::jwk::{fingerprint, DeviceJwk};
use demeteo_hub::pairing::pending::ApprovedPairing;
use demeteo_hub::pairing::testing::{claims, verifies_ed25519, MemoryFamily, SoftDeviceKey};
use demeteo_hub::pairing::{
    pkce, ACCESS_TOKEN_TTL_SECS, AUTHORIZATION_CODE_TTL_SECS, CONSENT_TTL_SECS, DPOP_MAX_AGE_SECS,
    REFRESH_TOKEN_TTL_SECS,
};
use demeteo_hub::ports::KeyService;
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::store::instances::{NewPairing, Paired};
use demeteo_hub::store::passkeys::Passkey;
use demeteo_hub::store::tokens::{FamilyWritten, NewToken, TokenHash, TokenKind, TokenRecord};
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

/// RFC 7636 Appendix B. The challenge was not produced by the code under
/// test: it is the RFC's own, and was recomputed outside this crate with
/// Python's `hashlib` and `base64` over the verifier's ASCII bytes.
const RFC_7636_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const RFC_7636_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

/// `text` with the character at `position` replaced by another of the
/// base64url alphabet, which is inside PKCE's unreserved set: what makes the
/// result wrong is its value, never its length or its alphabet.
fn with_one_character_changed(text: &str, position: usize) -> String {
    let mut bytes = text.as_bytes().to_vec();
    bytes[position] = if bytes[position] == b'A' { b'B' } else { b'A' };
    String::from_utf8(bytes).expect("an ASCII string with one ASCII byte replaced")
}

#[test]
fn the_rfc_7636_verifier_matches_its_challenge_and_a_one_character_change_to_either_fails() {
    assert!(pkce::verifier_matches(
        RFC_7636_VERIFIER,
        RFC_7636_CHALLENGE
    ));

    for position in 0..RFC_7636_VERIFIER.len() {
        let verifier = with_one_character_changed(RFC_7636_VERIFIER, position);
        assert_ne!(verifier, RFC_7636_VERIFIER);
        assert!(
            !pkce::verifier_matches(&verifier, RFC_7636_CHALLENGE),
            "verifier changed at {position}: {verifier}"
        );
    }
    for position in 0..RFC_7636_CHALLENGE.len() {
        let challenge = with_one_character_changed(RFC_7636_CHALLENGE, position);
        assert_ne!(challenge, RFC_7636_CHALLENGE);
        assert!(
            !pkce::verifier_matches(RFC_7636_VERIFIER, &challenge),
            "challenge changed at {position}: {challenge}"
        );
    }
}

const RFC_9449_THUMBPRINT: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";

/// The key of the example proof in RFC 9449 §4.1, with its members in the
/// order the RFC prints them. Neither expected string was produced by the
/// code under test: both were computed outside this crate, with Python's
/// `hashlib` and `base64` over the RFC 7638 text
/// `{"crv":"P-256","kty":"EC","x":"<x>","y":"<y>"}` typed by hand.
#[test]
fn the_thumbprint_and_fingerprint_match_a_published_key() {
    let published = serde_json::json!({
        "kty": "EC",
        "x": "l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs",
        "y": "9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA",
        "crv": "P-256",
    });
    let key = DeviceJwk::parse(&published).expect("the RFC 9449 example key is an ES256 JWK");

    let thumbprint = key.thumbprint();
    assert_eq!(thumbprint, RFC_9449_THUMBPRINT);
    assert_eq!(fingerprint(&thumbprint), "SHA256:0ZcO CORZ NYy- DWpq");
}

const REDIRECT: &str = "http://127.0.0.1:8080/cb";

/// What a desktop asks of `/authorize` when the Hub has nothing to refuse:
/// each refusal below is of this request with one parameter changed.
fn pairing_request() -> AuthorizeParams {
    let text = |value: &str| Some(value.to_owned());
    AuthorizeParams {
        response_type: text("code"),
        code_challenge_method: text("S256"),
        code_challenge: text(RFC_7636_CHALLENGE),
        redirect_uri: text(REDIRECT),
        dpop_jkt: text(RFC_9449_THUMBPRINT),
        scope: text("read gates"),
        state: text("xyz-123"),
        instance_name: text("Workstation"),
        os: text("linux"),
        arch: text("x86_64"),
        app_version: text("1.4.0"),
    }
}

fn redirecting_to(redirect_uri: &str) -> AuthorizeParams {
    AuthorizeParams {
        redirect_uri: Some(redirect_uri.to_owned()),
        ..pairing_request()
    }
}

const HUB_ORIGIN: &str = "https://hub.example";

/// A Hub whose pool never connects. Must be made inside a Tokio runtime, which
/// is the only reason a test of the synchronous consent bodies is `async`.
fn hub_state() -> AppState {
    let rp = RelyingParty {
        rp_id: "hub.example".to_owned(),
        origin: HUB_ORIGIN.to_owned(),
    };
    lazy_state("postgres://hub@db.invalid/hub", rp).expect("a parsable database URL")
}

/// `routes::router` over `state` on a loopback port, and the base URL it
/// answers at. A clone of `state` kept by the caller shares everything the
/// router holds, so a code parked through it is one the router finds. The
/// web directory is absent, and no route asked here reads it.
async fn serve_state(state: AppState) -> String {
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a free loopback port");
    let addr = listener.local_addr().expect("the bound address");
    serve(listener, routes::router(state, &web_dir));
    format!("http://{addr}")
}

/// [`serve_state`] over a Hub nothing is asked of but its routes.
async fn serve_hub() -> String {
    serve_state(hub_state()).await
}

/// Every byte outside RFC 3986's unreserved set as `%XX`. The near misses are
/// made of query syntax: left as it is, a `#` ends the URL before the client
/// sends it and a `?`, `&`, `+` or space is read as something else by the
/// Hub, which would then be refusing, or accepting, a value no test named.
fn percent_encoded(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The string members of `members` as `name=value` pairs, each value
/// percent-encoded: a query, or the body of a form. `reqwest` is built here
/// without the features that would write either.
fn urlencoded(members: &Value) -> String {
    let Value::Object(members) = members else {
        panic!("{members} is not an object");
    };
    let pairs: Vec<String> = members
        .iter()
        .filter_map(|(name, value)| Some(format!("{name}={}", percent_encoded(value.as_str()?))))
        .collect();
    pairs.join("&")
}

/// `params` as the query a desktop would send, an absent parameter left out.
/// It is written from the same value `validate` is handed, so the router and
/// the function are asked about one request.
fn query_of(params: &AuthorizeParams) -> String {
    urlencoded(&serde_json::to_value(params).expect("parameters that are JSON"))
}

struct Answer {
    /// The status, and the body as JSON; `Null` when the body is not JSON.
    outcome: (u16, Value),
    location: Option<String>,
    cache_control: Option<String>,
}

/// One `GET /authorize?<query>` and what came back for it. The client follows
/// no redirect: followed, the `303` would be read as whatever the static
/// fallback answers for `/`.
async fn answer_to(base: &str, query: &str) -> Answer {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("a client that follows no redirect");
    let response = client
        .get(format!("{base}/authorize?{query}"))
        .send()
        .await
        .expect("the loopback listener answers");
    answer_of(response).await
}

async fn answer_of(response: reqwest::Response) -> Answer {
    let header = |name: &str| {
        let value = response.headers().get(name)?;
        Some(value.to_str().expect("an ASCII header").to_owned())
    };
    let (location, cache_control) = (header("location"), header("cache-control"));
    let status = response.status().as_u16();
    let body = response.text().await.expect("a readable body");
    Answer {
        outcome: (status, serde_json::from_str(&body).unwrap_or(Value::Null)),
        location,
        cache_control,
    }
}

async fn authorize_on(base: &str, params: &AuthorizeParams) -> Answer {
    answer_to(base, &query_of(params)).await
}

fn invalid_request() -> (u16, Value) {
    (400, json!({ "error": "invalid_request" }))
}

/// The positive case every refusal is measured against: `params` is sent on
/// to the approval page, in a segment that reads back as what was asked.
async fn assert_sent_to_the_approval_page(base: &str, params: &AuthorizeParams) {
    let asked = authorize::validate(params);
    assert!(asked.is_ok(), "{params:?}: {asked:?}");

    let answer = authorize_on(base, params).await;
    assert_eq!(answer.outcome.0, 303, "{params:?}");
    let location = answer.location.expect("a 303 says where to go");
    let segment = location
        .strip_prefix("/#/pair/")
        .unwrap_or_else(|| panic!("`{location}` is not the approval page"));
    assert_eq!(authorize::decode(segment), asked);
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"));
}

#[tokio::test]
async fn authorize_refuses_a_request_whose_challenge_method_is_not_s256() {
    let base = serve_hub().await;

    let with_s256 = pairing_request();
    assert_eq!(with_s256.code_challenge_method.as_deref(), Some("S256"));
    assert_sent_to_the_approval_page(&base, &with_s256).await;

    for method in [None, Some("plain")] {
        let params = AuthorizeParams {
            code_challenge_method: method.map(str::to_owned),
            ..pairing_request()
        };
        assert_eq!(
            authorize::validate(&params),
            Err(AuthorizeRefusal::ChallengeMethod),
            "{method:?}"
        );
        let answer = authorize_on(&base, &params).await;
        assert_eq!(answer.outcome, invalid_request(), "{method:?}");
        assert_eq!(answer.location, None, "{method:?}");
    }
}

#[tokio::test]
async fn authorize_refuses_every_redirect_that_is_not_loopback_http_with_a_port() {
    let base = serve_hub().await;

    for uri in [
        REDIRECT,
        "http://127.0.0.1:1/",
        "http://127.0.0.1:65535/oauth/callback",
    ] {
        assert_sent_to_the_approval_page(&base, &redirecting_to(uri)).await;
    }

    let near_misses = [
        "https://127.0.0.1:8080/cb",
        "HTTP://127.0.0.1:8080/cb",
        "http://localhost:8080/cb",
        "http://127.0.0.1/cb",
        "http://127.0.0.1:8080",
        "http://127.0.0.1:0/cb",
        "http://127.0.0.1:65536/cb",
        "http://127.0.0.1:08080/cb",
        "http://127.1:8080/cb",
        "http://127.0.0.2:8080/cb",
        "http://2130706433:8080/cb",
        "http://0.0.0.0:8080/cb",
        "http://[::1]:8080/cb",
        "http://127.0.0.1.evil.example:8080/cb",
        "http://127.0.0.1:8080@evil.example/cb",
        "http://evil.example@127.0.0.1:8080/cb",
        "http://evil.example:8080/cb",
        "http://127.0.0.1:8080/cb?x=1",
        "http://127.0.0.1:8080/cb#f",
        "http://127.0.0.1:8080/c b",
        " http://127.0.0.1:8080/cb",
        "http://127.0.0.1:8080\\cb",
        "http://127.0.0.1:8080/%2e%2e",
        "demeteo://callback",
    ];
    for uri in near_misses {
        let params = redirecting_to(uri);
        assert_eq!(
            authorize::validate(&params),
            Err(AuthorizeRefusal::RedirectUri),
            "{uri:?}"
        );
        let answer = authorize_on(&base, &params).await;
        assert_eq!(answer.outcome, invalid_request(), "{uri:?}");
        assert_eq!(answer.location, None, "{uri:?}");
    }
}

/// A repeated parameter never reaches `validate`: the query does not read as
/// one request, and axum's extractor refuses it in prose of its own.
#[tokio::test]
async fn authorize_answers_a_parameter_sent_twice_as_it_answers_every_other_refusal() {
    let base = serve_hub().await;
    let params = pairing_request();
    assert_sent_to_the_approval_page(&base, &params).await;

    let answer = answer_to(&base, &format!("{}&state=xyz-123", query_of(&params))).await;

    assert_eq!(answer.outcome, invalid_request());
    assert_eq!(answer.location, None);
}

/// When the pairings below are granted, in Unix seconds.
const NOW: u64 = 1_700_000_000;

fn fresh_pair() -> FreshPair {
    FreshPair::generate().expect("the system random source")
}

/// One `grant_type=refresh_token` request as the store sees it: `refresh`
/// presented at `now` with a proof by the key whose thumbprint is `jkt`, the
/// decision taken over the family's rows, and its change written whatever
/// the verdict. A rotation would issue `next`.
fn present(
    family: &mut MemoryFamily,
    refresh: &TokenSecret,
    jkt: &str,
    now: u64,
    next: &FreshPair,
) -> RefreshDecision {
    let decision = refresh_change(family.rows(), &refresh.hash(), jkt, now, next)
        .expect("an expiry within the range of `time`");
    family.apply(&decision.change, now);
    decision
}

fn row_of<'a>(family: &'a MemoryFamily, token: &TokenSecret) -> &'a TokenRecord {
    family
        .find(&token.hash())
        .expect("a row for a token the family was issued")
}

fn live_rows(family: &MemoryFamily) -> usize {
    let live = |row: &&TokenRecord| row.retired_at.is_none();
    family.rows().iter().filter(live).count()
}

#[test]
fn a_refresh_token_used_twice_revokes_its_family_and_the_newest_token_then_fails() {
    let jkt = SoftDeviceKey::new().thumbprint();
    let first = fresh_pair();
    let mut family = MemoryFamily::paired(&first, &jkt, NOW);

    let second = fresh_pair();
    let rotated = present(&mut family, &first.refresh, &jkt, NOW + 60, &second);

    assert_eq!(rotated.verdict, Ok(()));
    assert_eq!(rotated.change.retire.len(), 1);
    let r1 = row_of(&family, &first.refresh);
    assert!(r1.retired_at.is_some(), "{r1:?}");
    let a2 = row_of(&family, &second.access);
    let r2 = row_of(&family, &second.refresh);
    assert_eq!(a2.kind, TokenKind::Access);
    assert_eq!(r2.kind, TokenKind::Refresh);
    for issued in [a2, r2] {
        assert_eq!(issued.family_id, r1.family_id);
        assert_eq!(issued.jkt, jkt);
        assert_eq!(issued.retired_at, None);
    }
    assert_eq!(access_verdict(a2, &jkt, NOW + 60), Ok(()));
    assert_eq!(family.rows().len(), 4);
    assert_eq!(live_rows(&family), 3);

    let third = fresh_pair();
    let reused = present(&mut family, &first.refresh, &jkt, NOW + 120, &third);

    assert_eq!(reused.verdict, Err(RefreshRefusal::Reused));
    assert_eq!(reused.change.retire.len(), 3);
    assert!(reused.change.insert.is_empty(), "{reused:?}");
    assert_eq!(live_rows(&family), 0);
    assert_eq!(family.rows().len(), 4);

    let fourth = fresh_pair();
    let newest = present(&mut family, &second.refresh, &jkt, NOW + 180, &fourth);

    assert_eq!(newest.verdict, Err(RefreshRefusal::Reused));
    assert_eq!(family.rows().len(), 4);
    assert_eq!(live_rows(&family), 0);
    assert_eq!(
        access_verdict(row_of(&family, &second.access), &jkt, NOW + 180),
        Err(AccessRefusal::Retired)
    );
}

#[test]
fn a_refresh_with_another_keys_proof_or_after_its_expiry_is_refused_and_retires_nothing() {
    let jkt = SoftDeviceKey::new().thumbprint();
    let other_jkt = SoftDeviceKey::new().thumbprint();
    assert_ne!(jkt, other_jkt);
    let first = fresh_pair();
    let mut family = MemoryFamily::paired(&first, &jkt, NOW);
    let next = fresh_pair();

    let by_another_key = present(&mut family, &first.refresh, &other_jkt, NOW + 60, &next);

    assert_eq!(by_another_key.verdict, Err(RefreshRefusal::KeyMismatch));
    assert!(
        by_another_key.change.retire.is_empty(),
        "{by_another_key:?}"
    );
    assert!(
        by_another_key.change.insert.is_empty(),
        "{by_another_key:?}"
    );
    assert_eq!(family.rows().len(), 2);
    assert_eq!(live_rows(&family), 2);

    let expires = NOW + REFRESH_TOKEN_TTL_SECS;
    let too_late = present(&mut family, &first.refresh, &jkt, expires, &next);

    assert_eq!(too_late.verdict, Err(RefreshRefusal::Expired));
    assert!(too_late.change.retire.is_empty(), "{too_late:?}");
    assert!(too_late.change.insert.is_empty(), "{too_late:?}");
    assert_eq!(family.rows().len(), 2);
    assert_eq!(live_rows(&family), 2);
    assert_eq!(row_of(&family, &first.refresh).retired_at, None);

    let in_time = present(&mut family, &first.refresh, &jkt, expires - 1, &next);

    assert_eq!(in_time.verdict, Ok(()));
    assert!(row_of(&family, &first.refresh).retired_at.is_some());
    assert_eq!(row_of(&family, &next.refresh).retired_at, None);
}

/// A tenant, as the only thing outside the Hub crate that can make one hands
/// it out: the owner of a passkey enrolled for nobody else.
fn tenant(state: &AppState) -> UserId {
    SoftAuthenticator::new()
        .enrolled(state, Status::Trusted)
        .user_id
}

/// What a client is sent for the outcome of one consent body.
async fn sent<T: Serialize>(outcome: Result<T, ApiError>) -> (u16, Value) {
    match outcome {
        Ok(answer) => (
            200,
            serde_json::to_value(answer).expect("an answer that is JSON"),
        ),
        Err(refusal) => answered(refusal).await,
    }
}

/// `POST /v1/pairing/start` past its extractors, for a session of `user`.
async fn start(state: &AppState, user: UserId, segment: &str, now: u64) -> (u16, Value) {
    let body = serde_json::from_value(json!({ "request": segment })).expect("a start body");
    sent(started_pairing(state, user, body, now)).await
}

async fn approve(
    state: &AppState,
    user: UserId,
    pairing_id: &str,
    scopes: &[&str],
    now: u64,
) -> (u16, Value) {
    let body = serde_json::from_value(json!({ "pairing_id": pairing_id, "scopes": scopes }))
        .expect("an approval body");
    sent(approved_pairing(state, user, body, now)).await
}

async fn deny(state: &AppState, user: UserId, pairing_id: &str, now: u64) -> (u16, Value) {
    let body = serde_json::from_value(json!({ "pairing_id": pairing_id })).expect("a denial body");
    sent(denied_pairing(state, user, body, now)).await
}

fn asked(params: &AuthorizeParams) -> AuthorizeRequest {
    authorize::validate(params).expect("a request the Hub has nothing to refuse")
}

/// [`pairing_request`], which asks for `read` and `gates`, opened by `user`
/// at `now`: the `pairing_id` its consent screen was handed.
async fn opened(state: &AppState, user: UserId, now: u64) -> String {
    let segment = authorize::encode(&asked(&pairing_request()));
    let (status, prompt) = start(state, user, &segment, now).await;
    assert_eq!(status, 200, "{prompt}");
    prompt["pairing_id"]
        .as_str()
        .expect("a prompt names its pairing")
        .to_owned()
}

fn pairing_expired() -> (u16, Value) {
    (400, json!({ "error": "pairing_expired" }))
}

fn invalid_scope() -> (u16, Value) {
    (400, json!({ "error": "invalid_scope" }))
}

/// The session lookup is what would answer `500` here, since the pool cannot
/// connect: a `401` is the extractor refusing before it. A path nothing
/// serves answers something else to the same request, so the `401` is a
/// route's.
#[tokio::test]
async fn the_consent_routes_answer_401_without_a_session() {
    let base = serve_hub().await;
    let token = SessionSecret::generate()
        .expect("the system random source")
        .csrf_token();
    let signed_out = |path: &str| {
        let request = reqwest::Client::new()
            .post(format!("{base}{path}"))
            .header("origin", HUB_ORIGIN)
            .header(session::CSRF_HEADER, token.as_str())
            .send();
        async move {
            let response = request.await.expect("the loopback listener answers");
            let status = response.status().as_u16();
            let body = response.text().await.expect("a readable body");
            (status, serde_json::from_str(&body).unwrap_or(Value::Null))
        }
    };

    for path in [
        "/v1/pairing/start",
        "/v1/pairing/approve",
        "/v1/pairing/deny",
    ] {
        assert_eq!(
            signed_out(path).await,
            (401, json!({ "error": "unauthenticated" })),
            "{path}"
        );
    }
    assert_ne!(signed_out("/v1/pairing/no-such-route").await.0, 401);
}

#[tokio::test]
async fn starting_a_pairing_answers_the_fingerprint_of_the_requested_key_and_its_scopes() {
    let state = hub_state();
    let user = tenant(&state);

    let published = asked(&pairing_request());
    let (status, prompt) = start(&state, user, &authorize::encode(&published), NOW).await;

    let pairing_id = prompt["pairing_id"].clone();
    assert!(pairing_id.is_string(), "{prompt}");
    assert_eq!(
        (status, prompt),
        (
            200,
            json!({
                "pairing_id": pairing_id,
                "fingerprint": "SHA256:0ZcO CORZ NYy- DWpq",
                "instance_name": "Workstation",
                "os": "linux",
                "arch": "x86_64",
                "app_version": "1.4.0",
                "scopes": ["read", "gates"],
                "expires_at": NOW + CONSENT_TTL_SECS,
            })
        )
    );

    let jkt = SoftDeviceKey::new().thumbprint();
    let another = asked(&AuthorizeParams {
        dpop_jkt: Some(jkt.clone()),
        scope: Some("spend".to_owned()),
        ..pairing_request()
    });
    let (status, prompt) = start(&state, user, &authorize::encode(&another), NOW).await;

    assert_eq!(status, 200, "{prompt}");
    assert_eq!(prompt["fingerprint"], json!(fingerprint(&jkt)));
    assert_ne!(prompt["fingerprint"], json!("SHA256:0ZcO CORZ NYy- DWpq"));
    assert_eq!(prompt["scopes"], json!(["spend"]));
    assert_ne!(prompt["pairing_id"], pairing_id);

    assert_eq!(
        start(&state, user, "not-a-segment", NOW).await,
        invalid_request()
    );
}

#[tokio::test]
async fn approving_a_scope_that_was_not_requested_or_none_at_all_is_invalid_scope() {
    let state = hub_state();
    let user = tenant(&state);

    for narrowed in [&["read", "gates"][..], &["gates"]] {
        let pairing_id = opened(&state, user, NOW).await;
        let (status, answer) = approve(&state, user, &pairing_id, narrowed, NOW).await;
        assert_eq!(status, 200, "{narrowed:?}: {answer}");
    }

    for widened in [&[][..], &["spend"], &["read", "spend"], &["read", "read"]] {
        let pairing_id = opened(&state, user, NOW).await;
        assert_eq!(
            approve(&state, user, &pairing_id, widened, NOW).await,
            invalid_scope(),
            "{widened:?}"
        );
        assert_eq!(
            approve(&state, user, &pairing_id, &["read"], NOW).await,
            pairing_expired(),
            "a refused approval of {widened:?} did not spend its pairing"
        );
    }
}

#[tokio::test]
async fn a_pairing_id_is_refused_on_second_use_after_its_ttl_and_to_another_user() {
    let state = hub_state();
    let (mine, theirs) = (tenant(&state), tenant(&state));
    assert_ne!(mine, theirs);

    let approved_once = opened(&state, mine, NOW).await;
    let (status, _) = approve(&state, mine, &approved_once, &["read"], NOW).await;
    assert_eq!(status, 200);
    assert_eq!(
        approve(&state, mine, &approved_once, &["read"], NOW).await,
        pairing_expired()
    );
    assert_eq!(
        deny(&state, mine, &approved_once, NOW).await,
        pairing_expired()
    );

    let denied_once = opened(&state, mine, NOW).await;
    let (status, _) = deny(&state, mine, &denied_once, NOW).await;
    assert_eq!(status, 200);
    assert_eq!(
        deny(&state, mine, &denied_once, NOW).await,
        pairing_expired()
    );

    let (in_time, late) = (NOW + CONSENT_TTL_SECS, NOW + CONSENT_TTL_SECS + 1);
    let approved_in_time = opened(&state, mine, NOW).await;
    let denied_in_time = opened(&state, mine, NOW).await;
    let approved_late = opened(&state, mine, NOW).await;
    let denied_late = opened(&state, mine, NOW).await;
    let (status, _) = approve(&state, mine, &approved_in_time, &["read"], in_time).await;
    assert_eq!(status, 200);
    let (status, _) = deny(&state, mine, &denied_in_time, in_time).await;
    assert_eq!(status, 200);
    assert_eq!(
        approve(&state, mine, &approved_late, &["read"], late).await,
        pairing_expired()
    );
    assert_eq!(
        deny(&state, mine, &denied_late, late).await,
        pairing_expired()
    );

    let kept = opened(&state, mine, NOW).await;
    assert_eq!(
        approve(&state, theirs, &kept, &["read"], NOW).await,
        pairing_expired()
    );
    assert_eq!(deny(&state, theirs, &kept, NOW).await, pairing_expired());
    let (status, _) = approve(&state, mine, &kept, &["read"], NOW).await;
    assert_eq!(status, 200, "another user's attempts spent the pairing");

    assert_eq!(
        approve(&state, mine, "never-issued", &["read"], NOW).await,
        pairing_expired()
    );
}

#[tokio::test]
async fn denying_a_pairing_redirects_with_access_denied_and_leaves_no_code_to_redeem() {
    let state = hub_state();
    let user = tenant(&state);
    let pairing_id = opened(&state, user, NOW).await;

    let denied = deny(&state, user, &pairing_id, NOW).await;

    assert_eq!(
        denied,
        (
            200,
            json!({ "redirect": format!("{REDIRECT}?error=access_denied&state=xyz-123") })
        )
    );
    assert_eq!(
        approve(&state, user, &pairing_id, &["read"], NOW).await,
        pairing_expired()
    );
}

/// The code in the redirect is the only form of it that exists outside the
/// Hub, and what it finds there is everything `/token` will know of the
/// approval: who gave it, for which request, and narrowed to what.
#[tokio::test]
async fn approving_a_pairing_redirects_to_the_requested_uri_with_a_code_and_the_requests_state() {
    let state = hub_state();
    let user = tenant(&state);
    let pairing_id = opened(&state, user, NOW).await;

    let (status, answer) = approve(&state, user, &pairing_id, &["gates"], NOW).await;

    assert_eq!(status, 200, "{answer}");
    let redirect = answer["redirect"].as_str().expect("a redirect");
    let code = redirect
        .strip_prefix(&format!("{REDIRECT}?code="))
        .and_then(|rest| rest.strip_suffix("&state=xyz-123"))
        .unwrap_or_else(|| panic!("`{redirect}` is not the requested redirect and state"));
    assert_eq!(answer, json!({ "redirect": redirect }));
    let code = AuthorizationCode::parse(code).expect("32 bytes of unpadded base64url");

    let only_gates = AuthorizeParams {
        scope: Some("gates".to_owned()),
        ..pairing_request()
    };
    assert_eq!(
        state.pairings().take_code(&code.hash(), NOW),
        Some(ApprovedPairing {
            user,
            request: asked(&pairing_request()),
            scopes: asked(&only_gates).scopes,
            approved_at: NOW,
        })
    );
}

/// The name the Hubs below know their request-signing key by.
const SIGNING_KEY: &str = "demeteo-hub-signing";

/// [`hub_state`] over a key service of the caller's.
fn hub_state_with(keys: Arc<dyn KeyService>) -> AppState {
    let rp = RelyingParty {
        rp_id: "hub.example".to_owned(),
        origin: HUB_ORIGIN.to_owned(),
    };
    lazy_state_with_keys("postgres://hub@db.invalid/hub", rp, keys, SIGNING_KEY)
        .expect("a parsable database URL")
}

/// The code a desktop holding `key` is redirected with: [`pairing_request`]
/// naming that key, started by `user` at `now` and approved for `scopes`.
async fn approved_code(
    state: &AppState,
    user: UserId,
    key: &SoftDeviceKey,
    scopes: &[&str],
    now: u64,
) -> String {
    let request = asked(&AuthorizeParams {
        dpop_jkt: Some(key.thumbprint()),
        ..pairing_request()
    });
    let (status, prompt) = start(state, user, &authorize::encode(&request), now).await;
    assert_eq!(status, 200, "{prompt}");
    let pairing_id = prompt["pairing_id"]
        .as_str()
        .expect("a prompt names its pairing");
    let (status, answer) = approve(state, user, pairing_id, scopes, now).await;
    assert_eq!(status, 200, "{answer}");
    let redirect = answer["redirect"].as_str().expect("a redirect");
    redirect
        .strip_prefix(&format!("{REDIRECT}?code="))
        .and_then(|rest| rest.strip_suffix("&state=xyz-123"))
        .unwrap_or_else(|| panic!("`{redirect}` is not the requested redirect and state"))
        .to_owned()
}

/// Everything a redemption asked a store to write, in the order it asked.
type Handed = Rc<RefCell<Vec<(UserId, NewPairing)>>>;

/// A store that writes nothing: it keeps what it is handed and answers
/// `paired`, which is the caller's to choose.
fn recording(
    handed: Handed,
    paired: Paired,
) -> impl FnOnce(UserId, NewPairing) -> Ready<Result<Paired, StoreError>> {
    move |user, new| {
        handed.borrow_mut().push((user, new));
        ready(Ok(paired))
    }
}

/// An instance of a tenant holding `passkeys`, as `pair_instance` answers.
fn paired_with(passkeys: Vec<Passkey>) -> Paired {
    Paired {
        instance_id: InstanceId::generate(),
        passkeys,
    }
}

/// `POST /token` with `grant_type=authorization_code`, past its extractors:
/// `code` redeemed at `now` with the verifier and redirect of
/// [`pairing_request`] and a proof `key` made for it. The proof's `jti` is
/// the same every time, so a Hub is asked this once per key.
async fn redeem_code(
    state: &AppState,
    key: &SoftDeviceKey,
    code: &str,
    now: u64,
    handed: &Handed,
    paired: Paired,
) -> (u16, Value) {
    let form = serde_json::from_value(json!({
        "code": code,
        "code_verifier": RFC_7636_VERIFIER,
        "redirect_uri": REDIRECT,
    }))
    .expect("a redemption form");
    let proof = key.proof(
        "POST",
        &format!("{HUB_ORIGIN}/token"),
        now,
        "one-redemption",
    );
    let pair = recording(handed.clone(), paired);
    sent(redeemed(state, form, Some(&proof), now, pair).await).await
}

#[tokio::test]
async fn an_access_token_is_refused_once_fifteen_minutes_have_passed() {
    assert_eq!(ACCESS_TOKEN_TTL_SECS, 900);

    let state = hub_state_with(Arc::new(MemoryKeys::with_keys([SIGNING_KEY])));
    let user = tenant(&state);
    let key = SoftDeviceKey::new();
    let code = approved_code(&state, user, &key, &["read"], NOW).await;
    let handed = Handed::default();

    let (status, answer) =
        redeem_code(&state, &key, &code, NOW, &handed, paired_with(Vec::new())).await;

    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["expires_in"], json!(900));
    let handed = handed.borrow();
    let [(to, new)] = handed.as_slice() else {
        panic!("the store was asked {} times", handed.len());
    };
    assert_eq!(*to, user);
    assert_eq!(new.access.kind, TokenKind::Access);
    assert_eq!(new.access.expires_at.unix_timestamp(), (NOW + 900) as i64);

    let mut written = MemoryFamily::empty();
    let rows = FamilyChange {
        retire: Vec::new(),
        insert: vec![new.access.clone(), new.refresh.clone()],
    };
    written.apply(&rows, NOW);
    let access = answer["access_token"]
        .as_str()
        .and_then(TokenSecret::parse)
        .expect("an access token of 32 bytes");
    let row = row_of(&written, &access);
    assert_eq!(row.kind, TokenKind::Access);
    let jkt = key.thumbprint();
    assert_eq!(access_verdict(row, &jkt, NOW + 899), Ok(()));
    assert_eq!(
        access_verdict(row, &jkt, NOW + 900),
        Err(AccessRefusal::Expired)
    );
}

/// What an instance pins of `authenticator`'s passkey: the raw credential id
/// and the COSE_Key bytes, each as base64url.
fn pinned(authenticator: &SoftAuthenticator) -> Value {
    json!({
        "credential_id": authenticator.credential_id(),
        "public_key": base64url_encode(&authenticator.public_key_cose()),
    })
}

#[tokio::test]
async fn the_pairing_response_carries_the_signing_key_the_rp_id_and_the_trusted_passkeys() {
    let keys: Arc<dyn KeyService> = Arc::new(MemoryKeys::with_keys([SIGNING_KEY]));
    let state = hub_state_with(keys.clone());
    let user = tenant(&state);
    let key = SoftDeviceKey::new();
    let code = approved_code(&state, user, &key, &["gates"], NOW).await;
    let authenticators: [SoftAuthenticator; 4] = std::array::from_fn(|_| SoftAuthenticator::new());
    let [first, pending, revoked, second] = &authenticators;
    let paired = paired_with(vec![
        first.enrolled(&state, Status::Trusted),
        pending.enrolled(&state, Status::Pending),
        revoked.enrolled(&state, Status::Revoked),
        second.enrolled(&state, Status::Trusted),
    ]);
    let instance_id = paired.instance_id.0.to_string();

    let (status, answer) = redeem_code(&state, &key, &code, NOW, &Handed::default(), paired).await;

    assert_eq!(status, 200, "{answer}");
    for token in ["access_token", "refresh_token"] {
        assert!(answer[token].is_string(), "{token}: {answer}");
    }
    assert_eq!(state.rp().rp_id, "hub.example");
    assert_eq!(
        answer,
        json!({
            "access_token": answer["access_token"],
            "token_type": "DPoP",
            "expires_in": 900,
            "refresh_token": answer["refresh_token"],
            "scope": "gates",
            "instance_id": instance_id,
            "request_signing_key": answer["request_signing_key"],
            "rp_id": "hub.example",
            "passkeys": [pinned(first), pinned(second)],
        })
    );

    let signing_key = answer["request_signing_key"]
        .as_str()
        .and_then(base64url_decode)
        .expect("a signing key in unpadded base64url");
    assert_eq!(signing_key.len(), 32);
    let message = b"a request the Hub signed";
    let signature = keys
        .sign(SIGNING_KEY, message)
        .await
        .expect("the configured key signs");
    assert!(verifies_ed25519(&signing_key, message, &signature));
    assert!(!verifies_ed25519(
        &signing_key,
        b"another request",
        &signature
    ));
    let by_another_hub = MemoryKeys::with_keys([SIGNING_KEY])
        .sign(SIGNING_KEY, message)
        .await
        .expect("a key of the same name signs");
    assert!(!verifies_ed25519(&signing_key, message, &by_another_hub));

    let refreshed = serde_json::to_value(refresh_response(fresh_pair())).expect("JSON");
    for pinned_value in ["request_signing_key", "rp_id", "passkeys"] {
        assert!(answer.get(pinned_value).is_some(), "{pinned_value}");
        assert!(refreshed.get(pinned_value).is_none(), "{pinned_value}");
    }
    assert_eq!(refreshed["token_type"], json!("DPoP"));
}

/// One redemption on a Hub whose key service is `keys`: what it answered,
/// and how many times it asked the store to write.
async fn redemption_beside(keys: MemoryKeys) -> ((u16, Value), usize) {
    let state = hub_state_with(Arc::new(keys));
    let user = tenant(&state);
    let key = SoftDeviceKey::new();
    let code = approved_code(&state, user, &key, &["read"], NOW).await;
    let handed = Handed::default();
    let answer = redeem_code(&state, &key, &code, NOW, &handed, paired_with(Vec::new())).await;
    let asked = handed.borrow().len();
    (answer, asked)
}

#[tokio::test]
async fn a_key_service_that_cannot_answer_is_503_and_hands_nothing_to_the_store() {
    let ((status, _), asked) = redemption_beside(MemoryKeys::with_keys([SIGNING_KEY])).await;
    assert_eq!((status, asked), (200, 1));

    let unavailable = (503, json!({ "error": "key_service_unavailable" }));
    for keys in [MemoryKeys::sealed(), MemoryKeys::with_keys(["another-key"])] {
        assert_eq!(redemption_beside(keys).await, (unavailable.clone(), 0));
    }
}

/// What a proof for the token endpoint names as its `htu`: the origin the Hub
/// was configured with, which is not the loopback address it is reached at
/// here.
fn token_url() -> String {
    format!("{HUB_ORIGIN}/token")
}

/// The second the Hub's clock reads, near enough. The router is handed no
/// `now`: it asks the clock, so what it is sent is parked and dated by it.
fn wall_clock() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock set after 1970");
    since_epoch.as_secs()
}

/// How far outside its window a proof refused for its age is dated. The Hub
/// reads the clock after [`wall_clock`] did: one second past the bound ahead
/// of the test's clock is inside the bound by the time a slow run asks.
const CLOCK_MARGIN_SECS: u64 = 60;

/// The form a desktop redirected with `code` sends to redeem it, when the
/// pairing was asked for as [`pairing_request`] asks.
fn redemption(code: &str) -> Value {
    json!({
        "grant_type": "authorization_code",
        "code": code,
        "code_verifier": RFC_7636_VERIFIER,
        "redirect_uri": REDIRECT,
    })
}

fn with_member(form: &Value, member: &str, value: &str) -> Value {
    let mut form = form.clone();
    form[member] = json!(value);
    form
}

/// One `POST /token` to the real router: `form` as a body encoded by hand,
/// under one `DPoP` header for each of `proofs`.
async fn post_token(base: &str, proofs: &[&str], form: &Value) -> Answer {
    let mut request = reqwest::Client::new()
        .post(format!("{base}/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(urlencoded(form));
    for proof in proofs {
        request = request.header("dpop", *proof);
    }
    let response = request.send().await.expect("the loopback listener answers");
    answer_of(response).await
}

/// `form` past the route's extractors, presented at `now` with `proof` to a
/// store that answers an instance with no passkeys. A redemption the router
/// would grant is watched succeed here: the router's own store is a database
/// that is not there, and a request it has nothing to refuse answers `500`.
async fn redeem_past_the_router(
    state: &AppState,
    form: &Value,
    proof: &str,
    now: u64,
) -> (u16, Value) {
    let form = serde_json::from_value(form.clone()).expect("a redemption form");
    let pair = recording(Handed::default(), paired_with(Vec::new()));
    sent(redeemed(state, form, Some(proof), now, pair).await).await
}

fn invalid_dpop_proof() -> (u16, Value) {
    (400, json!({ "error": "invalid_dpop_proof" }))
}

fn invalid_grant() -> (u16, Value) {
    (400, json!({ "error": "invalid_grant" }))
}

fn with_a_signature_bit_flipped(proof: &str) -> String {
    let (signed, signature) = proof.rsplit_once('.').expect("a compact JWS");
    let mut signature = base64url_decode(signature).expect("a base64url signature");
    signature[0] ^= 1;
    format!("{signed}.{}", base64url_encode(&signature))
}

/// Each near miss is of a proof the router is first watched take: by `key`,
/// dated at the Hub's clock, for a `POST` to `/token` at the configured
/// origin. Each is sent with a code that is then watched redeem, so the proof
/// is all that was wrong with the request it was refused in.
#[tokio::test]
async fn token_refuses_a_request_without_a_valid_dpop_proof() {
    let state = hub_state_with(Arc::new(MemoryKeys::with_keys([SIGNING_KEY])));
    let base = serve_state(state.clone()).await;
    let user = tenant(&state);
    let key = SoftDeviceKey::new();
    let (now, htu) = (wall_clock(), token_url());
    let verifying = |jti: &str| key.proof("POST", &htu, now, jti);
    let with_header = |member: &str, value: Value| {
        let mut header = key.header();
        header[member] = value;
        key.proof_with(&header, &claims("POST", &htu, now, member))
    };

    let presented_twice = verifying("presented twice");
    let taken = post_token(&base, &[&presented_twice], &redemption("never-issued")).await;
    assert_eq!(taken.outcome, invalid_grant());
    assert_eq!(taken.cache_control.as_deref(), Some("no-store"));

    let mut with_its_private_half = key.jwk();
    with_its_private_half["d"] = json!(base64url_encode(&[7; 32]));
    let out_of_window = DPOP_MAX_AGE_SECS + CLOCK_MARGIN_SECS;
    let near_misses = [
        ("no proof", vec![]),
        (
            "two proofs",
            vec![verifying("first of two"), verifying("second of two")],
        ),
        ("typ", vec![with_header("typ", json!("jwt"))]),
        ("alg none", vec![with_header("alg", json!("none"))]),
        ("alg HS256", vec![with_header("alg", json!("HS256"))]),
        (
            "a jwk with d",
            vec![with_header("jwk", with_its_private_half)],
        ),
        ("htm", vec![key.proof("GET", &htu, now, "htm")]),
        (
            "htu of another path",
            vec![key.proof("POST", &format!("{HUB_ORIGIN}/authorize"), now, "htu")],
        ),
        (
            "htu of the address the request was sent to",
            vec![key.proof("POST", &format!("{base}/token"), now, "host")],
        ),
        (
            "a stale iat",
            vec![key.proof("POST", &htu, now - out_of_window, "stale")],
        ),
        (
            "a future iat",
            vec![key.proof("POST", &htu, now + out_of_window, "future")],
        ),
        (
            "a flipped signature bit",
            vec![with_a_signature_bit_flipped(&verifying("signature"))],
        ),
        ("a replayed jti", vec![presented_twice.clone()]),
    ];
    for (fault, proofs) in &near_misses {
        let form = redemption(&approved_code(&state, user, &key, &["read"], now).await);
        let proofs: Vec<&str> = proofs.iter().map(String::as_str).collect();

        let answer = post_token(&base, &proofs, &form).await;

        assert_eq!(answer.outcome, invalid_dpop_proof(), "{fault}");
        assert_eq!(answer.cache_control.as_deref(), Some("no-store"), "{fault}");
        let another = verifying(&format!("after {fault}"));
        let (status, granted) = redeem_past_the_router(&state, &form, &another, now).await;
        assert_eq!(status, 200, "{fault}: {granted}");
    }
}

#[tokio::test]
async fn token_refuses_a_proof_signed_by_a_key_other_than_the_one_bound_at_authorization() {
    let state = hub_state_with(Arc::new(MemoryKeys::with_keys([SIGNING_KEY])));
    let base = serve_state(state.clone()).await;
    let user = tenant(&state);
    let (bound, another) = (SoftDeviceKey::new(), SoftDeviceKey::new());
    assert_ne!(bound.thumbprint(), another.thumbprint());
    let (now, htu) = (wall_clock(), token_url());

    let form = redemption(&approved_code(&state, user, &bound, &["read"], now).await);
    let by_the_bound_key = bound.proof("POST", &htu, now, "one");
    let (status, granted) = redeem_past_the_router(&state, &form, &by_the_bound_key, now).await;
    assert_eq!(status, 200, "{granted}");

    let form = redemption(&approved_code(&state, user, &bound, &["read"], now).await);
    let by_another_key = another.proof("POST", &htu, now, "one");
    let answer = post_token(&base, &[&by_another_key], &form).await;

    assert_eq!(answer.outcome, invalid_grant());
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"));
}

#[tokio::test]
async fn an_authorization_code_is_redeemed_once_within_its_ttl_and_burned_by_a_failed_redemption() {
    assert_eq!(AUTHORIZATION_CODE_TTL_SECS, 60);

    let state = hub_state_with(Arc::new(MemoryKeys::with_keys([SIGNING_KEY])));
    let user = tenant(&state);
    let key = SoftDeviceKey::new();
    let htu = token_url();
    let approved = || approved_code(&state, user, &key, &["read"], NOW);
    let present = |form: Value, now: u64, jti: &'static str| {
        let proof = key.proof("POST", &htu, now, jti);
        let state = &state;
        async move { redeem_past_the_router(state, &form, &proof, now).await }
    };
    let (in_time, late) = (NOW + 60, NOW + 61);

    let form = redemption(&approved().await);
    let (status, granted) = present(form.clone(), in_time, "in time").await;
    assert_eq!(status, 200, "{granted}");
    assert_eq!(present(form, in_time, "again").await, invalid_grant());

    let form = redemption(&approved().await);
    assert_eq!(present(form, late, "late").await, invalid_grant());

    let form = redemption(&approved().await);
    let mistaken = with_member(&form, "redirect_uri", "http://127.0.0.1:8081/cb");
    assert_eq!(present(mistaken, NOW, "mistaken").await, invalid_grant());
    assert_eq!(present(form, NOW, "corrected").await, invalid_grant());
}

#[tokio::test]
async fn a_wrong_code_verifier_and_a_different_redirect_uri_are_each_invalid_grant() {
    let state = hub_state_with(Arc::new(MemoryKeys::with_keys([SIGNING_KEY])));
    let base = serve_state(state.clone()).await;
    let user = tenant(&state);
    let key = SoftDeviceKey::new();
    let (now, htu) = (wall_clock(), token_url());
    let approved = || approved_code(&state, user, &key, &["read"], now);

    let wrong = [
        (
            "code_verifier",
            with_one_character_changed(RFC_7636_VERIFIER, 0),
        ),
        ("redirect_uri", format!("{REDIRECT}/")),
    ];
    for (member, value) in &wrong {
        let as_asked = redemption(&approved().await);
        let proof = key.proof("POST", &htu, now, &format!("{member} as asked"));
        let (status, granted) = redeem_past_the_router(&state, &as_asked, &proof, now).await;
        assert_eq!(status, 200, "{member}: {granted}");

        let changed = with_member(&redemption(&approved().await), member, value);
        let proof = key.proof("POST", &htu, now, &format!("{member} changed"));
        assert_eq!(
            redeem_past_the_router(&state, &changed, &proof, now).await,
            invalid_grant(),
            "{member}"
        );

        let changed = with_member(&redemption(&approved().await), member, value);
        let proof = key.proof("POST", &htu, now, &format!("{member} on the router"));
        let answer = post_token(&base, &[&proof], &changed).await;
        assert_eq!(answer.outcome, invalid_grant(), "{member}");
    }
}

/// Neither answer is axum's: a form the extractor refuses and a grant type
/// the Hub does not serve are each told in the Hub's own codes, and the
/// absent `Origin` that `/v1` answers `403` is no part of either.
#[tokio::test]
async fn token_answers_an_unreadable_form_and_an_unserved_grant_type_in_its_own_codes() {
    let base = serve_hub().await;
    let proof = SoftDeviceKey::new().proof("POST", &token_url(), wall_clock(), "one");

    let without_a_verifier = json!({
        "grant_type": "authorization_code",
        "code": "never-issued",
        "redirect_uri": REDIRECT,
    });
    let unreadable = post_token(&base, &[&proof], &without_a_verifier).await;
    assert_eq!(unreadable.outcome, invalid_request());
    assert_eq!(unreadable.cache_control.as_deref(), Some("no-store"));

    let unserved = with_member(&redemption("never-issued"), "grant_type", "password");
    let unserved = post_token(&base, &[&proof], &unserved).await;
    assert_eq!(
        unserved.outcome,
        (400, json!({ "error": "unsupported_grant_type" }))
    );
    assert_eq!(unserved.cache_control.as_deref(), Some("no-store"));
}

/// One `grant_type=refresh_token` request past the route's extractors:
/// `refresh_token` presented at `now` with a proof `key` made for it, to a
/// store whose rows are `family`. What it was answered, and what the store
/// was handed if it was asked: the hash to find the family by and the change
/// to write. The change is written whatever the answer, as `rotate_family`
/// writes it; of what such a store answers, the body reads only the pair.
async fn refresh_token(
    state: &AppState,
    key: &SoftDeviceKey,
    family: &mut MemoryFamily,
    refresh_token: &str,
    now: u64,
    jti: &str,
) -> ((u16, Value), Option<(TokenHash, FamilyChange)>) {
    let form =
        serde_json::from_value(json!({ "refresh_token": refresh_token })).expect("a refresh form");
    let proof = key.proof("POST", &token_url(), now, jti);
    let mut handed = None;
    let rotate = |presented: TokenHash, decide: Rotation| {
        let (change, answer) = decide(family.rows());
        family.apply(&change, now);
        handed = Some((presented, change));
        ready(Ok(FamilyWritten {
            retired: Vec::new(),
            inserted: Vec::new(),
            answer,
        }))
    };
    let answer = sent(refreshed(state, form, Some(&proof), now, rotate).await).await;
    (answer, handed)
}

/// The token or code a client was sent as `member` of `answer`: 43
/// characters, the unpadded base64url of 32 bytes.
fn sent_secret<'a>(answer: &'a Value, member: &str) -> &'a str {
    let text = answer[member]
        .as_str()
        .unwrap_or_else(|| panic!("no `{member}` in {answer}"));
    assert_eq!(text.len(), 43, "{member}");
    text
}

/// What a row holds of the secret a client was sent as `text`, if the row
/// was built by the rule `pairing::grant` states: the digest of the 32 bytes
/// the text encodes, not of the text. It is computed here from the decoded
/// text and not by `TokenSecret`, which is the code under test.
fn digest_of_the_bytes(text: &str) -> TokenHash {
    let bytes = base64url_decode(text).expect("a secret in unpadded base64url");
    assert_eq!(bytes.len(), 32);
    TokenHash::digest(&bytes)
}

/// Every text a row carries, beside the row as `Debug` shows it.
fn shown_of_row(row: &NewToken) -> Vec<String> {
    vec![format!("{row:?}"), row.jkt.clone()]
}

/// The pairing is granted by `redeemed` and the rotation by `refreshed`,
/// whose decision is `refresh_change`: what is examined is what each body
/// handed its store, not rows built for the purpose.
#[tokio::test]
async fn nothing_handed_to_the_store_holds_a_token_or_code_in_plaintext() {
    let state = hub_state_with(Arc::new(MemoryKeys::with_keys([SIGNING_KEY])));
    let user = tenant(&state);
    let key = SoftDeviceKey::new();
    let code = approved_code(&state, user, &key, &["read", "gates"], NOW).await;
    let handed = Handed::default();

    let (status, paired) =
        redeem_code(&state, &key, &code, NOW, &handed, paired_with(Vec::new())).await;

    assert_eq!(status, 200, "{paired}");
    let handed = handed.take();
    let [(_, new)] = handed.as_slice() else {
        panic!("the store was asked {} times", handed.len());
    };
    let mut family = MemoryFamily::empty();
    let first_rows = FamilyChange {
        retire: Vec::new(),
        insert: vec![new.access.clone(), new.refresh.clone()],
    };
    family.apply(&first_rows, NOW);

    let first_refresh = sent_secret(&paired, "refresh_token");
    let (rotated, asked) =
        refresh_token(&state, &key, &mut family, first_refresh, NOW + 60, "one").await;

    let (status, rotated) = rotated;
    assert_eq!(status, 200, "{rotated}");
    let (presented, change) = asked.expect("a rotation asks its store");
    let [access_row, refresh_row] = change.insert.as_slice() else {
        panic!("a rotation adds a pair: {change:?}");
    };

    let first_access = sent_secret(&paired, "access_token");
    let next_access = sent_secret(&rotated, "access_token");
    let next_refresh = sent_secret(&rotated, "refresh_token");
    let secrets = [
        ("the authorization code", code.as_str()),
        ("the first access token", first_access),
        ("the first refresh token", first_refresh),
        ("the next access token", next_access),
        ("the next refresh token", next_refresh),
    ];
    assert_eq!(code.len(), 43);
    let mut shown = vec![
        format!("{new:?}"),
        new.instance.name.clone(),
        new.instance.os.clone(),
        new.instance.arch.clone(),
        new.instance.version.clone(),
        new.device_key.public_jwk.clone(),
        new.device_key.jkt.clone(),
        format!("{presented:?}"),
        format!("{change:?}"),
    ];
    for row in [&new.access, &new.refresh, access_row, refresh_row] {
        shown.extend(shown_of_row(row));
    }
    for (name, secret) in secrets {
        for shown in &shown {
            assert!(!shown.contains(secret), "{name} is in `{shown}`");
        }
    }

    assert_eq!(new.access.hash, digest_of_the_bytes(first_access));
    assert_eq!(new.refresh.hash, digest_of_the_bytes(first_refresh));
    assert_eq!(presented, digest_of_the_bytes(first_refresh));
    assert_eq!(access_row.hash, digest_of_the_bytes(next_access));
    assert_eq!(refresh_row.hash, digest_of_the_bytes(next_refresh));
}

/// The refresh the router is asked for names a text that is no token, which
/// is refused before the store is asked: the router's store is a database
/// that is not there. With a proof it is the token that is refused, so the
/// proof is all the first request lacked.
#[tokio::test]
async fn token_refuses_a_refresh_without_a_dpop_proof_and_a_grant_type_it_does_not_serve() {
    let base = serve_hub().await;
    let key = SoftDeviceKey::new();
    let proof = |jti: &str| key.proof("POST", &token_url(), wall_clock(), jti);
    let refresh = json!({ "grant_type": "refresh_token", "refresh_token": "never-issued" });

    let without_a_proof = post_token(&base, &[], &refresh).await;

    assert_eq!(without_a_proof.outcome, invalid_dpop_proof());
    assert_eq!(without_a_proof.cache_control.as_deref(), Some("no-store"));
    let with_one = post_token(&base, &[&proof("with one")], &refresh).await;
    assert_eq!(with_one.outcome, invalid_grant());
    assert_eq!(with_one.cache_control.as_deref(), Some("no-store"));

    for grant_type in ["refresh", "client_credentials"] {
        let unserved = with_member(&refresh, "grant_type", grant_type);

        let unserved = post_token(&base, &[&proof(grant_type)], &unserved).await;

        assert_eq!(
            unserved.outcome,
            (400, json!({ "error": "unsupported_grant_type" })),
            "{grant_type}"
        );
        assert_eq!(unserved.cache_control.as_deref(), Some("no-store"));
    }
}

/// The key service of this Hub is sealed, and the first presentation is
/// granted all the same: a rotation asks it nothing.
#[tokio::test]
async fn a_reused_refresh_token_is_invalid_grant_and_its_family_is_retired_under_that_answer() {
    let state = hub_state_with(Arc::new(MemoryKeys::sealed()));
    let key = SoftDeviceKey::new();
    let first = fresh_pair();
    let mut family = MemoryFamily::paired(&first, &key.thumbprint(), NOW);
    let first_refresh = first.refresh.text();

    let ((status, rotated), _) =
        refresh_token(&state, &key, &mut family, &first_refresh, NOW + 60, "one").await;

    assert_eq!(status, 200, "{rotated}");
    assert_eq!(
        rotated,
        json!({
            "access_token": sent_secret(&rotated, "access_token"),
            "token_type": "DPoP",
            "expires_in": 900,
            "refresh_token": sent_secret(&rotated, "refresh_token"),
        })
    );
    assert_eq!(live_rows(&family), 3);

    let (reused, asked) =
        refresh_token(&state, &key, &mut family, &first_refresh, NOW + 120, "two").await;

    assert_eq!(reused, invalid_grant());
    let (_, change) = asked.expect("a reused token is the store's to find");
    assert_eq!(change.retire.len(), 3);
    assert!(change.insert.is_empty(), "{change:?}");
    assert_eq!(live_rows(&family), 0);

    let newest = sent_secret(&rotated, "refresh_token");
    let (after, _) = refresh_token(&state, &key, &mut family, newest, NOW + 180, "three").await;
    assert_eq!(after, invalid_grant());
    assert_eq!(live_rows(&family), 0);
}
