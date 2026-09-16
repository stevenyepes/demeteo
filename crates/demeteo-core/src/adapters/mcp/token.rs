//! `POST /token` — exchanges a single-use authorization code (plus its PKCE
//! `code_verifier`) for an opaque MCP access token.
//!
//! `code_verifier` is only ever seen here: `/authorize` never receives one,
//! so re-running [`pkce::verify_pkce`] against the stored `code_challenge` is
//! this handler's job, not a re-check of work `/authorize` already did.
//! [`take_pending_authorization`] removes the code from the pending-
//! authorization store before any of the checks below run, which is what
//! makes a replayed code fail even when the first exchange's own PKCE or
//! `redirect_uri` check would otherwise have failed too — a code is
//! consumed by being looked up, not by succeeding, so an attacker cannot use
//! a stolen code to probe multiple verifier guesses against it.
//!
//! `resource` (RFC 8707) is mandatory and must equal [`canonical_uri`]
//! exactly, checked before the code is ever looked up — the same
//! `invalid_target` rule `authorize.rs` applies, and structural mistakes
//! that don't need the code should never burn it.
//!
//! The issued access token is generated, hashed, and persisted (as a hash
//! only — never itself) here, and returned in the response body exactly
//! once: this handler is the only place in the codebase that ever holds the
//! plaintext token in memory. Never log it, the authorization code, or the
//! `code_verifier` at any `tracing` level (implementation-spec.md §6).

use axum::extract::{Form, State};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::domain::ids::GrantId;
use crate::domain::oauth::{pkce, GrantRecord, Scope};
use crate::state::AppContext;

use super::authorize::{generate_token, oauth_bad_request, take_pending_authorization};
use super::canonical_uri;
use super::guard::hash_token;

/// Fixed grant lifetime from issuance — implementation-spec.md §7 Open
/// Question 3's stated default. No refresh tokens exist in this design, so
/// expiry always means a fresh `/authorize` round trip, never a renewal.
const GRANT_LIFETIME_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Mounted onto the shared router by [`super::router`]. `pub(super)` — same
/// visibility as `metadata::routes` and `register::routes`.
pub(super) fn routes() -> axum::Router<AppContext> {
    axum::Router::new().route("/token", post(token))
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    #[serde(default)]
    grant_type: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    resource: String,
}

/// RFC 6749 §5.1 token response shape. `scope` is the space-separated grant
/// the token was actually issued with, mirroring the `oauth_grants.scopes`
/// storage convention.
#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    scope: String,
}

fn encode_scope(scopes: &[Scope]) -> String {
    scopes
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

async fn token(State(ctx): State<AppContext>, Form(req): Form<TokenRequest>) -> Response {
    if req.grant_type != "authorization_code" {
        return oauth_bad_request("unsupported_grant_type");
    }

    let Some(canonical) = canonical_uri() else {
        return oauth_bad_request("invalid_target");
    };
    if req.resource != canonical {
        return oauth_bad_request("invalid_target");
    }

    // Consuming here — not after the checks below — is deliberate: see the
    // module docs on why a code must burn on lookup, not on success.
    let Some(pending) = take_pending_authorization(&req.code) else {
        return oauth_bad_request("invalid_grant");
    };

    if req.redirect_uri != pending.redirect_uri {
        return oauth_bad_request("invalid_grant");
    }

    if pkce::verify_pkce(&req.code_verifier, &pending.code_challenge, "S256").is_err() {
        return oauth_bad_request("invalid_grant");
    }

    let access_token = generate_token();
    let token_hash = hash_token(&access_token);
    let issued_at = crate::paths::now_ms();
    let scope = encode_scope(&pending.scopes);

    let grant = GrantRecord {
        id: GrantId::from(crate::shared::ids::new_id()),
        client_id: pending.client_id,
        scopes: pending.scopes,
        resource: pending.resource,
        issued_at,
        expires_at: issued_at + GRANT_LIFETIME_MS,
        revoked_at: None,
    };

    if let Err(e) = ctx.oauth_grants.insert_grant(grant, &token_hash) {
        tracing::error!(error = %e, "failed to persist mcp grant");
        return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Json(TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: GRANT_LIFETIME_MS / 1000,
        scope,
    })
    .into_response()
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/full_flow.rs"]
mod tests;
