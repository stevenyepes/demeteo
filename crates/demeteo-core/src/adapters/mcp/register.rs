//! RFC 7591 dynamic client registration — `POST /register`.
//!
//! Unauthenticated by design: an MCP client has no prior credential
//! relationship with Demeteo (`docs/MCP_INTEGRATION.md` §5), so
//! there is no existing client to authenticate this request against.
//! `client_name` is self-asserted and never checked against an allowlist —
//! the real security boundary is the human consent screen in front of
//! `/authorize`, not this endpoint. What this endpoint does own is *bounds*:
//! [`validate_registration`] limits what the consent screen can be made to
//! show or redirect to, and the client table is capped so an anonymous caller
//! cannot grow it without limit ([`MAX_CLIENTS`]).
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
use serde_json::json;

use crate::domain::ids::ClientId;
use crate::domain::oauth::registration::{validate_registration, RegistrationError};
use crate::domain::oauth::OAuthClient;
use crate::state::AppContext;

/// Registered clients kept at once. A registration is a row anyone on the
/// loopback interface can add, so the table is bounded rather than trusted to
/// stay small.
const MAX_CLIENTS: usize = 100;

/// A registration that never received a grant is dropped after this once the
/// table is full.
const UNUSED_CLIENT_TTL_MS: i64 = 24 * 60 * 60 * 1000;

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
    if let Err(e) = validate_registration(&req.client_name, &req.redirect_uris) {
        tracing::debug!(error = %e, "rejected mcp client registration");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": match e {
                    RegistrationError::RedirectUri(_) | RegistrationError::RedirectUriCount => {
                        "invalid_redirect_uri"
                    }
                    RegistrationError::ClientName => "invalid_client_metadata",
                },
                "error_description": e.to_string(),
            })),
        )
            .into_response();
    }

    let now = crate::paths::now_ms();
    let client = OAuthClient {
        id: ClientId::from(crate::shared::ids::new_id()),
        client_name: req.client_name.trim().to_string(),
        redirect_uris: req.redirect_uris,
        created_at: now,
    };

    match ctx.oauth_clients.register_client_bounded(
        client.clone(),
        MAX_CLIENTS,
        now - UNUSED_CLIENT_TTL_MS,
    ) {
        Ok(true) => {}
        Ok(false) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "temporarily_unavailable" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to register mcp client");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
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
