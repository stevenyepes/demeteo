// Tests extracted from `src/adapters/mcp/token.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::token`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

use crate::adapters::mcp::consent_waiter::ConsentDecision;
use crate::adapters::mcp::{record_canonical_uri, router};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::oauth::Scope;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;
use crate::support::notification_capture::CapturingNotificationPort;

const REDIRECT_URI: &str = "http://127.0.0.1:9/cb";

/// Same shape as `authorize_pkce.rs`'s `fixture()`, wired to a capturing
/// double so the ordering assertion AC3 requires (`McpConsentRequested`
/// observed before `/token` is ever called) is provable, not assumed.
fn fixture(tag: &str) -> (AppContext, Arc<CapturingNotificationPort>) {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-token-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    let captured = Arc::new(CapturingNotificationPort::new());
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        captured.clone() as Arc<dyn NotificationPort>,
        tokio::runtime::Handle::current(),
    );
    (ctx, captured)
}

async fn spawn_mcp_router(tag: &str) -> (SocketAddr, AppContext, Arc<CapturingNotificationPort>) {
    let (ctx, captured) = fixture(tag);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener
        .local_addr()
        .expect("bound listener has a local address");

    let router_ctx = ctx.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router(router_ctx)).await;
    });

    (addr, ctx, captured)
}

async fn register_client(addr: SocketAddr) -> String {
    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{addr}/register"))
        .json(&serde_json::json!({
            "client_name": "claude-desktop",
            "redirect_uris": [REDIRECT_URI],
        }))
        .send()
        .await
        .expect("request /register")
        .json()
        .await
        .expect("registration response is JSON");
    body["client_id"]
        .as_str()
        .expect("response carries a client_id")
        .to_string()
}

