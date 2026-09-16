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

/// Boots the real `/mcp` router on its own ephemeral loopback listener.
/// Returns that listener's address alongside the canonical resource URI
/// every seeded grant in this file must match — see `tests/adapters/mcp/
/// guard.rs`'s `spawn_guarded_route` for why `canonical_uri()`'s
/// process-wide `OnceLock` makes this the authoritative value, not
/// necessarily this listener's own address.
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

async fn call_start_feature(addr: SocketAddr, token: &str) -> reqwest::Response {
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
}

#[tokio::test]
async fn read_only_grant_calling_a_spend_tool_returns_403_insufficient_scope() {
    let (addr, resource, ctx) = spawn_mcp_router("scope-step-up").await;
    let token = "token-read-only";
    seed_grant(
        &ctx,
        token,
        &[Scope::Read],
        &resource,
        crate::paths::now_ms() + 3_600_000,
        None,
    );

    let resp = call_start_feature(addr, token).await;

    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
    let challenge = resp
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .expect("403 carries a WWW-Authenticate header")
        .to_str()
        .expect("header value is ASCII");
    assert!(challenge.contains(r#"error="insufficient_scope""#));
    assert!(challenge.contains(r#"scope="spend""#));
}
