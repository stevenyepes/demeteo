//! The tool→scope table every MCP dispatch decision funnels through.
//!
//! `mcp_handler.rs` looks up [`required_scope`] for `params.name` **before**
//! running the guard, so a 401/403 challenge names the real attempted scope
//! rather than a hardcoded default. An unrecognized name maps to `None`,
//! which the dispatcher turns into JSON-RPC `method not found` — never a
//! scope error, since there is no scope to be insufficient in.

use super::Scope;

/// The literal 12-row table from implementation-spec.md §4.
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

#[cfg(test)]
#[path = "../../../tests/domain/oauth/tools.rs"]
mod tools_tests;
