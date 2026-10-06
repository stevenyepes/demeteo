// Tests extracted from `src/adapters/mcp/mcp_handler.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::mcp_handler`.
//
// `start_feature`'s and `start_ticket`'s placement arguments: what the
// catalog advertises, and what a refused placement looks like to an MCP
// client.

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::adapters::mcp::{record_canonical_uri, router};
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::launch::tests::{harness, RunnerAt};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{ClientId, GrantId, ProjectId};
use crate::domain::models::Project;
use crate::domain::oauth::tools::required_scope;
use crate::domain::oauth::{GrantRecord, OAuthClient, Scope};
use crate::state::AppContext;

const PLACEMENT_ARGS: [&str; 5] = [
    "machine_id",
    "target_repo_id",
    "unattended",
    "max_cost_usd",
    "max_wall_clock_secs",
];

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
fn start_feature_schema_lists_the_placement_arguments_as_optional() {
    let entry = catalog_entry("start_feature");
    let schema = &entry["inputSchema"];
    let properties = schema["properties"]
        .as_object()
        .expect("inputSchema has properties");
    let required = required_names(schema);

    for name in PLACEMENT_ARGS {
        let property = properties
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not in the schema: {schema}"));
        assert!(
            property["description"]
                .as_str()
                .is_some_and(|d| !d.is_empty()),
            "{name} carries no description: {property}"
        );
        assert!(!required.contains(&name), "{name} must not be required");
    }
    for name in ["project_id", "workflow_id", "title", "description"] {
        assert!(required.contains(&name), "{name} is still required");
    }
}

#[test]
fn start_ticket_schema_lists_machine_id_as_optional() {
    let entry = catalog_entry("start_ticket");
    let schema = &entry["inputSchema"];
    let property = &schema["properties"]["machine_id"];
    let required = required_names(schema);

    assert!(
        property["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "machine_id is missing or undescribed: {schema}"
    );
    assert!(!required.contains(&"machine_id"), "machine_id is optional");
    assert!(
        required.contains(&"ticket_id"),
        "ticket_id is still required"
    );
}

#[test]
fn start_tools_keep_spend_scope_and_the_catalog_keeps_twelve_tools() {
    assert_eq!(required_scope("start_feature"), Some(Scope::Spend));
    assert_eq!(required_scope("start_ticket"), Some(Scope::Spend));
    assert_eq!(
        super::tool_catalog().as_array().map(Vec::len),
        Some(12),
        "machine_id is an argument, not a new tool"
    );
}

/// Same shape as `tests/adapters/mcp/scope_step_up.rs`'s `fixture()`.
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

/// See `tests/adapters/mcp/scope_step_up.rs`'s twin helper for why the
/// returned resource, not the listener's own address, is authoritative.
async fn spawn_mcp_router(tag: &str) -> (SocketAddr, String, AppContext) {
    let ctx = fixture(tag);
    let (addr, resource) = serve(ctx.clone()).await;
    (addr, resource, ctx)
}

async fn serve(ctx: AppContext) -> (SocketAddr, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener
        .local_addr()
        .expect("bound listener has a local address");
    let resource = record_canonical_uri(addr);

    let app = router(ctx);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    (addr, resource)
}

fn seed_spend_grant(ctx: &AppContext, token: &str, resource: &str) {
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
        scopes: vec![Scope::Spend],
        resource: resource.to_string(),
        issued_at: crate::paths::now_ms(),
        expires_at: crate::paths::now_ms() + 3_600_000,
        revoked_at: None,
    };
    ctx.oauth_grants
        .insert_grant(grant, &format!("{:x}", Sha256::digest(token.as_bytes())))
        .expect("insert test grant");
}

#[tokio::test]
async fn start_feature_on_an_unknown_machine_is_a_tool_error_naming_it() {
    let (addr, resource, ctx) = spawn_mcp_router("start-placement-unknown").await;
    let token = "token-spend-start-placement";
    seed_spend_grant(&ctx, token, &resource);
    let project_id = ProjectId::from("p-1");
    ctx.projects
        .add(Project {
            id: project_id.clone(),
            name: "placed".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .unwrap();

    let body: Value = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "start_feature",
                "arguments": {
                    "project_id": "p-1",
                    "workflow_id": "w-1",
                    "title": "Ship it",
                    "description": "Placed over MCP",
                    "machine_id": "ghost-box",
                },
            },
        }))
        .send()
        .await
        .expect("request /mcp")
        .json()
        .await
        .expect("response is JSON");

    assert_eq!(body["result"]["isError"], json!(true), "got: {body}");
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .expect("a tool error carries text");
    assert!(text.contains("ghost-box"), "got: {text}");
    assert!(
        ctx.features
            .get_all_for_project(&project_id)
            .unwrap()
            .is_empty(),
        "a refused start creates no Feature row"
    );
}

/// A `tools/call` of `name` against a router over `runner`'s harness, as a
/// `spend` grant. Returns the tool result.
async fn call_on(runner: RunnerAt, name: &str, arguments: Value) -> Value {
    let h = harness(runner);
    let (addr, resource) = serve(h.ctx.clone()).await;
    let token = "token-spend-start-degraded";
    seed_spend_grant(&h.ctx, token, &resource);

    let body: Value = reqwest::Client::new()
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
        .json()
        .await
        .expect("response is JSON");
    assert_eq!(body["result"]["isError"], json!(false), "got: {body}");
    body["result"]["structuredContent"].clone()
}

fn detached_start(machine_id: Option<&str>) -> Value {
    let mut arguments = json!({
        "project_id": "p-1",
        "workflow_id": "w-1",
        "title": "Ship it",
        "description": "Placed over MCP",
    });
    if let Some(id) = machine_id {
        arguments["machine_id"] = json!(id);
    }
    arguments
}

/// An accepted run whose PAT was not delivered is a success that says so:
/// the Feature's own fields where a client already reads them, and the note
/// beside them.
#[tokio::test]
async fn start_feature_reports_parked_credentials_beside_the_feature() {
    let parked = RunnerAt {
        accepts_credentials: false,
        ..RunnerAt::accepting()
    };

    let result = call_on(parked, "start_feature", detached_start(Some("runner-1"))).await;

    assert!(
        result["id"].as_str().is_some_and(|id| !id.is_empty()),
        "{result}"
    );
    assert_eq!(result["project_id"], json!("p-1"), "{result}");
    assert_eq!(result["title"], json!("Ship it"), "{result}");
    let note = result["credentials_parked"]
        .as_str()
        .unwrap_or_else(|| panic!("credentials_parked is a string: {result}"));
    assert!(note.contains("inject_credentials"), "{note}");
    assert!(result.get("mirror_unrecorded").is_none(), "{result}");
}

#[tokio::test]
async fn a_clean_start_feature_carries_no_degraded_state_key() {
    for machine_id in [Some("runner-1"), None] {
        let result = call_on(
            RunnerAt::accepting(),
            "start_feature",
            detached_start(machine_id),
        )
        .await;

        assert!(result["id"].as_str().is_some(), "{machine_id:?}: {result}");
        for key in ["credentials_parked", "mirror_unrecorded"] {
            assert!(
                result.get(key).is_none(),
                "{machine_id:?}: {key} in {result}"
            );
        }
    }
}
