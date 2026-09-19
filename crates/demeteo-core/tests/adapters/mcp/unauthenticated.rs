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

/// `tools/list` needs no grant — it is discovery, the same posture as the
/// `.well-known` metadata routes (see `mcp_handler.rs` module docs).
#[tokio::test]
async fn unauthenticated_tools_list_returns_the_full_catalog() {
    let addr = spawn_mcp_router("unauthenticated-tools-list").await;

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }))
        .send()
        .await
        .expect("request /mcp")
        .error_for_status()
        .expect("tools/list returns 200 without a grant")
        .json()
        .await
        .expect("tools/list response is JSON");

    let tools = body["result"]["tools"]
        .as_array()
        .expect("result.tools is an array");
    assert_eq!(tools.len(), 12);
    assert!(tools
        .iter()
        .any(|t| t["name"] == "start_feature" && t["inputSchema"].is_object()));
}
