//! DNS-rebinding / CSRF-to-loopback guard. A local HTTP listener is
//! reachable from any page a user's browser loads, so without an `Origin`
//! check a malicious site's script could hit `127.0.0.1` directly. Mounted
//! by [`super::router`] via `.layer(...)` so [`enforce`] runs ahead of
//! axum's own route/method matching — independent of and strictly ahead of
//! scope resolution and `guard::check` (implementation-spec.md §6): a
//! request failing this check must never reach dispatch.

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::canonical_uri;

/// Same shape as `guard.rs`'s `challenge()`: a bare status, since there is
/// no bearer-token context here for a `WWW-Authenticate` value to name.
fn forbidden() -> Response {
    StatusCode::FORBIDDEN.into_response()
}

/// Allows a request through when `Origin` is absent, or present and exactly
/// equal to this server's own [`canonical_uri`]. Fails closed in every
/// other case — including a header present but not valid UTF-8 — rather
/// than falling back to a permissive default. The comparison is
/// deliberately the whole value: never scheme-less host only, never a
/// wildcard, either of which would reopen the exact rebinding hole loopback
/// binding exists to close.
pub async fn enforce(request: Request, next: Next) -> Response {
    let Some(origin) = request.headers().get(axum::http::header::ORIGIN) else {
        return next.run(request).await;
    };
    let Ok(origin) = origin.to_str() else {
        return forbidden();
    };
    if canonical_uri().as_deref() == Some(origin) {
        next.run(request).await
    } else {
        forbidden()
    }
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/origin.rs"]
mod tests;
