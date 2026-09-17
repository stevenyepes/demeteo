// Tests extracted from `src/adapters/mcp/origin.rs` (mirrored-tests
// convention). Boots the real `router()` — not `super::enforce` in
// isolation — since header/status-code behavior is what's under test.

use std::sync::Arc;

use crate::adapters::mcp::{record_canonical_uri, router};
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::state::AppContext;

/// An `Origin` no seeded fixture ever matches — `canonical_uri()` is always
/// an `http://127.0.0.1:PORT` value (see `mod.rs::canonical_uri_for`), never
/// this.
const DISALLOWED_ORIGIN: &str = "https://evil.example";

fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-origin-{tag}-{}",
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

/// Boots the real router on its own ephemeral loopback listener. Returns
/// that listener's address alongside the process-wide canonical URI every
/// Origin check compares against — not necessarily this listener's own
/// address, since `canonical_uri()` backs onto a `--lib`-binary-wide static
/// that the first test to prime it wins (see
/// `adapters/mcp/mod.rs::record_canonical_uri`, and `tests/adapters/mcp/
/// guard.rs`'s `spawn_guarded_route` for the same pattern).
async fn spawn_origin_router(tag: &str) -> (std::net::SocketAddr, String) {
    let ctx = fixture(tag);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener
        .local_addr()
        .expect("bound listener has a local address");
    let resource = record_canonical_uri(addr);

    tokio::spawn(async move {
        let _ = axum::serve(listener, router(ctx)).await;
    });

    (addr, resource)
}

fn tools_list_body() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {},
    })
}

#[tokio::test]
async fn disallowed_origin_on_mcp_post_returns_403() {
    let (addr, _resource) = spawn_origin_router("disallowed-mcp").await;

    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header(reqwest::header::ORIGIN, DISALLOWED_ORIGIN)
        .json(&tools_list_body())
        .send()
        .await
        .expect("request /mcp");

    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn disallowed_origin_on_well_known_get_returns_403() {
    let (addr, _resource) = spawn_origin_router("disallowed-well-known").await;

    let resp = reqwest::Client::new()
        .get(format!(
            "http://{addr}/.well-known/oauth-protected-resource"
        ))
        .header(reqwest::header::ORIGIN, DISALLOWED_ORIGIN)
        .send()
        .await
        .expect("request the protected-resource metadata endpoint");

    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn missing_origin_header_proceeds_on_mcp_and_well_known() {
    let (addr, _resource) = spawn_origin_router("missing-origin").await;

    let mcp_resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .json(&tools_list_body())
        .send()
        .await
        .expect("request /mcp");
    assert_eq!(mcp_resp.status(), reqwest::StatusCode::OK);

    let well_known_resp = reqwest::Client::new()
        .get(format!(
            "http://{addr}/.well-known/oauth-protected-resource"
        ))
        .send()
        .await
        .expect("request the protected-resource metadata endpoint");
    assert_eq!(well_known_resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn origin_matching_canonical_uri_proceeds_on_mcp_and_well_known() {
    let (addr, resource) = spawn_origin_router("matching-origin").await;

    let mcp_resp = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header(reqwest::header::ORIGIN, &resource)
        .json(&tools_list_body())
        .send()
        .await
        .expect("request /mcp");
    assert_eq!(mcp_resp.status(), reqwest::StatusCode::OK);

    let well_known_resp = reqwest::Client::new()
        .get(format!(
            "http://{addr}/.well-known/oauth-protected-resource"
        ))
        .header(reqwest::header::ORIGIN, &resource)
        .send()
        .await
        .expect("request the protected-resource metadata endpoint");
    assert_eq!(well_known_resp.status(), reqwest::StatusCode::OK);
}
