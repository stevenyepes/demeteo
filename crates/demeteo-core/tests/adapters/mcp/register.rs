// Tests extracted from `src/adapters/mcp/register.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::register`.

use std::sync::Arc;

use crate::adapters::mcp::router;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::ClientId;
use crate::state::AppContext;

/// Same shape as `tests/adapters/mcp/guard.rs`'s `fixture()`.
fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-register-{tag}-{}",
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

async fn spawn_mcp_router(tag: &str) -> (std::net::SocketAddr, AppContext) {
    let ctx = fixture(tag);
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

    (addr, ctx)
}

#[tokio::test]
async fn register_returns_a_client_id_and_never_a_client_secret() {
    let (addr, ctx) = spawn_mcp_router("register").await;

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{addr}/register"))
        .json(&serde_json::json!({
            "client_name": "claude-desktop",
            "redirect_uris": ["http://127.0.0.1:5173/callback"],
        }))
        .send()
        .await
        .expect("request /register")
        .error_for_status()
        .expect("valid registration returns a success status")
        .json()
        .await
        .expect("registration response is JSON");

    let client_id = body["client_id"]
        .as_str()
        .expect("response carries a client_id")
        .to_string();
    assert_eq!(body["client_name"], "claude-desktop");
    assert_eq!(
        body["redirect_uris"],
        serde_json::json!(["http://127.0.0.1:5173/callback"])
    );
    assert!(
        body.as_object()
            .expect("response body is a JSON object")
            .get("client_secret")
            .is_none(),
        "response must never carry a client_secret key: {body:?}"
    );

    let stored = ctx
        .oauth_clients
        .get_client(&ClientId::from(client_id.clone()))
        .expect("get_client does not error")
        .expect("the registered client is retrievable afterward");
    assert_eq!(stored.id, ClientId::from(client_id));
    assert_eq!(stored.client_name, "claude-desktop");
    assert_eq!(
        stored.redirect_uris,
        vec!["http://127.0.0.1:5173/callback".to_string()]
    );
}
