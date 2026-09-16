// Tests extracted from `src/adapters/mcp/metadata.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::metadata`.

use std::sync::Arc;

use crate::adapters::mcp::{record_canonical_uri, router};
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::state::AppContext;

/// A fully wired `AppContext` over a fresh temp-dir SQLite database — same
/// shape as `tests/application/agent_surface.rs`'s `fixture()`. Neither
/// `.well-known` handler reads from it today, but the router-builder takes
/// one so later tickets' routes (`/authorize`, `/token`, `/mcp`) can.
fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-metadata-{tag}-{}",
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

#[tokio::test]
async fn well_known_endpoints_return_rfc_shapes() {
    let ctx = fixture("well-known");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener
        .local_addr()
        .expect("bound listener has a local address");
    let expected_uri = record_canonical_uri(addr);

    tokio::spawn(async move {
        let _ = axum::serve(listener, router(ctx)).await;
    });

    let client = reqwest::Client::new();

    let prm: serde_json::Value = client
        .get(format!(
            "http://{addr}/.well-known/oauth-protected-resource"
        ))
        .send()
        .await
        .expect("request the protected-resource metadata endpoint")
        .error_for_status()
        .expect("protected-resource metadata endpoint returns 200")
        .json()
        .await
        .expect("protected-resource metadata response is JSON");

    assert_eq!(prm["resource"], expected_uri);
    assert_eq!(prm["scopes_supported"], serde_json::json!(["read"]));

    let asm: serde_json::Value = client
        .get(format!(
            "http://{addr}/.well-known/oauth-authorization-server"
        ))
        .send()
        .await
        .expect("request the authorization-server metadata endpoint")
        .error_for_status()
        .expect("authorization-server metadata endpoint returns 200")
        .json()
        .await
        .expect("authorization-server metadata response is JSON");

    assert_eq!(asm["issuer"], expected_uri);
    assert_eq!(
        asm["authorization_endpoint"],
        format!("{expected_uri}/authorize")
    );
    assert_eq!(asm["token_endpoint"], format!("{expected_uri}/token"));
    assert_eq!(
        asm["registration_endpoint"],
        format!("{expected_uri}/register")
    );
    assert_eq!(
        asm["code_challenge_methods_supported"],
        serde_json::json!(["S256"])
    );
}
