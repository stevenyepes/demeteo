//! The `2026-07-28` stateless-revision transport surface layered on top of
//! the existing `POST /mcp` JSON-RPC dispatch: the `MCP-Protocol-Version` /
//! `Mcp-Method` / `Mcp-Name` header-vs-body consistency check, and the
//! `server/discover` response body. Mounted by [`super::mod`]'s `router()`
//! via `.route_layer(...)` scoped to the `/mcp` route alone — never the
//! whole router, so `.well-known/*` is untouched — and strictly ahead of
//! [`super::mcp_handler`]'s scope resolution and `guard::check`: a request
//! that fails a check here never reaches dispatch
//! (implementation-spec.md §6).
//!
//! This revision is stateless by design: neither this module nor
//! `mcp_handler` reads or acts on `Mcp-Session-Id` or `Last-Event-ID`.
//! `initialize` is answered but not implemented — no session, no capability
//! negotiation — solely so a legacy client gets a correctly-named diagnostic
//! naming the versions this server actually understands, rather than a
//! generic "method not found".
//!
//! `server/discover`'s exact response shape has no authoritative source in
//! this repo (implementation-spec.md §7 Open Question 1: no vendored
//! `2026-07-28` spec text exists here) — the shape below is a best-effort,
//! internally-consistent placeholder, not a verified spec contract.

use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

pub const SUPPORTED_PROTOCOL_VERSION: &str = "2026-07-28";
pub const ERR_HEADER_MISMATCH: i64 = -32020;
pub const ERR_UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// Matches `axum_core`'s own default `Bytes`/`Json` extractor body-size
/// limit. [`enforce_headers`] reads the whole body ahead of
/// [`super::mcp_handler`]'s `Json` extractor, so it must carry this same cap
/// itself — passing `usize::MAX` here would silently remove the only limit
/// this route ever had, since the extractor downstream never gets to enforce
/// its own (critic review, Critical Issue #1: a pre-auth memory-exhaustion
/// regression).
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

const HEADER_PROTOCOL_VERSION: &str = "mcp-protocol-version";
const HEADER_METHOD: &str = "mcp-method";
const HEADER_NAME: &str = "mcp-name";

fn json_rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

fn header_mismatch_response(id: Value) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json_rpc_error(
            id,
            ERR_HEADER_MISMATCH,
            "Mcp-Method/Mcp-Name header does not match the request body",
            None,
        )),
    )
        .into_response()
}

/// Shared by the header-vs-body check below (an unsupported declared
/// version) and [`super::mcp_handler`]'s `"initialize"` arm (which answers
/// this unconditionally). HTTP `200`, matching how `-32601`/`-32602` already
/// pair with `200` elsewhere in this file's sibling `mcp_handler.rs` — the
/// error lives in the JSON-RPC envelope, not the HTTP status
/// (implementation-spec.md §7 Open Question 4).
pub(super) fn unsupported_version_response(id: Value) -> Response {
    Json(json_rpc_error(
        id,
        ERR_UNSUPPORTED_PROTOCOL_VERSION,
        "unsupported MCP-Protocol-Version",
        Some(json!({ "supported": [SUPPORTED_PROTOCOL_VERSION] })),
    ))
    .into_response()
}

/// `server/discover`: names [`SUPPORTED_PROTOCOL_VERSION`] under both a
/// singular and a list field so a caller can read whichever shape it
/// expects, plus minimal server identity. See this module's doc comment for
/// why the exact shape is a placeholder.
pub(super) fn discover(id: Value) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": SUPPORTED_PROTOCOL_VERSION,
            "supported": [SUPPORTED_PROTOCOL_VERSION],
            "serverInfo": {
                "name": "demeteo",
                "version": env!("CARGO_PKG_VERSION"),
            },
        },
    }))
    .into_response()
}

/// `true` when a declared `Mcp-Method`/`Mcp-Name` header is present and
/// disagrees with the parsed body's `method`/`params.name`. A header that is
/// simply absent is not a mismatch — only a declared-and-wrong value is.
fn header_body_mismatch(headers: &axum::http::HeaderMap, body: &Value) -> bool {
    if let Some(declared) = headers.get(HEADER_METHOD).and_then(|v| v.to_str().ok()) {
        if Some(declared) != body.get("method").and_then(Value::as_str) {
            return true;
        }
    }
    if let Some(declared) = headers.get(HEADER_NAME).and_then(|v| v.to_str().ok()) {
        if Some(declared) != body.pointer("/params/name").and_then(Value::as_str) {
            return true;
        }
    }
    false
}

/// The `/mcp`-scoped `.route_layer(...)` middleware
/// (implementation-spec.md §3/§6): buffers the body to compare it against
/// `Mcp-Method`/`Mcp-Name`, then restores it unchanged so
/// [`super::mcp_handler::handle`]'s own `Json` extractor still sees the
/// original bytes. A body that isn't valid JSON is left for that extractor
/// to reject on its own terms — this check has nothing to compare against
/// and stays out of the way rather than inventing a second error shape for
/// the same condition.
pub(super) async fn enforce_headers(request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };

    let parsed_body: Option<Value> = serde_json::from_slice(&bytes).ok();
    let id = parsed_body
        .as_ref()
        .and_then(|body| body.get("id"))
        .cloned()
        .unwrap_or(Value::Null);

    if let Some(body) = &parsed_body {
        if header_body_mismatch(&parts.headers, body) {
            return header_mismatch_response(id);
        }
    }

    if let Some(declared_version) = parts
        .headers
        .get(HEADER_PROTOCOL_VERSION)
        .and_then(|v| v.to_str().ok())
    {
        if declared_version != SUPPORTED_PROTOCOL_VERSION {
            return unsupported_version_response(id);
        }
    }

    let request = Request::from_parts(parts, Body::from(bytes));
    next.run(request).await
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/protocol_version.rs"]
mod tests;
