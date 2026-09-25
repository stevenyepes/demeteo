// Tests extracted from `src/adapters/mcp/mcp_handler.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::mcp_handler`.

use std::sync::Arc;

use crate::adapters::mcp::router;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::state::AppContext;

/// Same shape as `tests/adapters/mcp/guard.rs`'s `fixture()`.
fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-handler-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    )
}

async fn spawn_mcp_router(tag: &str) -> std::net::SocketAddr {
    let ctx = fixture(tag);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener
        .local_addr()
        .expect("bound listener has a local address");

    tokio::spawn(async move {
        let _ = axum::serve(listener, router(ctx)).await;
    });

    addr
}

#[tokio::test]
async fn unauthenticated_tools_call_for_a_spend_tier_tool_returns_401_naming_spend() {
    let addr = spawn_mcp_router("unauthenticated").await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "start_feature", "arguments": {} },
        }))
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let challenge = resp
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .expect("401 carries a WWW-Authenticate header")
        .to_str()
        .expect("header value is ASCII");
    assert!(
        challenge.contains(r#"scope="spend""#),
        "challenge {challenge:?} does not name the real attempted scope"
    );
}

/// The handshake needs a grant too: OpenCode and Hermes start OAuth only when
/// connecting is refused. The challenge names no scope, since nothing was
/// attempted — the client falls back to `scopes_supported`.
#[tokio::test]
async fn unauthenticated_handshake_is_refused_with_a_scopeless_challenge() {
    let addr = spawn_mcp_router("unauthenticated-handshake").await;

    for method in ["initialize", "tools/list"] {
        let resp = reqwest::Client::new()
            .post(format!("http://{addr}/mcp"))
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": {},
            }))
            .send()
            .await
            .expect("request /mcp");

        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED, "{method}");
        let challenge = resp
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .expect("401 carries a WWW-Authenticate header")
            .to_str()
            .expect("header value is ASCII");
        assert!(
            challenge.starts_with("Bearer resource_metadata="),
            "{method}: {challenge:?}"
        );
        assert!(!challenge.contains("scope="), "{method}: {challenge:?}");
    }
}
