//! `POST /mcp` — JSON-RPC 2.0 dispatch over the `agent_surface` tool catalog.
//!
//! `tools/list` is unauthenticated, the same discovery posture as the
//! `.well-known` metadata routes: it hands out names, descriptions, and
//! argument schemas, none of which are secret, and an MCP client needs it to
//! know what to ask an eventual `/authorize` grant for.
//!
//! `tools/call` is the enforcement boundary. [`required_scope`] is looked up
//! **before** [`guard::check`] runs, on purpose: the 401/403 challenge that
//! `guard::check` builds names whatever `Scope` it was asked to check, so
//! resolving the *real* scope for `params.name` first is what makes that
//! challenge name the actually-attempted operation instead of a fixed
//! default (implementation-spec.md AC2). An unrecognized tool name has no
//! scope to look up — [`required_scope`] returns `None` — and that short-
//! circuits straight to a JSON-RPC "method not found" error, never reaching
//! the guard: there is nothing to be insufficiently scoped for.
//!
//! A tool that runs (guard passed) but fails at the application layer (e.g.
//! "project not found") is reported as MCP's own tool-level error shape
//! (`isError: true` inside a `200`), not a JSON-RPC protocol error — the
//! distinction the MCP spec draws between "the RPC failed" and "the tool
//! ran and reported failure".

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Json;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::application::agent_surface::{self, AgentFeatureLaunch};
use crate::application::projects::{ProjectConfig, RepositoryConfig};
use crate::application::tickets::TicketView;
use crate::domain::ids::{DiscoveryId, FeatureId, ProjectId, StepExecutionId, TicketId};
use crate::domain::models::project::RunShapePatch;
use crate::domain::oauth::tools::required_scope;
use crate::domain::ticket_graph::TicketProgress;
use crate::state::AppContext;

use super::guard;
use super::protocol;

/// Mounted onto the shared router by [`super::router`]. `pub(super)` — same
/// visibility as `metadata::routes`.
pub(super) fn routes() -> axum::Router<AppContext> {
    axum::Router::new().route("/mcp", post(handle))
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcErrorBody>,
}

#[derive(Serialize)]
struct JsonRpcErrorBody {
    code: i64,
    message: String,
}

fn success(id: Value, result: Value) -> Response {
    Json(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    })
    .into_response()
}

fn rpc_error(id: Value, code: i64, message: String) -> Response {
    Json(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcErrorBody { code, message }),
    })
    .into_response()
}

/// JSON-RPC's own "method not found" — reused for an unrecognized tool
/// `name` inside `tools/call`, not only an unrecognized top-level `method`,
/// since neither has a scope to check.
fn method_not_found(id: Value, name: &str) -> Response {
    rpc_error(id, -32601, format!("method not found: {name}"))
}

fn invalid_params(id: Value, detail: &str) -> Response {
    rpc_error(id, -32602, format!("invalid params: {detail}"))
}

fn tool_success(id: Value, value: Value) -> Response {
    success(
        id,
        json!({
            "content": [{ "type": "text", "text": value.to_string() }],
            "structuredContent": value,
            "isError": false,
        }),
    )
}

fn tool_failure(id: Value, message: String) -> Response {
    success(
        id,
        json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        }),
    )
}

async fn handle(
    State(ctx): State<AppContext>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    match req.method.as_str() {
        "tools/list" => success(req.id, json!({ "tools": tool_catalog() })),
        "tools/call" => handle_tools_call(&ctx, &headers, req.id, req.params).await,
        "server/discover" => protocol::discover(req.id),
        // No real handshake, sessions, or capability negotiation — this
        // revision is stateless. The arm exists only so a legacy client
        // gets a correctly-named diagnostic naming the versions this server
        // understands, rather than a generic "method not found" (this may
        // be the only diagnostic such a client ever surfaces to a user).
        "initialize" => protocol::unsupported_version_response(req.id),
        other => method_not_found(req.id, other),
    }
}

#[derive(Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

enum DispatchError {
    InvalidParams(String),
    Failed(String),
}

