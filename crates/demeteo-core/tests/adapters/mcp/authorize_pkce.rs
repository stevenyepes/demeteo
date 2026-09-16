// Tests extracted from `src/adapters/mcp/authorize.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::authorize`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::adapters::mcp::consent_waiter::ConsentDecision;
use crate::adapters::mcp::{record_canonical_uri, router};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::ClientId;
use crate::domain::oauth::{OAuthClient, Scope};
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;
use crate::support::notification_capture::CapturingNotificationPort;

const CLIENT_REDIRECT_URI: &str = "http://127.0.0.1:9/cb";

/// Same shape as `tests/adapters/mcp/register.rs`'s `fixture()`, wired to a
/// capturing double instead of the no-op adapter: this file's whole point is
/// asserting what did — and, more importantly, did not — get emitted.
fn fixture(tag: &str) -> (AppContext, Arc<CapturingNotificationPort>) {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-authorize-{tag}-{}",
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

fn register_test_client(ctx: &AppContext) {
    ctx.oauth_clients
        .register_client(OAuthClient {
            id: ClientId::new("client-authorize"),
            client_name: "claude-desktop".to_string(),
            redirect_uris: vec![CLIENT_REDIRECT_URI.to_string()],
            created_at: crate::paths::now_ms(),
        })
        .expect("register test client");
}

fn authorize_url(addr: SocketAddr, resource: &str, code_challenge: &str, method: &str) -> String {
    format!(
        "http://{addr}/authorize?client_id=client-authorize&redirect_uri={CLIENT_REDIRECT_URI}&code_challenge={code_challenge}&code_challenge_method={method}&resource={resource}&scope=read%20spend"
    )
}

/// Waits (bounded) until `captured` has recorded at least one event.
/// `/authorize`'s success path emits `McpConsentRequested` synchronously
/// before it ever parks, but reaching that point still crosses a real
/// loopback TCP round trip, so this polls rather than assuming a single
/// `yield_now` schedules the whole hop.
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

#[tokio::test]
async fn missing_code_challenge_is_rejected_before_any_consent_event() {
    let (addr, _ctx, captured) = spawn_mcp_router("missing-code-challenge").await;

    let resp = reqwest::Client::new()
        .get(authorize_url(addr, "http://example.invalid", "", "S256"))
        .send()
        .await
        .expect("request /authorize");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(
        captured.events().is_empty(),
        "a PKCE-rejected /authorize request must emit zero DomainEvents, got {:?}",
        captured.events()
    );
}

#[tokio::test]
async fn non_s256_method_is_rejected_before_any_consent_event() {
    let (addr, _ctx, captured) = spawn_mcp_router("non-s256-method").await;

    let resp = reqwest::Client::new()
        .get(authorize_url(
            addr,
            "http://example.invalid",
            "a-challenge",
            "plain",
        ))
        .send()
        .await
        .expect("request /authorize");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(
        captured.events().is_empty(),
        "a PKCE-rejected /authorize request must emit zero DomainEvents, got {:?}",
        captured.events()
    );
}

#[tokio::test]
async fn resource_mismatch_is_rejected_with_no_events() {
    let (addr, ctx, captured) = spawn_mcp_router("resource-mismatch").await;
    record_canonical_uri(addr);
    register_test_client(&ctx);

    let resp = reqwest::Client::new()
        .get(authorize_url(
            addr,
            "https://not-this-server.example",
            "a-challenge",
            "S256",
        ))
        .send()
        .await
        .expect("request /authorize");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(captured.events().is_empty());
}

#[tokio::test]
async fn unknown_client_id_is_rejected_after_pkce_and_resource_pass() {
    let (addr, _ctx, captured) = spawn_mcp_router("unknown-client").await;
    let resource = record_canonical_uri(addr);
    // Deliberately no client registered — client_id will not resolve.

    let resp = reqwest::Client::new()
        .get(authorize_url(addr, &resource, "a-challenge", "S256"))
        .send()
        .await
        .expect("request /authorize");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(captured.events().is_empty());
}

#[tokio::test]
async fn valid_request_emits_consent_requested_and_raises_the_window_before_parking() {
    let (addr, ctx, captured) = spawn_mcp_router("valid-request").await;
    let resource = record_canonical_uri(addr);
    register_test_client(&ctx);

    tokio::spawn({
        let url = authorize_url(addr, &resource, "a-challenge", "S256");
        async move {
            let _ = reqwest::Client::new().get(url).send().await;
        }
    });

    let event = wait_for_event(&captured).await;
    match event {
        DomainEvent::McpConsentRequested {
            client_name,
            requested_scopes,
            resource: got_resource,
            ..
        } => {
            assert_eq!(client_name, "claude-desktop");
            assert_eq!(requested_scopes, vec![Scope::Read, Scope::Spend]);
            assert_eq!(got_resource, resource);
        }
        other => panic!("expected McpConsentRequested, got {other:?}"),
    }
    assert!(
        captured.window_raised(),
        "a valid /authorize request must raise the main window"
    );
}

#[tokio::test]
async fn approval_redirects_with_a_single_use_code() {
    let (addr, ctx, captured) = spawn_mcp_router("approval").await;
    let resource = record_canonical_uri(addr);
    register_test_client(&ctx);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build a non-redirecting client");
    let request = tokio::spawn({
        let url = authorize_url(addr, &resource, "a-challenge", "S256");
        let client = client.clone();
        async move { client.get(url).send().await }
    });

    let event = wait_for_event(&captured).await;
    let DomainEvent::McpConsentRequested { request_id, .. } = event else {
        panic!("expected McpConsentRequested");
    };
    ctx.mcp_consent
        .deliver(&request_id, ConsentDecision::Approved);

    let resp = request
        .await
        .expect("request task panicked")
        .expect("request /authorize");
    assert_eq!(resp.status(), reqwest::StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("redirect carries a Location header")
        .to_str()
        .expect("Location is ASCII");
    assert!(location.starts_with(CLIENT_REDIRECT_URI));
    assert!(location.contains("code="));
    assert!(!location.contains("error="));
}

#[tokio::test]
async fn denial_redirects_with_access_denied() {
    let (addr, ctx, captured) = spawn_mcp_router("denial").await;
    let resource = record_canonical_uri(addr);
    register_test_client(&ctx);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build a non-redirecting client");
    let request = tokio::spawn({
        let url = authorize_url(addr, &resource, "a-challenge", "S256");
        let client = client.clone();
        async move { client.get(url).send().await }
    });

    let event = wait_for_event(&captured).await;
    let DomainEvent::McpConsentRequested { request_id, .. } = event else {
        panic!("expected McpConsentRequested");
    };
    ctx.mcp_consent
        .deliver(&request_id, ConsentDecision::Denied);

    let resp = request
        .await
        .expect("request task panicked")
        .expect("request /authorize");
    assert_eq!(resp.status(), reqwest::StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("redirect carries a Location header")
        .to_str()
        .expect("Location is ASCII");
    assert!(location.contains("error=access_denied"));
}
