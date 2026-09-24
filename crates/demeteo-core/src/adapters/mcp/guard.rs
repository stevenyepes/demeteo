//! Shared bearer-token enforcement for every MCP-protected route.
//! `docs/MCP_INTEGRATION.md` §5: no route calls a tool function without
//! going through this: [`check`] turns a request's `Authorization`
//! header into a [`GrantRecord`], or into the exact `401`/`403` response the
//! caller should return — a route never inspects the header or an
//! [`OAuthError`] itself.
//!
//! `token_hash` is SHA-256(access token), hex-encoded — the encoding the V56
//! migration comment and `adapters/database/repos/oauth.rs` both commit to.
//! This module is the only place that hash is computed; the repository layer
//! only ever sees the digest.
//!
//! A missing `Authorization` header and a header naming an unrecognized
//! token both resolve to `None` grant below and get the identical `401` —
//! deliberately: neither is "more wrong" than the other, both are just "no
//! valid grant".

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::domain::oauth::{validate_grant, validate_session, GrantRecord, OAuthError, Scope};
use crate::state::AppContext;

use super::canonical_uri;

/// Shared with `token.rs`, which hashes a freshly issued access token the
/// same way before persisting it — the two must never diverge, or a token
/// this module just issued would fail its own next lookup.
pub(super) fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let (scheme, token) = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .split_once(' ')?;
    // RFC 7235 §2.1: the auth scheme is case-insensitive.
    scheme.eq_ignore_ascii_case("Bearer").then_some(token)
}

fn challenge(status: StatusCode, www_authenticate: &str) -> Response {
    let mut response = status.into_response();
    if let Ok(value) = HeaderValue::from_str(www_authenticate) {
        response
            .headers_mut()
            .insert(axum::http::header::WWW_AUTHENTICATE, value);
    }
    response
}

/// `401` — no, or no longer valid, grant. `resource_metadata` points at the
/// RFC 9728 discovery document; `scope` names the operation that was
/// actually attempted, never a fixed default, so a client can tell what to
/// request on its next `/authorize` round trip. A request that attempted no
/// operation ([`authenticate`]) names no scope, and the client falls back to
/// `scopes_supported` — the MCP spec's scope-selection order.
fn unauthorized(required: Option<Scope>) -> Response {
    let resource_metadata = canonical_uri()
        .map(|uri| format!("{uri}/.well-known/oauth-protected-resource"))
        .unwrap_or_default();
    let header = match required {
        Some(scope) => format!(
            r#"Bearer resource_metadata="{resource_metadata}", scope="{}""#,
            scope.as_str()
        ),
        None => format!(r#"Bearer resource_metadata="{resource_metadata}""#),
    };
    challenge(StatusCode::UNAUTHORIZED, &header)
}

/// `403` — a real, unexpired, unrevoked grant that simply doesn't carry
/// `required`. Distinct from [`unauthorized`]: the client already holds a
/// usable token, it just needs a broader grant.
fn insufficient_scope(required: Scope) -> Response {
    challenge(
        StatusCode::FORBIDDEN,
        &format!(
            r#"Bearer error="insufficient_scope", scope="{}""#,
            required.as_str()
        ),
    )
}

fn lookup_grant(ctx: &AppContext, headers: &HeaderMap) -> Option<GrantRecord> {
    bearer_token(headers).and_then(|token| {
        ctx.oauth_grants
            .find_grant_by_token_hash(&hash_token(token))
            .ok()
            .flatten()
    })
}

/// Resolves a request's bearer token to the [`GrantRecord`] it authorizes
/// `required` for, or to the exact response the caller should return.
/// Every protected route funnels through this or [`authenticate`] rather
/// than reading `Authorization` or matching on [`OAuthError`] itself.
///
pub async fn check(
    ctx: &AppContext,
    required: Scope,
    headers: &HeaderMap,
) -> Result<GrantRecord, Response> {
    let Some(grant) = lookup_grant(ctx, headers) else {
        return Err(unauthorized(Some(required)));
    };

    let resource = canonical_uri().unwrap_or_default();
    match validate_grant(&grant, crate::paths::now_ms(), &resource, required) {
        Ok(()) => Ok(grant),
        Err(OAuthError::InsufficientScope { required }) => Err(insufficient_scope(required)),
        Err(_) => Err(unauthorized(Some(required))),
    }
}

/// [`check`] for a request that names no operation: any live grant bound to
/// this server passes, whatever its scopes. The `401` it answers otherwise
/// is what starts sign-in for a client whose MCP SDK authorizes only when
/// connecting is refused (OpenCode, Hermes) — see `docs/MCP_INTEGRATION.md`
/// §5 for why the handshake is not left open.
pub async fn authenticate(ctx: &AppContext, headers: &HeaderMap) -> Result<GrantRecord, Response> {
    let Some(grant) = lookup_grant(ctx, headers) else {
        return Err(unauthorized(None));
    };

    let resource = canonical_uri().unwrap_or_default();
    match validate_session(&grant, crate::paths::now_ms(), &resource) {
        Ok(()) => Ok(grant),
        Err(_) => Err(unauthorized(None)),
    }
}

/// [`check`] adapted to `axum::middleware::from_fn_with_state`'s shape,
/// parameterized by `required` via the `(AppContext, Scope)` tuple state —
/// mount with
/// `.route_layer(axum::middleware::from_fn_with_state((ctx, required), guard::layer))`
/// in front of any route that needs a fixed, statically-known scope.
/// `/mcp`'s JSON-RPC dispatch resolves its scope per tool call instead, so it
/// calls [`check`] directly rather than mounting this.
pub async fn layer(
    State((ctx, required)): State<(AppContext, Scope)>,
    request: Request,
    next: Next,
) -> Response {
    match check(&ctx, required, request.headers()).await {
        Ok(_grant) => next.run(request).await,
        Err(response) => response,
    }
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/guard.rs"]
mod tests;