async fn handle_tools_call(
    ctx: &AppContext,
    headers: &HeaderMap,
    id: Value,
    params: Value,
) -> Response {
    let call: ToolCallParams = match serde_json::from_value(params) {
        Ok(call) => call,
        Err(e) => return invalid_params(id, &e.to_string()),
    };

    let Some(required) = required_scope(&call.name) else {
        return method_not_found(id, &call.name);
    };

    if let Err(response) = guard::check(ctx, required, headers).await {
        return response;
    }

    let arguments = call.arguments.unwrap_or_else(|| json!({}));
    match dispatch(ctx, &call.name, arguments).await {
        Ok(value) => tool_success(id, value),
        Err(DispatchError::InvalidParams(detail)) => invalid_params(id, &detail),
        Err(DispatchError::Failed(message)) => tool_failure(id, message),
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, DispatchError> {
    serde_json::from_value(value).map_err(|e| DispatchError::InvalidParams(e.to_string()))
}

fn to_json<T: Serialize>(result: Result<T, String>) -> Result<Value, DispatchError> {
    match result {
        Ok(value) => serde_json::to_value(value).map_err(|e| DispatchError::Failed(e.to_string())),
        Err(message) => Err(DispatchError::Failed(message)),
    }
}

/// `limit` defaults to [`DEFAULT_PAGE_LIMIT`] and silently clamps to
/// [`MAX_PAGE_LIMIT`] — a caller asking for too much gets the max, not an
/// error, since the value is only ever a size hint. `cursor` is the decimal
/// string of the next start index into the already-materialized `Vec`
/// (in-memory pagination over a fully-fetched result, not a DB-level
/// cursor); a cursor that fails to parse as that index is an
/// [`DispatchError::InvalidParams`], never a silent reset to page 1, which
/// would mask a client bug. A cursor past the end yields an empty,
/// non-truncated page rather than an error, since the underlying result set
/// may have legitimately shrunk between calls.
const DEFAULT_PAGE_LIMIT: u32 = 50;
const MAX_PAGE_LIMIT: u32 = 200;

fn paginate<T: Serialize>(items: Vec<T>, page: &PageArgs) -> Result<Page<T>, DispatchError> {
    let start = match &page.cursor {
        Some(raw) => raw.parse::<usize>().map_err(|_| {
            DispatchError::InvalidParams(format!("cursor is not a valid page offset: {raw:?}"))
        })?,
        None => 0,
    };
    let limit = page.limit.unwrap_or(DEFAULT_PAGE_LIMIT).min(MAX_PAGE_LIMIT) as usize;

    let total = items.len();
    let start = start.min(total);
    let end = start.saturating_add(limit).min(total);
    let truncated = end < total;
    let next_cursor = truncated.then(|| end.to_string());
    let items = items.into_iter().skip(start).take(end - start).collect();

    Ok(Page {
        items,
        truncated,
        next_cursor,
    })
}

fn to_paged_json<T: Serialize>(
    result: Result<Vec<T>, String>,
    page: &PageArgs,
) -> Result<Value, DispatchError> {
    let items = result.map_err(DispatchError::Failed)?;
    let page = paginate(items, page)?;
    serde_json::to_value(page).map_err(|e| DispatchError::Failed(e.to_string()))
}

/// Routes an already-authorized call to its backing `agent_surface`
/// function. `name` has already passed [`required_scope`] by the time this
/// runs, so the fallback arm is unreachable by construction rather than a
/// real error case.
async fn dispatch(ctx: &AppContext, name: &str, arguments: Value) -> Result<Value, DispatchError> {
    match name {
        "list_projects" => {
            let args: NoArgs = parse(arguments)?;
            to_paged_json(agent_surface::list_projects(ctx), &args.page)
        }
        "list_features" => {
            let args: OptionalProjectArgs = parse(arguments)?;
            to_paged_json(
                agent_surface::list_features(ctx, args.project_id.map(ProjectId::from).as_ref()),
                &args.page,
            )
        }
        "get_feature" => {
            let args: GetFeatureArgs = parse(arguments)?;
            to_json(agent_surface::get_feature(
                ctx,
                &FeatureId::from(args.feature_id),
            ))
        }
        "list_step_attempts" => {
            let args: ListStepAttemptsArgs = parse(arguments)?;
            to_paged_json(
                agent_surface::list_step_attempts(
                    ctx,
                    &StepExecutionId::from(args.step_execution_id),
                ),
                &args.page,
            )
        }
        "get_failure_verdict" => {
            let args: StepExecutionArgs = parse(arguments)?;
            to_json(agent_surface::get_failure_verdict(
                ctx,
                &StepExecutionId::from(args.step_execution_id),
            ))
        }
        "list_pending_gates" => {
            let args: OptionalProjectArgs = parse(arguments)?;
            to_paged_json(
                agent_surface::list_pending_gates(
                    ctx,
                    args.project_id.map(ProjectId::from).as_ref(),
                ),
                &args.page,
            )
        }
        "get_discovery_board" => {
            let args: DiscoveryArgs = parse(arguments)?;
            let board =
                agent_surface::get_discovery_board(ctx, &DiscoveryId::from(args.discovery_id))
                    .map_err(DispatchError::Failed)?;
            let tickets = paginate(board.tickets, &args.page)?;
            to_json(Ok::<_, String>(DiscoveryBoardPage {
                tickets,
                progress: board.progress,
            }))
        }
        "run_events_since" => {
            let args: RunEventsSinceArgs = parse(arguments)?;
            to_paged_json(
                agent_surface::run_events_since(
                    ctx,
                    &FeatureId::from(args.feature_id),
                    args.from_offset,
                ),
                &args.page,
            )
        }
        "create_workspace_project" => {
            let args: CreateWorkspaceProjectArgs = parse(arguments)?;
            let config = ProjectConfig {
                name: args.name,
                compute_type: args.compute_type,
                remote_host: args.remote_host,
                repos: args
                    .repos
                    .into_iter()
                    .map(|r| RepositoryConfig {
                        repo_path: r.repo_path,
                        provider_id: r.provider_id,
                    })
                    .collect(),
            };
            to_json(agent_surface::create_workspace_project(ctx, config))
        }
        "apply_run_shape_patch" => {
            let args: ApplyRunShapePatchArgs = parse(arguments)?;
            let patch: RunShapePatch = serde_json::from_value(args.patch)
                .map_err(|e| DispatchError::InvalidParams(e.to_string()))?;
            to_json(agent_surface::apply_run_shape_patch(
                ctx,
                &ProjectId::from(args.project_id),
                patch,
            ))
        }
        "start_feature" => {
            let args: StartFeatureArgs = parse(arguments)?;
            to_json(
                agent_surface::start_feature(
                    ctx,
                    AgentFeatureLaunch {
                        project_id: args.project_id,
                        workflow_id: args.workflow_id,
                        title: args.title,
                        description: args.description,
                    },
                )
                .await,
            )
        }
        "start_ticket" => {
            let args: TicketArgs = parse(arguments)?;
            to_json(agent_surface::start_ticket(ctx, &TicketId::from(args.ticket_id)).await)
        }
        other => unreachable!("required_scope filtered out unknown tool name {other:?} already"),
    }
}

/// Shared by every list-shaped tool's args struct via `#[serde(flatten)]`.
/// See [`paginate`] for the pagination semantics this drives.
#[derive(Debug, Default, Deserialize, JsonSchema)]
struct PageArgs {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    cursor: Option<String>,
}

/// The response shape every paginated tool result wraps its items in —
/// nested under a `tickets` key for `get_discovery_board`
/// ([`DiscoveryBoardPage`]), returned at the top level for the other five.
#[derive(Serialize)]
struct Page<T: Serialize> {
    items: Vec<T>,
    truncated: bool,
    next_cursor: Option<String>,
}

/// `get_discovery_board`'s result: [`Page`] nests under `tickets` instead of
/// replacing the whole result, since `progress` (the derived board) is not
/// itself list-shaped.
#[derive(Serialize)]
struct DiscoveryBoardPage {
    tickets: Page<TicketView>,
    progress: TicketProgress,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct NoArgs {
    #[serde(flatten)]
    page: PageArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct OptionalProjectArgs {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(flatten)]
    page: PageArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetFeatureArgs {
    feature_id: String,
}

/// `get_failure_verdict`'s args — unpaginated, since its result is already
/// bounded by `LogTail`'s own budget (implementation-spec.md §2).
#[derive(Debug, Deserialize, JsonSchema)]
struct StepExecutionArgs {
    step_execution_id: String,
}

/// `list_step_attempts`'s args. Kept distinct from [`StepExecutionArgs`]
/// even though both name only a `step_execution_id`: flattening [`PageArgs`]
/// into the shared struct would also add `limit`/`cursor` to
/// `get_failure_verdict`'s schema, which implementation-spec.md §2
/// explicitly excludes from pagination.
#[derive(Debug, Deserialize, JsonSchema)]
struct ListStepAttemptsArgs {
    step_execution_id: String,
    #[serde(flatten)]
    page: PageArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DiscoveryArgs {
    discovery_id: String,
    #[serde(flatten)]
    page: PageArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunEventsSinceArgs {
    feature_id: String,
    #[serde(default)]
    from_offset: i64,
    #[serde(flatten)]
    page: PageArgs,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RepositoryConfigArgs {
    repo_path: String,
    provider_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateWorkspaceProjectArgs {
    name: String,
    compute_type: String,
    #[serde(default)]
    remote_host: Option<String>,
    #[serde(default)]
    repos: Vec<RepositoryConfigArgs>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ApplyRunShapePatchArgs {
    project_id: String,
    /// The `RunShapePatch` fields (`default_agent_kind`, `default_model`,
    /// `default_effort`, `default_workflow_id`, `artifact_subdir`,
    /// `commit_artifacts`, `sync_resolver_agent_kind`, `sync_resolver_model`,
    /// `sync_resolver_effort`) — kept as an opaque value here and decoded via
    /// `RunShapePatch`'s own `Deserialize` at dispatch time rather than
    /// duplicated field-by-field, so the two can never drift.
    patch: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StartFeatureArgs {
    project_id: String,
    workflow_id: String,
    title: String,
    description: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TicketArgs {
    ticket_id: String,
}

fn tool_descriptor(name: &str, description: &str, schema: schemars::Schema) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema.to_value(),
    })
}

/// The literal 12-row catalog from implementation-spec.md §4, in the same
/// order as [`crate::domain::oauth::tools::required_scope`]'s table.
fn tool_catalog() -> Value {
    json!([
        tool_descriptor(
            "list_projects",
            "List every workspace project.",
            schemars::schema_for!(NoArgs),
        ),
        tool_descriptor(
            "list_features",
            "List active features, optionally scoped to one project.",
            schemars::schema_for!(OptionalProjectArgs),
        ),
        tool_descriptor(
            "get_feature",
            "Get a feature and its step executions.",
            schemars::schema_for!(GetFeatureArgs),
        ),
        tool_descriptor(
            "list_step_attempts",
            "List a step's per-attempt history.",
            schemars::schema_for!(ListStepAttemptsArgs),
        ),
        tool_descriptor(
            "get_failure_verdict",
            "Explain why a step failed, with log evidence.",
            schemars::schema_for!(StepExecutionArgs),
        ),
        tool_descriptor(
            "list_pending_gates",
            "List gates awaiting a human decision, optionally scoped to one project.",
            schemars::schema_for!(OptionalProjectArgs),
        ),
        tool_descriptor(
            "get_discovery_board",
            "Get a Discovery's tickets and its derived board.",
            schemars::schema_for!(DiscoveryArgs),
        ),
        tool_descriptor(
            "run_events_since",
            "List a feature's durable run events after an offset.",
            schemars::schema_for!(RunEventsSinceArgs),
        ),
        tool_descriptor(
            "create_workspace_project",
            "Create a project and its repositories.",
            schemars::schema_for!(CreateWorkspaceProjectArgs),
        ),
        tool_descriptor(
            "apply_run_shape_patch",
            "Patch a project's run-shape settings (agent, model, effort, workflow, artifacts).",
            schemars::schema_for!(ApplyRunShapePatchArgs),
        ),
        tool_descriptor(
            "start_feature",
            "Start a Feature run.",
            schemars::schema_for!(StartFeatureArgs),
        ),
        tool_descriptor(
            "start_ticket",
            "Start a Ticket's current attempt.",
            schemars::schema_for!(TicketArgs),
        ),
    ])
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/unauthenticated.rs"]
mod unauthenticated_tests;

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/scope_step_up.rs"]
mod scope_step_up_tests;

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/audience_and_expiry.rs"]
mod audience_and_expiry_tests;

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/revocation.rs"]
mod revocation_tests;

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/pagination.rs"]
mod pagination_tests;
