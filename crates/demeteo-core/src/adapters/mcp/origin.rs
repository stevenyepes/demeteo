//! DNS-rebinding / CSRF-to-loopback guard. A local HTTP listener is
//! reachable from any page a user's browser loads, so without an `Origin`
//! check a malicious site's script could hit `127.0.0.1` directly.
//!
//! `Origin` alone leaves a hole: a rebound same-origin `GET` carries none. So
//! `Host` must also name a loopback literal — a rebinding page reaches this
//! listener under its own hostname, which no loopback name can spell. Only the
//! host part is compared, not the port: the port is the configured one, and
//! nothing about a rebinding attack depends on it.
//!
//! This also means a browser-based MCP client on any other origin is refused
//! by design. Mounted
//! by [`super::router`] via `.layer(...)` so [`enforce`] runs ahead of
//! axum's own route/method matching — independent of and strictly ahead of
//! scope resolution and `guard::check` (`docs/MCP_INTEGRATION.md` §9): a
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
    if !host_is_loopback(&request) {
        return forbidden();
    }
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

/// `Host` for HTTP/1.x, the URI authority for HTTP/2 (which has no `Host`
/// header). A request naming neither is refused.
fn host_is_loopback(request: &Request) -> bool {
    let raw = match request.headers().get(axum::http::header::HOST) {
        Some(value) => match value.to_str() {
            Ok(host) => host,
            Err(_) => return false,
        },
        None => match request.uri().authority() {
            Some(authority) => authority.as_str(),
            None => return false,
        },
    };
    let host = match raw.strip_prefix('[') {
        Some(rest) => rest.split_once(']').map(|(ip, _)| format!("[{ip}]")),
        None => Some(raw.split(':').next().unwrap_or(raw).to_string()),
    };
    matches!(host.as_deref(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/origin.rs"]
mod tests;
