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
use crate::domain::ids::{DiscoveryId, FeatureId, ProjectId, StepExecutionId, TicketId};
use crate::domain::models::project::RunShapePatch;
use crate::domain::oauth::tools::required_scope;
use crate::state::AppContext;

use super::guard;

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

/// Routes an already-authorized call to its backing `agent_surface`
/// function. `name` has already passed [`required_scope`] by the time this
/// runs, so the fallback arm is unreachable by construction rather than a
/// real error case.
async fn dispatch(ctx: &AppContext, name: &str, arguments: Value) -> Result<Value, DispatchError> {
    match name {
        "list_projects" => to_json(agent_surface::list_projects(ctx)),
        "list_features" => {
            let args: OptionalProjectArgs = parse(arguments)?;
            to_json(agent_surface::list_features(
                ctx,
                args.project_id.map(ProjectId::from).as_ref(),
            ))
        }
        "get_feature" => {
            let args: GetFeatureArgs = parse(arguments)?;
            to_json(agent_surface::get_feature(
                ctx,
                &FeatureId::from(args.feature_id),
            ))
        }
        "list_step_attempts" => {
            let args: StepExecutionArgs = parse(arguments)?;
            to_json(agent_surface::list_step_attempts(
                ctx,
                &StepExecutionId::from(args.step_execution_id),
            ))
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
            to_json(agent_surface::list_pending_gates(
                ctx,
                args.project_id.map(ProjectId::from).as_ref(),
            ))
        }
        "get_discovery_board" => {
            let args: DiscoveryArgs = parse(arguments)?;
            to_json(agent_surface::get_discovery_board(
                ctx,
                &DiscoveryId::from(args.discovery_id),
            ))
        }
        "run_events_since" => {
            let args: RunEventsSinceArgs = parse(arguments)?;
            to_json(agent_surface::run_events_since(
                ctx,
                &FeatureId::from(args.feature_id),
                args.from_offset,
            ))
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

#[derive(Debug, Deserialize, JsonSchema)]
struct NoArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
struct OptionalProjectArgs {
    #[serde(default)]
    project_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetFeatureArgs {
    feature_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StepExecutionArgs {
    step_execution_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DiscoveryArgs {
    discovery_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RunEventsSinceArgs {
    feature_id: String,
    #[serde(default)]
    from_offset: i64,
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
            schemars::schema_for!(StepExecutionArgs),
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
