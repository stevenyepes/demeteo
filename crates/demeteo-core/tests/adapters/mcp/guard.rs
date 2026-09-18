// Tests extracted from `src/adapters/mcp/guard.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::guard`; `use super::*` also pulls in
// `guard`'s own private imports (`AppContext`, `GrantRecord`, `OAuthError`,
// `Scope`, `validate_grant`, `canonical_uri`) the same way
// `tests/domain/oauth/validate_grant.rs` does.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::middleware;
use axum::routing::get;
use axum::Router;

use super::*;

use crate::adapters::mcp::record_canonical_uri;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{ClientId, GrantId};
use crate::domain::oauth::OAuthClient;

/// Same shape as `tests/adapters/mcp/metadata.rs`'s `fixture()`: a fully
/// wired `AppContext` over a fresh temp-dir SQLite database, so
/// `ctx.oauth_clients` / `ctx.oauth_grants` are real repositories a grant can
/// be seeded into directly.
fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-guard-{tag}-{}",
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

async fn ok_handler() -> &'static str {
    "ok"
}

/// Boots `/protected`, guarded by `guard::layer` at `required`, on its own
/// ephemeral loopback listener. Returns that listener's address alongside
/// the canonical resource URI every seeded grant in this file must match to
/// pass the audience check: `canonical_uri()` backs onto a process-wide
/// `OnceLock` shared with every other test in this `--lib` binary (see
/// `adapters/mcp/mod.rs::record_canonical_uri`), so the authoritative value
/// is whatever `record_canonical_uri` returns here — not necessarily this
/// listener's own address, if some other test's listener won the race to
/// initialize it first.
async fn spawn_guarded_route(tag: &str, required: Scope) -> (SocketAddr, String, AppContext) {
    let ctx = fixture(tag);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener
        .local_addr()
        .expect("bound listener has a local address");
    let resource = record_canonical_uri(addr);

    let app = Router::new()
        .route("/protected", get(ok_handler))
        .route_layer(middleware::from_fn_with_state(
            (ctx.clone(), required),
            layer,
        ));

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (addr, resource, ctx)
}

/// Registers a client and a grant bearing `token`'s hash directly through
/// the repositories `check` reads from — the same seam a real `/token`
/// response would populate, skipped here since this ticket predates it.
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

#[tokio::test]
async fn missing_header_returns_401_with_scope_aware_challenge() {
    let (addr, resource, _ctx) = spawn_guarded_route("missing-header", Scope::Spend).await;

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/protected"))
        .send()
        .await
        .expect("request the guarded route");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    let challenge = resp
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .expect("401 carries a WWW-Authenticate header")
        .to_str()
        .expect("header value is ASCII");
    assert!(challenge.contains(&format!(
        r#"resource_metadata="{resource}/.well-known/oauth-protected-resource""#
    )));
    assert!(challenge.contains(r#"scope="spend""#));
}

#[tokio::test]
async fn audience_mismatched_grant_returns_401() {
    let (addr, _resource, ctx) = spawn_guarded_route("audience-mismatch", Scope::Spend).await;
    let token = "token-audience-mismatch";
    seed_grant(
        &ctx,
        token,
        &[Scope::Spend],
        "https://mismatched.example/mcp",
        crate::paths::now_ms() + 3_600_000,
        None,
    );

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/protected"))
        .bearer_auth(token)
        .send()
        .await
        .expect("request the guarded route");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_grant_returns_401() {
    let (addr, resource, ctx) = spawn_guarded_route("expired", Scope::Spend).await;
    let token = "token-expired";
    seed_grant(
        &ctx,
        token,
        &[Scope::Spend],
        &resource,
        crate::paths::now_ms() - 1,
        None,
    );

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/protected"))
        .bearer_auth(token)
        .send()
        .await
        .expect("request the guarded route");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn revoked_grant_returns_401() {
    let (addr, resource, ctx) = spawn_guarded_route("revoked", Scope::Spend).await;
    let token = "token-revoked";
    seed_grant(
        &ctx,
        token,
        &[Scope::Spend],
        &resource,
        crate::paths::now_ms() + 3_600_000,
        Some(crate::paths::now_ms()),
    );

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/protected"))
        .bearer_auth(token)
        .send()
        .await
        .expect("request the guarded route");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn read_only_grant_gets_403_insufficient_scope() {
    let (addr, resource, ctx) = spawn_guarded_route("insufficient-scope", Scope::Spend).await;
    let token = "token-read-only";
    seed_grant(
        &ctx,
        token,
        &[Scope::Read],
        &resource,
        crate::paths::now_ms() + 3_600_000,
        None,
    );

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/protected"))
        .bearer_auth(token)
        .send()
        .await
        .expect("request the guarded route");

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

#[tokio::test]
async fn the_bearer_scheme_is_case_insensitive() {
    let (addr, resource, ctx) = spawn_guarded_route("bearer-case", Scope::Read).await;
    let token = "token-case";
    seed_grant(
        &ctx,
        token,
        &[Scope::Read],
        &resource,
        crate::paths::now_ms() + 3_600_000,
        None,
    );

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/protected"))
        .header(reqwest::header::AUTHORIZATION, format!("bearer {token}"))
        .send()
        .await
        .expect("request the guarded route");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn matching_grant_passes_through_to_the_handler() {
    let (addr, resource, ctx) = spawn_guarded_route("passthrough", Scope::Spend).await;
    let token = "token-valid";
    seed_grant(
        &ctx,
        token,
        &[Scope::Read, Scope::Spend],
        &resource,
        crate::paths::now_ms() + 3_600_000,
        None,
    );

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/protected"))
        .bearer_auth(token)
        .send()
        .await
        .expect("request the guarded route");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.expect("response body is text"), "ok");
}
