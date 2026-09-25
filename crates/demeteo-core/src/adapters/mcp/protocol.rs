//! The `2026-07-28` stateless-revision transport surface layered on top of
//! the existing `POST /mcp` JSON-RPC dispatch: the `Mcp-Method` / `Mcp-Name`
//! header-vs-body consistency check, and the `server/discover` and
//! `initialize` response bodies. Mounted by [`super::mod`]'s `router()` via
//! `.route_layer(...)` scoped to the `/mcp` route alone — never the whole
//! router, so `.well-known/*` is untouched — and strictly ahead of
//! [`super::mcp_handler`]'s scope resolution and `guard::check`: a request
//! that fails a check here never reaches dispatch
//! (`docs/MCP_INTEGRATION.md` §4).
//!
//! This revision is stateless by design: neither this module nor
//! `mcp_handler` reads or acts on `Mcp-Session-Id` or `Last-Event-ID`, and no
//! per-connection state is kept anywhere. That statelessness is also why
//! `MCP-Protocol-Version` is no longer checked against one pinned string: a
//! real client (verified against Claude Code) always opens with `initialize`
//! regardless of revision, and Demeteo has nowhere to remember what a prior
//! request on the same TCP connection negotiated — HTTP requests here are
//! not guaranteed to share one. `initialize` answers with whatever
//! `protocolVersion` the client itself declared (falling back to
//! [`SUPPORTED_PROTOCOL_VERSION`] when absent or malformed): Demeteo's tool
//! surface doesn't vary across recent revisions, so there is nothing to gate
//! on, and the real MCP negotiation contract already puts the compatibility
//! decision on the client — it disconnects on its own if it can't cope with
//! the version a server names. `server/discover` still advertises exactly
//! one revision for a client that wants to know before committing.
//!
//! `server/discover`'s exact response shape has no authoritative source in
//! this repo (no vendored `2026-07-28` spec text exists here; see
//! `docs/MCP_INTEGRATION.md` §4) — the shape below is a best-effort,
//! internally-consistent placeholder, not a verified spec contract. The same
//! caveat applies to `initialize`'s `capabilities` object.

use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

pub const SUPPORTED_PROTOCOL_VERSION: &str = "2026-07-28";
pub const ERR_HEADER_MISMATCH: i64 = -32020;

/// Matches `axum_core`'s own default `Bytes`/`Json` extractor body-size
/// limit. [`enforce_headers`] reads the whole body ahead of
/// [`super::mcp_handler`]'s `Json` extractor, so it must carry this same cap
/// itself — passing `usize::MAX` here would silently remove the only limit
/// this route ever had, since the extractor downstream never gets to enforce
/// its own — that would be a pre-auth memory-exhaustion hole.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

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

/// `true` when `version` is shaped like an ISO date (`YYYY-MM-DD`) — cheap
/// sanity check before echoing a client-declared `protocolVersion` back in
/// [`initialize_response`], so a malformed value doesn't round-trip into
/// Demeteo's own response. Not a calendar validation; `format!` on the
/// digits is enough to catch garbage without pulling in a date crate for one
/// shape check.
fn looks_like_protocol_version(version: &str) -> bool {
    let bytes = version.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => true,
            _ => b.is_ascii_digit(),
        })
}

/// `initialize`: answers with whatever `protocolVersion` the client itself
/// declared in `params`, falling back to [`SUPPORTED_PROTOCOL_VERSION`] when
/// absent or not shaped like a revision. See this module's doc comment for
/// why echoing rather than gatekeeping is correct here — verified against a
/// real client (`docs/MCP_INTEGRATION.md` §4).
pub(super) fn initialize_response(id: Value, params: &Value) -> Response {
    let protocol_version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|v| looks_like_protocol_version(v))
        .unwrap_or(SUPPORTED_PROTOCOL_VERSION);

    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": protocol_version,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "demeteo",
                "version": env!("CARGO_PKG_VERSION"),
            },
        },
    }))
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
/// (`docs/MCP_INTEGRATION.md` §4): buffers the body to compare it against
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

    let request = Request::from_parts(parts, Body::from(bytes));
    next.run(request).await
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/protocol_version.rs"]
mod tests;
