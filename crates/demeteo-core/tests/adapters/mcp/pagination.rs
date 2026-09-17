// Tests extracted from `src/adapters/mcp/mcp_handler.rs` (mirrored-tests
// convention). `super` = `adapters::mcp::mcp_handler`.
//
// AC5 (implementation-spec.md): each of the six list-shaped tools accepts
// `limit`/`cursor` and wraps its result in `{items, truncated, next_cursor}`
// (nested under `tickets` for `get_discovery_board`).

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::adapters::mcp::{record_canonical_uri, router};
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::discovery::{create as open_discovery, NewDiscovery};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{
    ClientId, DiscoveryId, FeatureId, GateDecisionId, GrantId, ProjectId, StepExecutionId, StepId,
    TicketId,
};
use crate::domain::models::{Feature, GateDecision, Project, StepExecution, Ticket, TicketState};
use crate::domain::oauth::{GrantRecord, OAuthClient, Scope};
use crate::state::AppContext;

use super::{paginate, DispatchError, PageArgs};

fn page_args(limit: Option<u32>, cursor: Option<&str>) -> PageArgs {
    PageArgs {
        limit,
        cursor: cursor.map(str::to_string),
    }
}

/// Unit-level, no listener: the boundary cases of `paginate` itself, in
/// isolation from any tool's HTTP surface.
#[test]
fn paginate_covers_the_boundary_cases() {
    struct Case {
        name: &'static str,
        total: u32,
        limit: Option<u32>,
        cursor: Option<&'static str>,
        want_len: usize,
        want_truncated: bool,
        want_next_cursor: Option<&'static str>,
    }

    let cases = [
        Case {
            name: "under limit",
            total: 10,
            limit: Some(50),
            cursor: None,
            want_len: 10,
            want_truncated: false,
            want_next_cursor: None,
        },
        Case {
            name: "exactly at limit",
            total: 50,
            limit: None,
            cursor: None,
            want_len: 50,
            want_truncated: false,
            want_next_cursor: None,
        },
        Case {
            name: "over limit uses the default page size",
            total: 55,
            limit: None,
            cursor: None,
            want_len: 50,
            want_truncated: true,
            want_next_cursor: Some("50"),
        },
        Case {
            name: "an over-large limit clamps to MAX_PAGE_LIMIT rather than erroring",
            total: 250,
            limit: Some(10_000),
            cursor: None,
            want_len: 200,
            want_truncated: true,
            want_next_cursor: Some("200"),
        },
        Case {
            name: "a cursor past the end yields an empty, non-truncated page",
            total: 10,
            limit: Some(50),
            cursor: Some("999"),
            want_len: 0,
            want_truncated: false,
            want_next_cursor: None,
        },
    ];

    for case in cases {
        let items: Vec<u32> = (0..case.total).collect();
        let page = paginate(items, &page_args(case.limit, case.cursor))
            .unwrap_or_else(|_| panic!("{}: paginate should not error", case.name));
        assert_eq!(page.items.len(), case.want_len, "{}: item count", case.name);
        assert_eq!(
            page.truncated, case.want_truncated,
            "{}: truncated",
            case.name
        );
        assert_eq!(
            page.next_cursor.as_deref(),
            case.want_next_cursor,
            "{}: next_cursor",
            case.name
        );
    }
}

#[test]
fn paginate_rejects_an_unparseable_cursor_instead_of_resetting_to_page_one() {
    let items: Vec<u32> = vec![1, 2, 3];
    match paginate(items, &page_args(None, Some("not-a-number"))) {
        Err(DispatchError::InvalidParams(_)) => {}
        Err(DispatchError::Failed(_)) => panic!("expected InvalidParams, got Failed"),
        Ok(_) => panic!("an unparseable cursor should error"),
    }
}

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

fn seed_read_grant(ctx: &AppContext, token: &str, resource: &str) {
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
        scopes: vec![Scope::Read],
        resource: resource.to_string(),
        issued_at: crate::paths::now_ms(),
        expires_at: crate::paths::now_ms() + 3_600_000,
        revoked_at: None,
    };
    ctx.oauth_grants
        .insert_grant(grant, &hash_token(token))
        .expect("insert test grant");
}

async fn call_tool(addr: SocketAddr, token: &str, name: &str, arguments: Value) -> Value {
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
        .json::<Value>()
        .await
        .expect("response is JSON")
}

fn project(id: &str) -> Project {
    Project {
        id: ProjectId::from(id.to_string()),
        name: format!("project {id}"),
        compute_type: "local".to_string(),
        remote_host: None,
        status: "idle".to_string(),
        nodes: 0,
        spend: 0.0,
        tokens: 0,
        created_at: 0,
    }
}

