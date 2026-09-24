//! The tool→scope table every MCP dispatch decision funnels through.
//!
//! `mcp_handler.rs` looks up [`required_scope`] for `params.name` **before**
//! running the guard, so a 401/403 challenge names the real attempted scope
//! rather than a hardcoded default. An unrecognized name maps to `None`,
//! which the dispatcher turns into JSON-RPC `method not found` — never a
//! scope error, since there is no scope to be insufficient in.

use super::Scope;

/// The literal 12-row table from `docs/MCP_INTEGRATION.md` §7.
pub fn required_scope(tool_name: &str) -> Option<Scope> {
    match tool_name {
        "list_projects"
        | "list_features"
        | "get_feature"
        | "list_step_attempts"
        | "get_failure_verdict"
        | "list_pending_gates"
        | "get_discovery_board"
        | "run_events_since" => Some(Scope::Read),
        "create_workspace_project" | "apply_run_shape_patch" => Some(Scope::Configure),
        "start_feature" | "start_ticket" => Some(Scope::Spend),
        _ => None,
    }
}

/// What a `POST /mcp` JSON-RPC method needs before it is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodAuth {
    /// Answered without a grant.
    Open,
    /// Any live grant bound to this server, whatever its scopes.
    Session,
    /// Scoped per tool by [`required_scope`] on `params.name`.
    PerTool,
}

/// `server/discover` names the protocol revision and nothing else, so it
/// stays open. Everything else — `initialize` included — needs a grant: a
/// client whose MCP SDK authorizes only on a connect-time 401 never signs in
/// otherwise (`docs/MCP_INTEGRATION.md` §5).
pub fn method_auth(method: &str) -> MethodAuth {
    match method {
        "server/discover" => MethodAuth::Open,
        "tools/call" => MethodAuth::PerTool,
        _ => MethodAuth::Session,
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/oauth/tools.rs"]
mod tools_tests;
