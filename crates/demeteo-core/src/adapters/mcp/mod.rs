//! MCP (Model Context Protocol) authorization surface — the in-process HTTP
//! listener and OAuth 2.1 plumbing that let an external MCP client reach the
//! `agent_surface` tool catalog under a human-approved grant. The listener
//! bootstrap, the consent rendezvous, the RFC 9728 / RFC 8414 `.well-known`
//! metadata endpoints, RFC 7591 dynamic client registration, the shared
//! `guard` bearer-token check, `POST /mcp`'s JSON-RPC `tools/list` /
//! `tools/call` dispatch, `GET /authorize`'s PKCE-gated consent flow, and
//! `POST /token`'s code-for-token exchange all exist — [`router`] builds an
//! `axum::Router` carrying the metadata, `/register`, `/authorize`,
//! `/token`, and `/mcp` routes.
//!
//! Binds `127.0.0.1` only, on a fixed, app-setting-configurable port
//! (`mcp_server_port`, default `8765`) rather than an OS-assigned ephemeral
//! one, so an already-configured MCP client's callback/audience URL doesn't
//! change across launches (implementation-spec.md §7 Open Question 5). A
//! bind failure disables the MCP surface for the run instead of retrying on
//! a different port: a silently-different port would break the
//! canonical-URI-stability property token audience checks rely on.
//!
//! Started only under `ExecutionMode::Router` (see `composition::mod`) — the
//! headless runner has no consent UI and no reason to host this.

pub mod authorize;
pub mod consent_waiter;
pub mod guard;
mod mcp_handler;
mod metadata;
mod register;
mod token;

use std::net::SocketAddr;
use std::sync::OnceLock;

use crate::state::AppContext;

const MCP_SERVER_PORT_KEY: &str = "mcp_server_port";

/// Fixed default loopback port for the MCP HTTP listener. Chosen at
/// implementation time; overridable per-install via the `mcp_server_port`
/// app setting.
const DEFAULT_MCP_SERVER_PORT: u16 = 8765;

static CANONICAL_URI: OnceLock<String> = OnceLock::new();

/// The server's canonical resource URI (RFC 8707), computed once from the
/// actual bound listener address. `None` until the listener has bound
/// successfully — which never happens under `ExecutionMode::LocalOnly`, and
/// may not happen under `ExecutionMode::Router` if the configured port could
/// not be bound. Every OAuth handler that needs "this server's identity"
/// (metadata `resource`/`issuer`, `resource` validation on `/authorize` and
/// `/token`, audience checks in `guard.rs`) must call this rather than
/// re-deriving the URI from the port setting, since only the bound address
/// is authoritative.
pub fn canonical_uri() -> Option<String> {
    CANONICAL_URI.get().cloned()
}

/// Compute this process's canonical resource URI from a bound loopback
/// address and record it — the one place that string gets built, so every
/// caller of [`canonical_uri`] agrees on it. Idempotent via [`OnceLock`]: a
/// second call (a test binding its own listener, e.g.) returns the value
/// [`serve`] already recorded rather than deriving a second, divergent one.
fn record_canonical_uri(addr: SocketAddr) -> String {
    CANONICAL_URI
        .get_or_init(|| format!("http://{addr}"))
        .clone()
}

/// Build the MCP HTTP router. Exposed so tests can drive it directly against
/// a listener they bind themselves, without going through [`start`].
pub fn router(ctx: AppContext) -> axum::Router {
    axum::Router::new()
        .merge(metadata::routes())
        .merge(register::routes())
        .merge(authorize::routes())
        .merge(token::routes())
        .merge(mcp_handler::routes())
        .with_state(ctx)
}

/// Read the configured `mcp_server_port` app setting, defaulting to and
/// persisting [`DEFAULT_MCP_SERVER_PORT`] when unset or unparseable — the
/// same read-with-default idiom as
/// `application::remote_runs::client_id::client_install_id`.
fn resolve_port(ctx: &AppContext) -> u16 {
    if let Ok(Some(raw)) = ctx.app_settings.app_setting_get(MCP_SERVER_PORT_KEY) {
        if let Ok(port) = raw.trim().parse::<u16>() {
            return port;
        }
    }
    let _ = ctx
        .app_settings
        .app_setting_set(MCP_SERVER_PORT_KEY, &DEFAULT_MCP_SERVER_PORT.to_string());
    DEFAULT_MCP_SERVER_PORT
}

/// Bind the loopback listener and start serving. Spawned onto `runtime` by
/// [`start`] rather than run inline, since binding is async and this is
/// called from a synchronous composition-root context.
async fn serve(ctx: AppContext) {
    let port = resolve_port(&ctx);
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!(
                "[Mcp] failed to bind 127.0.0.1:{port}: {e} — MCP surface disabled for this run"
            );
            return;
        }
    };
    let addr: SocketAddr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[Mcp] bound listener has no local address: {e} — MCP surface disabled for this run");
            return;
        }
    };
    let uri = record_canonical_uri(addr);
    eprintln!("[Mcp] listening on {uri}");

    if let Err(e) = axum::serve(listener, router(ctx)).await {
        eprintln!("[Mcp] listener exited: {e}");
    }
}

/// Start the MCP HTTP listener on `runtime`. Caller's responsibility to only
/// call this under `ExecutionMode::Router` (see `composition::mod`) — this
/// function has no opinion on execution mode itself.
pub fn start(ctx: AppContext, runtime: &tokio::runtime::Handle) {
    runtime.spawn(serve(ctx));
}