fn feature(id: &str, project_id: &ProjectId) -> Feature {
    Feature {
        id: FeatureId::from(id.to_string()),
        project_id: project_id.clone(),
        workflow_id: None,
        workflow_version_id: None,
        title: format!("feature {id}"),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        duration: String::new(),
        tokens: 0,
        created_at: 0,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: Some("none".to_string()),
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    }
}

fn step(id: &str, feature_id: &FeatureId, index: u32) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from(id.to_string()),
        feature_id: feature_id.clone(),
        step_id: StepId::from(format!("s-{id}")),
        step_index: index,
        step_kind: "agent".to_string(),
        status: "completed".to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn ticket(id: &str, discovery_id: &DiscoveryId, seq: i64) -> Ticket {
    Ticket {
        id: TicketId::from(id.to_string()),
        discovery_id: discovery_id.clone(),
        seq,
        title: format!("ticket {seq}"),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        attachments: Vec::new(),
        state: TicketState::Unstarted,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn opening(project_id: &ProjectId, title: &str) -> NewDiscovery {
    NewDiscovery {
        project_id: project_id.as_str().to_string(),
        title: title.to_string(),
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        machine_id: None,
        staged_attachments: Vec::new(),
    }
}

/// Asserts the common over-limit → follow-cursor → exhausted shape shared by
/// every list-shaped tool: page 1 is full and `truncated`, its `next_cursor`
/// fetches exactly the remainder, and the remainder is `truncated: false`
/// with no further cursor.
async fn assert_pages_to_exhaustion(
    addr: SocketAddr,
    token: &str,
    tool: &str,
    base_args: Value,
    total: usize,
    page_of: impl Fn(&Value) -> &Value,
) {
    let page1 = call_tool(addr, token, tool, base_args.clone()).await;
    let content = page_of(&page1["result"]["structuredContent"]);
    let first_len = content["items"]
        .as_array()
        .expect("items is an array")
        .len();
    assert_eq!(
        first_len, 50,
        "page 1 for {tool} did not use the default limit"
    );
    assert_eq!(
        content["truncated"],
        json!(true),
        "{tool} page 1 not truncated"
    );
    let cursor = content["next_cursor"]
        .as_str()
        .unwrap_or_else(|| panic!("{tool} page 1 has no next_cursor"))
        .to_string();

    let mut next_args = base_args.as_object().cloned().unwrap_or_default();
    next_args.insert("cursor".to_string(), json!(cursor));
    let page2 = call_tool(addr, token, tool, Value::Object(next_args)).await;
    let content2 = page_of(&page2["result"]["structuredContent"]);
    let second_len = content2["items"]
        .as_array()
        .expect("items is an array")
        .len();
    assert_eq!(second_len, total - 50, "{tool} follow-up page missed rows");
    assert_eq!(
        content2["truncated"],
        json!(false),
        "{tool} last page still truncated"
    );
    assert!(
        content2["next_cursor"].is_null(),
        "{tool} last page still has a cursor"
    );
}

fn top_level(v: &Value) -> &Value {
    v
}

#[tokio::test]
async fn list_projects_paginates_to_exhaustion() {
    let (addr, resource, ctx) = spawn_mcp_router("pagination-projects").await;
    let token = "token-projects";
    seed_read_grant(&ctx, token, &resource);
    for i in 0..55 {
        ctx.projects.add(project(&format!("p-{i:03}"))).unwrap();
    }

    assert_pages_to_exhaustion(addr, token, "list_projects", json!({}), 55, top_level).await;
}

#[tokio::test]
async fn list_features_paginates_to_exhaustion() {
    let (addr, resource, ctx) = spawn_mcp_router("pagination-features").await;
    let token = "token-features";
    seed_read_grant(&ctx, token, &resource);
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    for i in 0..55 {
        ctx.features
            .add(feature(&format!("f-{i:03}"), &project_id))
            .unwrap();
    }

    assert_pages_to_exhaustion(
        addr,
        token,
        "list_features",
        json!({ "project_id": project_id.as_str() }),
        55,
        top_level,
    )
    .await;
}

#[tokio::test]
async fn list_step_attempts_paginates_to_exhaustion() {
    let (addr, resource, ctx) = spawn_mcp_router("pagination-attempts").await;
    let token = "token-attempts";
    seed_read_grant(&ctx, token, &resource);
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-1".to_string());
    ctx.features.add(feature("f-1", &project_id)).unwrap();
    let step_execution_id = StepExecutionId::from("se-1".to_string());
    ctx.features
        .step_create(step("se-1", &feature_id, 0))
        .unwrap();
    for i in 0..55i64 {
        let attempt_no = ctx
            .features
            .attempt_open(&step_execution_id, i, None)
            .unwrap();
        ctx.features
            .attempt_close(
                &step_execution_id,
                attempt_no,
                "completed",
                0.0,
                0,
                0,
                None,
                None,
                None,
                i + 1,
            )
            .unwrap();
    }

    assert_pages_to_exhaustion(
        addr,
        token,
        "list_step_attempts",
        json!({ "step_execution_id": step_execution_id.as_str() }),
        55,
        top_level,
    )
    .await;
}

#[tokio::test]
async fn list_pending_gates_paginates_to_exhaustion() {
    let (addr, resource, ctx) = spawn_mcp_router("pagination-gates").await;
    let token = "token-gates";
    seed_read_grant(&ctx, token, &resource);
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    for i in 0..55 {
        let feature_id = FeatureId::from(format!("f-{i:03}"));
        ctx.features
            .add(feature(&format!("f-{i:03}"), &project_id))
            .unwrap();
        let step_execution_id = StepExecutionId::from(format!("se-{i:03}"));
        ctx.features
            .step_create(step(&format!("se-{i:03}"), &feature_id, 0))
            .unwrap();
        ctx.gates
            .create(GateDecision {
                id: GateDecisionId::from(format!("gd-{i:03}")),
                step_execution_id,
                decision: None,
                feedback: None,
                created_at: 0,
            })
            .unwrap();
    }

    assert_pages_to_exhaustion(
        addr,
        token,
        "list_pending_gates",
        json!({ "project_id": project_id.as_str() }),
        55,
        top_level,
    )
    .await;
}

#[tokio::test]
async fn run_events_since_paginates_to_exhaustion() {
    let (addr, resource, ctx) = spawn_mcp_router("pagination-events").await;
    let token = "token-events";
    seed_read_grant(&ctx, token, &resource);
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-1".to_string());
    ctx.features.add(feature("f-1", &project_id)).unwrap();
    for i in 0..55i64 {
        ctx.run_events
            .append(feature_id.as_str(), "kind", None, i)
            .unwrap();
    }

    assert_pages_to_exhaustion(
        addr,
        token,
        "run_events_since",
        json!({ "feature_id": feature_id.as_str(), "from_offset": 0 }),
        55,
        top_level,
    )
    .await;
}

#[tokio::test]
async fn get_discovery_board_paginates_tickets_under_the_tickets_key() {
    let (addr, resource, ctx) = spawn_mcp_router("pagination-board").await;
    let token = "token-board";
    seed_read_grant(&ctx, token, &resource);
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let discovery =
        open_discovery(&ctx, opening(&project_id, "board fixture")).expect("the discovery opens");
    let tickets: Vec<Ticket> = (0..55)
        .map(|i| ticket(&format!("t-{i:03}"), &discovery.id, i))
        .collect();
    ctx.tickets.upsert_batch(&tickets).unwrap();

    assert_pages_to_exhaustion(
        addr,
        token,
        "get_discovery_board",
        json!({ "discovery_id": discovery.id.as_str() }),
        55,
        |v| &v["tickets"],
    )
    .await;
}

#[tokio::test]
async fn exactly_at_limit_is_not_truncated_and_has_no_cursor() {
    let (addr, resource, ctx) = spawn_mcp_router("pagination-exact").await;
    let token = "token-exact";
    seed_read_grant(&ctx, token, &resource);
    for i in 0..50 {
        ctx.projects.add(project(&format!("p-{i:03}"))).unwrap();
    }

    let body = call_tool(addr, token, "list_projects", json!({})).await;
    let content = &body["result"]["structuredContent"];
    assert_eq!(content["items"].as_array().unwrap().len(), 50);
    assert_eq!(content["truncated"], json!(false));
    assert!(content["next_cursor"].is_null());
}

#[tokio::test]
async fn an_unparseable_cursor_is_invalid_params_not_a_silent_reset() {
    let (addr, resource, ctx) = spawn_mcp_router("pagination-bad-cursor").await;
    let token = "token-bad-cursor";
    seed_read_grant(&ctx, token, &resource);
    ctx.projects.add(project("p-1")).unwrap();

    let body = call_tool(
        addr,
        token,
        "list_projects",
        json!({ "cursor": "not-a-number" }),
    )
    .await;

    assert!(
        body.get("result").is_none(),
        "expected a protocol error, not a result"
    );
    assert_eq!(body["error"]["code"], json!(-32602));
}
