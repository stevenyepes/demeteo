// Tests extracted from `src/adapters/mcp/protocol.rs` (mirrored-tests
// convention). Boots the real `router()` — not `enforce_headers` in
// isolation — since header/status-code/body behavior is what's under test.

use std::sync::Arc;

use crate::adapters::mcp::router;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::state::AppContext;

fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-protocol-{tag}-{}",
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

async fn spawn_protocol_router(tag: &str) -> std::net::SocketAddr {
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
async fn mcp_name_header_mismatching_body_returns_400_with_header_mismatch_code() {
    let addr = spawn_protocol_router("name-mismatch").await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("Mcp-Name", "not_list_projects")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "list_projects", "arguments": {} },
        }))
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.expect("JSON body");
    assert_eq!(body["error"]["code"], -32020);
}

#[tokio::test]
async fn mcp_method_header_mismatching_body_returns_400_with_header_mismatch_code() {
    let addr = spawn_protocol_router("method-mismatch").await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("Mcp-Method", "tools/list")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "list_projects", "arguments": {} },
        }))
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = resp.json().await.expect("JSON body");
    assert_eq!(body["error"]["code"], -32020);
}

// A client's post-handshake requests carry whatever version `initialize`
// negotiated — which may not be `2026-07-28` (see the two `initialize`
// tests below). Demeteo cannot remember what a prior request on the same
// TCP connection negotiated (this transport is genuinely stateless), so a
// declared `MCP-Protocol-Version` header is no longer checked for exact
// equality; only its absence-vs-presence ever mattered for the header-body
// mismatch checks, which this test is not exercising.
#[tokio::test]
async fn declared_protocol_version_header_does_not_block_dispatch() {
    let addr = spawn_protocol_router("declared-version").await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("MCP-Protocol-Version", "2024-01-01")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {},
        }))
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("JSON body");
    assert!(body["result"]["tools"].is_array());
}

#[tokio::test]
async fn initialize_echoes_the_clients_declared_protocol_version() {
    let addr = spawn_protocol_router("initialize-echo").await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "initialize",
            "params": { "protocolVersion": "2025-11-25" },
        }))
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("JSON body");
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        body["result"]["capabilities"]["tools"],
        serde_json::json!({})
    );
}

#[tokio::test]
async fn initialize_falls_back_to_the_supported_version_when_absent_or_malformed() {
    let addr = spawn_protocol_router("initialize-fallback").await;
    let client = reqwest::Client::new();

    let absent = client
        .post(format!("http://{addr}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {},
        }))
        .send()
        .await
        .expect("request /mcp")
        .json::<serde_json::Value>()
        .await
        .expect("JSON body");
    assert_eq!(absent["result"]["protocolVersion"], "2026-07-28");

    let malformed = client
        .post(format!("http://{addr}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "initialize",
            "params": { "protocolVersion": "not-a-version" },
        }))
        .send()
        .await
        .expect("request /mcp")
        .json::<serde_json::Value>()
        .await
        .expect("JSON body");
    assert_eq!(malformed["result"]["protocolVersion"], "2026-07-28");
}

#[tokio::test]
async fn server_discover_returns_200_naming_the_supported_version() {
    let addr = spawn_protocol_router("discover").await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "server/discover",
            "params": {},
        }))
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("JSON body");
    assert_eq!(body["result"]["protocolVersion"], "2026-07-28");
    assert_eq!(
        body["result"]["supported"],
        serde_json::json!(["2026-07-28"])
    );
}

#[tokio::test]
async fn get_on_mcp_returns_405() {
    let addr = spawn_protocol_router("get-405").await;

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/mcp"))
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn delete_on_mcp_returns_405() {
    let addr = spawn_protocol_router("delete-405").await;

    let resp = reqwest::Client::new()
        .delete(format!("http://{addr}/mcp"))
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
}

// Regression coverage for the pre-auth body-size cap: `enforce_headers`
// buffered the whole body via `to_bytes(body, usize::MAX)` ahead of
// `mcp_handler`'s own size-limited `Json` extractor, so a body larger than
// `super::MAX_BODY_BYTES` must still be rejected rather than fully buffered
// into memory.
#[tokio::test]
async fn oversized_body_on_mcp_is_rejected_with_payload_too_large() {
    let addr = spawn_protocol_router("oversized-body").await;

    let oversized = vec![b'a'; super::MAX_BODY_BYTES + 1];
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .body(oversized)
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
}
