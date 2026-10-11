// Tests extracted from `src/adapters/mcp/mcp_handler.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::mcp_handler`.
//
// `retry_step`, `set_step_assignment` and `rework_from_critic`: what the
// catalog advertises, which grant reaches them, and what a refusal looks like
// to an MCP client. What each one does is covered at the seam, in
// `tests/application/agent_surface_writes.rs`.

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::adapters::mcp::{record_canonical_uri, router};
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{ClientId, GrantId};
use crate::domain::oauth::{GrantRecord, OAuthClient, Scope};
use crate::state::AppContext;

const STEP_TOOLS: [&str; 3] = ["retry_step", "set_step_assignment", "rework_from_critic"];

fn catalog_entry(name: &str) -> Value {
    super::tool_catalog()
        .as_array()
        .expect("tool_catalog() returns a JSON array")
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("{name} is in the catalog"))
        .clone()
}

fn required_names(schema: &Value) -> Vec<&str> {
    schema["required"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

#[test]
fn each_step_tool_requires_the_step_and_nothing_else() {
    for name in STEP_TOOLS {
        let entry = catalog_entry(name);
        assert_eq!(
            required_names(&entry["inputSchema"]),
            ["step_execution_id"],
            "{name}"
        );
    }
}

/// A feedback or decision argument on this tool would be text an outside
/// caller wrote reaching the rework agents, or a route to approving a gate.
#[test]
fn rework_from_critic_takes_no_argument_but_the_gate() {
    let entry = catalog_entry("rework_from_critic");
    let properties: Vec<&String> = entry["inputSchema"]["properties"]
        .as_object()
        .expect("inputSchema has properties")
        .keys()
        .collect();

    assert_eq!(properties, ["step_execution_id"]);
}

#[test]
fn an_assignment_advertises_its_three_dimensions_as_optional_and_described() {
    let entry = catalog_entry("set_step_assignment");
    let properties = entry["inputSchema"]["properties"]
        .as_object()
        .expect("inputSchema has properties");

    for name in ["agent_kind", "model", "effort"] {
        let property = properties
            .get(name)
            .unwrap_or_else(|| panic!("{name} is advertised"));
        assert!(
            property["description"]
                .as_str()
                .is_some_and(|d| !d.is_empty()),
            "{name} is described"
        );
    }
}

async fn serve(tag: &str) -> (SocketAddr, String, AppContext) {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-mcp-step-control-{tag}"));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
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

fn seed_grant(ctx: &AppContext, scope: Scope, token: &str, resource: &str) {
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
        scopes: vec![scope],
        resource: resource.to_string(),
        issued_at: crate::paths::now_ms(),
        expires_at: crate::paths::now_ms() + 3_600_000,
        revoked_at: None,
    };
    ctx.oauth_grants
        .insert_grant(grant, &format!("{:x}", Sha256::digest(token.as_bytes())))
        .expect("insert test grant");
}

async fn call(addr: SocketAddr, token: &str, name: &str, arguments: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        }))
        .send()
        .await
        .expect("request /mcp")
}

#[tokio::test]
async fn a_read_or_configure_grant_reaches_none_of_the_step_tools() {
    for scope in [Scope::Read, Scope::Configure] {
        let (addr, resource, ctx) = serve("insufficient").await;
        seed_grant(&ctx, scope, "token-not-spend", &resource);

        for name in STEP_TOOLS {
            let response = call(
                addr,
                "token-not-spend",
                name,
                json!({ "step_execution_id": "se-1" }),
            )
            .await;
            assert_eq!(response.status(), 403, "{name} with {scope:?}");
        }
    }
}

#[tokio::test]
async fn a_refusal_is_a_tool_error_and_not_a_protocol_error() {
    let (addr, resource, ctx) = serve("refused").await;
    seed_grant(&ctx, Scope::Spend, "token-spend", &resource);

    for (name, arguments, expected) in [
        (
            "retry_step",
            json!({ "step_execution_id": "se-missing" }),
            "not found",
        ),
        (
            "rework_from_critic",
            json!({ "step_execution_id": "se-missing" }),
            "not found",
        ),
        (
            "set_step_assignment",
            json!({ "step_execution_id": "se-missing", "model": "--yolo" }),
            "must not start with `-`",
        ),
    ] {
        let body: Value = call(addr, "token-spend", name, arguments)
            .await
            .json()
            .await
            .expect("response is JSON");
        assert_eq!(body["result"]["isError"], json!(true), "{name}: {body}");
        let text = body["result"]["content"][0]["text"]
            .as_str()
            .expect("a tool error carries text");
        assert!(text.contains(expected), "{name}: {text}");
    }
}
