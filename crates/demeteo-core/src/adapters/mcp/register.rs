//! RFC 7591 dynamic client registration — `POST /register`.
//!
//! Unauthenticated by design: an MCP client has no prior credential
//! relationship with Demeteo (implementation-spec.md §7 Open Question 2), so
//! there is no existing client to authenticate this request against.
//! `client_name` is self-asserted and never checked against an allowlist —
//! the real security boundary is the human consent screen a later ticket
//! adds in front of `/authorize`, not this endpoint.
//!
//! Public client only. No `client_secret` is ever generated, stored, or
//! returned — PKCE is this design's only proof of possession, and the V56
//! migration's `oauth_clients` table has no `client_secret` column to put
//! one in.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::domain::ids::ClientId;
use crate::domain::oauth::OAuthClient;
use crate::state::AppContext;

/// Mounted onto the shared router by [`super::router`]. `pub(super)` — same
/// visibility as `metadata::routes` and `mcp_handler::routes`.
pub(super) fn routes() -> axum::Router<AppContext> {
    axum::Router::new().route("/register", post(register))
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    client_name: String,
    redirect_uris: Vec<String>,
}

/// Deliberately has no `client_secret` field — see the module docs.
#[derive(Debug, Serialize)]
struct RegisterResponse {
    client_id: String,
    client_name: String,
    redirect_uris: Vec<String>,
}

async fn register(State(ctx): State<AppContext>, Json(req): Json<RegisterRequest>) -> Response {
    let client = OAuthClient {
        id: ClientId::from(crate::shared::ids::new_id()),
        client_name: req.client_name,
        redirect_uris: req.redirect_uris,
        created_at: crate::paths::now_ms(),
    };

    if let Err(e) = ctx.oauth_clients.register_client(client.clone()) {
        tracing::error!(error = %e, "failed to register mcp client");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (
        StatusCode::CREATED,
        Json(RegisterResponse {
            client_id: client.id.into(),
            client_name: client.client_name,
            redirect_uris: client.redirect_uris,
        }),
    )
        .into_response()
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/register.rs"]
mod tests;
