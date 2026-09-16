//! RFC 9728 (OAuth 2.0 Protected Resource Metadata) and RFC 8414 (OAuth 2.0
//! Authorization Server Metadata) `.well-known` endpoints — the discovery
//! surface an MCP client probes before it holds a token. Both handlers read
//! [`canonical_uri`](super::canonical_uri) rather than re-deriving the
//! server's address. `/authorize` and `/token` don't exist yet (later
//! tickets); the authorization-server document names them anyway, since RFC
//! 8414 describes capability, not what has already been dialed.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde_json::json;

use crate::domain::oauth::Scope;
use crate::state::AppContext;

use super::canonical_uri;

/// The two `.well-known` routes, mounted onto the shared router by
/// [`super::router`]. `pub(super)` — only the parent module assembles the
/// full router; nothing else needs to merge these in isolation.
pub(super) fn routes() -> axum::Router<AppContext> {
    axum::Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server),
        )
}

/// Unreachable once [`super::serve`] has bound its listener — the canonical
/// URI is recorded before the router ever starts accepting connections. Kept
/// as a real response rather than a panic because a handler is not a
/// production path that gets to assume its preconditions hold.
fn resource_not_yet_known() -> Response {
    StatusCode::SERVICE_UNAVAILABLE.into_response()
}

async fn protected_resource() -> Response {
    let Some(resource) = canonical_uri() else {
        return resource_not_yet_known();
    };
    Json(json!({
        "resource": resource,
        "scopes_supported": [Scope::Read.as_str()],
    }))
    .into_response()
}

async fn authorization_server() -> Response {
    let Some(issuer) = canonical_uri() else {
        return resource_not_yet_known();
    };
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "registration_endpoint": format!("{issuer}/register"),
        "code_challenge_methods_supported": ["S256"],
    }))
    .into_response()
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/metadata.rs"]
mod tests;
