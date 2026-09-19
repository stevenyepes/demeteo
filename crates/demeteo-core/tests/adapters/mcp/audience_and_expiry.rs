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

fn seed_grant(
    ctx: &AppContext,
    token: &str,
    scopes: &[Scope],
    resource: &str,
    expires_at: i64,
    revoked_at: Option<i64>,
) {
    let client = OAuthClient {
        id: ClientId::new("client-1"),
        client_name: "test-client".to_string(),
        redirect_uris: vec![],
        created_at: crate::paths::now_ms(),
    };
    ctx.oauth_clients
        .register_client(client.clone())
        .expect("register test client");

    let grant = GrantRecord {
        id: GrantId::new("grant-1"),
        client_id: client.id,
        scopes: scopes.to_vec(),
        resource: resource.to_string(),
        issued_at: crate::paths::now_ms(),
        expires_at,
        revoked_at,
    };
    ctx.oauth_grants
        .insert_grant(grant, &hash_token(token))
        .expect("insert test grant");
}

async fn call_start_feature(addr: SocketAddr, token: &str) -> reqwest::StatusCode {
    reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "start_feature", "arguments": {} },
        }))
        .send()
        .await
        .expect("request /mcp")
        .status()
}

#[tokio::test]
async fn audience_mismatched_grant_returns_401() {
    let (addr, _resource, ctx) = spawn_mcp_router("audience-mismatch").await;
    let token = "token-audience-mismatch";
    seed_grant(
        &ctx,
        token,
        &[Scope::Spend],
        "https://mismatched.example/mcp",
        crate::paths::now_ms() + 3_600_000,
        None,
    );

    assert_eq!(
        call_start_feature(addr, token).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn expired_grant_returns_401() {
    let (addr, resource, ctx) = spawn_mcp_router("expired").await;
    let token = "token-expired";
    seed_grant(
        &ctx,
        token,
        &[Scope::Spend],
        &resource,
        crate::paths::now_ms() - 1,
        None,
    );

    assert_eq!(
        call_start_feature(addr, token).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
}
