// Tests extracted from `src/adapters/mcp/mcp_handler.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::mcp_handler`.

use std::net::SocketAddr;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::adapters::mcp::{record_canonical_uri, router};
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{ClientId, GrantId};
use crate::domain::oauth::{GrantRecord, OAuthClient, Scope};
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

fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// See `tests/adapters/mcp/scope_step_up.rs`'s twin helper for why the
/// returned resource, not the listener's own address, is authoritative.
async fn spawn_mcp_router(tag: &str) -> (SocketAddr, String, AppContext) {
    let ctx = fixture(tag);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener
        .local_addr()
        .expect("bound listener has a local address");
    let resource = record_canonical_uri(addr);

    let app = router(ctx.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (addr, resource, ctx)
}

/// `list_projects` (`read`-tier, no side effects) rather than `start_feature`:
/// this test's "succeeds" step needs a call that genuinely completes against
/// a fresh, empty database, not a spend operation that would need a real
/// project/workflow fixture to avoid an application-layer error that would
/// be indistinguishable from the 401 this test is trying to isolate.
async fn call_list_projects(addr: SocketAddr, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "list_projects", "arguments": {} },
        }))
        .send()
        .await
        .expect("request /mcp")
}

#[tokio::test]
async fn revoking_a_grant_takes_effect_on_the_very_next_request() {
    let (addr, resource, ctx) = spawn_mcp_router("revocation").await;
    let token = "token-revocation";
    let grant_id = GrantId::new("grant-1");

    let client = OAuthClient {
        id: ClientId::new("client-1"),
        client_name: "test-client".to_string(),
        redirect_uris: vec![],
        created_at: crate::paths::now_ms(),
    };
    ctx.oauth_clients
        .register_client(client.clone())
        .expect("register test client");
    ctx.oauth_grants
        .insert_grant(
            GrantRecord {
                id: grant_id.clone(),
                client_id: client.id,
                scopes: vec![Scope::Read],
                resource,
                issued_at: crate::paths::now_ms(),
                expires_at: crate::paths::now_ms() + 3_600_000,
                revoked_at: None,
            },
            &hash_token(token),
        )
        .expect("insert active grant");

    let first = call_list_projects(addr, token).await;
    assert_eq!(
        first.status(),
        reqwest::StatusCode::OK,
        "the active grant should authorize the first call"
    );
    let first_body: serde_json::Value = first.json().await.expect("first response is JSON");
    assert_eq!(first_body["result"]["isError"], false);

    ctx.oauth_grants
        .revoke_grant(&grant_id)
        .expect("revoke the grant directly through the repository, no restart");

    let second = call_list_projects(addr, token).await;
    assert_eq!(
        second.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "the very next request with the same token must be rejected, with no restart or cache-clear"
    );
}