/// A fixed verifier/challenge pair is fine across tests: each test spins up
/// its own router and client registration, so there is no shared state a
/// fixed value could collide on.
fn pkce_pair() -> (&'static str, String) {
    let verifier = "a-verifier-that-is-long-enough-for-pkce";
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn authorize_url(
    addr: SocketAddr,
    client_id: &str,
    resource: &str,
    code_challenge: &str,
) -> String {
    format!(
        "http://{addr}/authorize?client_id={client_id}&redirect_uri={REDIRECT_URI}&code_challenge={code_challenge}&code_challenge_method=S256&resource={resource}&scope=read%20spend"
    )
}

/// Percent-encodes a `application/x-www-form-urlencoded` value. Hand-rolled
/// because `reqwest`'s `form` feature isn't enabled in this workspace
/// (`Cargo.toml` only turns on `blocking`, `json`, `rustls`) and this test
/// exists to exercise `axum::extract::Form` server-side, not to justify a
/// new client-side feature flag for it.
fn urlencode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

async fn post_token(addr: SocketAddr, pairs: &[(&str, &str)]) -> reqwest::Response {
    let body = pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencode(v)))
        .collect::<Vec<_>>()
        .join("&");
    reqwest::Client::new()
        .post(format!("http://{addr}/token"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .expect("request /token")
}

/// Waits (bounded) until `captured` has recorded at least one event — see
/// `authorize_pkce.rs`'s identical helper for why this polls rather than
/// assumes a single `yield_now` schedules the whole loopback round trip.
async fn wait_for_event(captured: &CapturingNotificationPort) -> DomainEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = captured.events().into_iter().next() {
            return event;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for /authorize to emit McpConsentRequested"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Drives `GET /authorize` to an approved decision and returns the
/// single-use authorization code. Only returns once `McpConsentRequested`
/// has been observed and asserted on `captured` — the ordering AC3 requires
/// — so every caller gets that guarantee before it can reach `POST /token`.
async fn authorize_and_approve(
    addr: SocketAddr,
    ctx: &AppContext,
    captured: &CapturingNotificationPort,
    client_id: &str,
    resource: &str,
    code_challenge: &str,
) -> String {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build a non-redirecting client");
    let request = tokio::spawn({
        let url = authorize_url(addr, client_id, resource, code_challenge);
        let client = client.clone();
        async move { client.get(url).send().await }
    });

    let event = wait_for_event(captured).await;
    let DomainEvent::McpConsentRequested { request_id, .. } = event else {
        panic!("expected McpConsentRequested before /token can be reached, got {event:?}");
    };
    ctx.mcp_consent
        .deliver(&request_id, ConsentDecision::Approved);

    let resp = request
        .await
        .expect("authorize request task panicked")
        .expect("request /authorize");
    assert_eq!(resp.status(), reqwest::StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("redirect carries a Location header")
        .to_str()
        .expect("Location is ASCII")
        .to_string();

    let code = location
        .split("code=")
        .nth(1)
        .expect("approved redirect carries a code")
        .to_string();
    assert!(!code.is_empty());
    code
}

/// The single ordered test implementation-spec.md AC3 requires: register ->
/// authorize (approved the same way `mcp_consent_decide` will) -> assert
/// `McpConsentRequested` arrived before `/token` is called -> token ->
/// assert a plaintext token comes back and an `oauth_grants` row now exists,
/// found only by hash.
#[tokio::test]
async fn register_authorize_consent_token_round_trips_to_a_working_grant() {
    let (addr, ctx, captured) = spawn_mcp_router("full-flow").await;
    let resource = record_canonical_uri(addr);
    let client_id = register_client(addr).await;
    let (verifier, challenge) = pkce_pair();

    let code =
        authorize_and_approve(addr, &ctx, &captured, &client_id, &resource, &challenge).await;

    let resp = post_token(
        addr,
        &[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("code_verifier", verifier),
            ("redirect_uri", REDIRECT_URI),
            ("resource", resource.as_str()),
        ],
    )
    .await;

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("token response is JSON");
    let access_token = body["access_token"]
        .as_str()
        .expect("response carries a plaintext access_token")
        .to_string();
    assert!(!access_token.is_empty());
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["scope"], "read spend");
    assert_eq!(body["expires_in"], 30 * 24 * 60 * 60);

    let token_hash = format!("{:x}", Sha256::digest(access_token.as_bytes()));
    let grant = ctx
        .oauth_grants
        .find_grant_by_token_hash(&token_hash)
        .expect("find_grant_by_token_hash does not error")
        .expect("an oauth_grants row now exists for the issued token's hash");
    assert_eq!(grant.resource, resource);
    assert_eq!(grant.scopes, vec![Scope::Read, Scope::Spend]);
    assert!(grant.revoked_at.is_none());
    assert_eq!(
        grant.expires_at - grant.issued_at,
        30 * 24 * 60 * 60 * 1000,
        "grant must carry a fixed 30-day expiry from issuance"
    );
}

#[tokio::test]
async fn a_replayed_code_is_rejected_on_its_second_use() {
    let (addr, ctx, captured) = spawn_mcp_router("replay").await;
    let resource = record_canonical_uri(addr);
    let client_id = register_client(addr).await;
    let (verifier, challenge) = pkce_pair();

    let code =
        authorize_and_approve(addr, &ctx, &captured, &client_id, &resource, &challenge).await;
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("code_verifier", verifier),
        ("redirect_uri", REDIRECT_URI),
        ("resource", resource.as_str()),
    ];

    let first = post_token(addr, &form).await;
    assert_eq!(first.status(), reqwest::StatusCode::OK);

    let replay = post_token(addr, &form).await;
    assert_eq!(replay.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_pkce_verifier_mismatch_is_rejected_and_burns_the_code() {
    let (addr, ctx, captured) = spawn_mcp_router("pkce-mismatch").await;
    let resource = record_canonical_uri(addr);
    let client_id = register_client(addr).await;
    let (verifier, challenge) = pkce_pair();

    let code =
        authorize_and_approve(addr, &ctx, &captured, &client_id, &resource, &challenge).await;

    let mismatched = post_token(
        addr,
        &[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("code_verifier", "not-the-real-verifier"),
            ("redirect_uri", REDIRECT_URI),
            ("resource", resource.as_str()),
        ],
    )
    .await;
    assert_eq!(mismatched.status(), reqwest::StatusCode::BAD_REQUEST);

    // The code is consumed on lookup, not on success (see token.rs's module
    // docs) — even the *correct* verifier must now fail, proving a stolen
    // code can't be probed with repeated verifier guesses.
    let retry_with_correct_verifier = post_token(
        addr,
        &[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("code_verifier", verifier),
            ("redirect_uri", REDIRECT_URI),
            ("resource", resource.as_str()),
        ],
    )
    .await;
    assert_eq!(
        retry_with_correct_verifier.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "a code must not survive a failed exchange attempt"
    );
}

#[tokio::test]
async fn a_resource_not_matching_the_canonical_uri_is_rejected() {
    let (addr, _ctx, _captured) = spawn_mcp_router("resource-mismatch").await;
    record_canonical_uri(addr);

    let resp = post_token(
        addr,
        &[
            ("grant_type", "authorization_code"),
            ("code", "irrelevant"),
            ("code_verifier", "irrelevant"),
            ("redirect_uri", REDIRECT_URI),
            ("resource", "https://not-this-server.example"),
        ],
    )
    .await;

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_unsupported_grant_type_is_rejected() {
    let (addr, _ctx, _captured) = spawn_mcp_router("bad-grant-type").await;
    let resource = record_canonical_uri(addr);

    let resp = post_token(
        addr,
        &[
            ("grant_type", "client_credentials"),
            ("code", "irrelevant"),
            ("code_verifier", "irrelevant"),
            ("redirect_uri", REDIRECT_URI),
            ("resource", resource.as_str()),
        ],
    )
    .await;

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
}
