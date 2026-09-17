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
//! headless runner has no consent UI and no reason to host this — and even
//! there only when the persisted `mcp_server_enabled` setting is on (default
//! off; [`start_if_enabled`] is the composition-time check, [`set_enabled`]
//! the live on/off toggle within the same process).

pub mod authorize;
pub mod consent_waiter;
pub mod guard;
mod mcp_handler;
mod metadata;
mod origin;
mod protocol;
mod register;
mod token;

use std::net::SocketAddr;

use crate::state::AppContext;

const MCP_SERVER_PORT_KEY: &str = "mcp_server_port";
const MCP_SERVER_ENABLED_KEY: &str = "mcp_server_enabled";

/// Fixed default loopback port for the MCP HTTP listener. Chosen at
/// implementation time; overridable per-install via the `mcp_server_port`
/// app setting.
const DEFAULT_MCP_SERVER_PORT: u16 = 8765;

/// A currently-bound MCP listener: its canonical resource URI, plus what
/// [`set_enabled`] needs to stop it — a graceful-shutdown sender paired with
/// the serve task's own handle.
struct RunningListener {
    canonical_uri: String,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

static LISTENER: std::sync::Mutex<Option<RunningListener>> = std::sync::Mutex::new(None);

/// Lock [`LISTENER`], recovering rather than panicking if a prior panic
/// while holding it left the mutex poisoned — a second caller finding a
/// panicked predecessor's state stale is not a reason for it to panic too.
fn listener_guard() -> std::sync::MutexGuard<'static, Option<RunningListener>> {
    LISTENER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The server's canonical resource URI (RFC 8707), computed from the
/// actual bound listener address. `None` whenever nothing is currently
/// bound — before the first successful bind, under `ExecutionMode::LocalOnly`,
/// while `mcp_server_enabled` is off, if the configured port could not be
/// bound, or after [`set_enabled`] has stopped a running listener. Every
/// OAuth handler that needs "this server's identity" (metadata
/// `resource`/`issuer`, `resource` validation on `/authorize` and `/token`,
/// audience checks in `guard.rs`) must call this rather than re-deriving the
/// URI from the port setting, since only the bound address is authoritative.
pub fn canonical_uri() -> Option<String> {
    listener_guard()
        .as_ref()
        .map(|listener| listener.canonical_uri.clone())
}

/// Test-only priming hook: some MCP tests drive their own ad hoc router on a
/// listener they bind themselves, without going through [`start_if_enabled`]
/// / [`set_enabled`], and need [`canonical_uri`] to agree with that
/// listener's address. First caller across the whole `--lib` test binary
/// wins — a second call, from any test, returns the value already recorded
/// rather than layering a second, divergent one (see
/// `tests/adapters/mcp/guard.rs`'s `spawn_guarded_route` doc for why).
#[cfg(test)]
fn record_canonical_uri(addr: SocketAddr) -> String {
    let mut listener = listener_guard();
    if let Some(existing) = listener.as_ref() {
        return existing.canonical_uri.clone();
    }
    let uri = canonical_uri_for(addr);
    let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let _ = shutdown_rx.await;
    });
    *listener = Some(RunningListener {
        canonical_uri: uri.clone(),
        shutdown,
        handle,
    });
    uri
}

fn canonical_uri_for(addr: SocketAddr) -> String {
    format!("http://{addr}")
}

/// Build the MCP HTTP router. Exposed so tests can drive it directly against
/// a listener they bind themselves, without going through [`start_if_enabled`].
///
/// The [`origin::enforce`] layer wraps every route added above it,
/// including `.well-known/*` and `/mcp` — `.layer(...)` runs ahead of
/// axum's own route/method matching, so a disallowed `Origin` never reaches
/// scope resolution or `guard::check`. [`protocol::enforce_headers`] is
/// `.route_layer(...)`-scoped to `mcp_handler::routes()` alone before it is
/// merged in, so it runs only in front of `/mcp`'s registered `POST` handler
/// — never against `.well-known/*`, and never against `GET`/`DELETE /mcp`
/// (unmatched on that path, so axum's own 405 answers them without either
/// layer running).
pub fn router(ctx: AppContext) -> axum::Router {
    axum::Router::new()
        .merge(metadata::routes())
        .merge(register::routes())
        .merge(authorize::routes())
        .merge(token::routes())
        .merge(
            mcp_handler::routes().route_layer(axum::middleware::from_fn(protocol::enforce_headers)),
        )
        .layer(axum::middleware::from_fn(origin::enforce))
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

/// Read the configured `mcp_server_enabled` app setting, defaulting to and
/// persisting `"false"` (unset means off) when unset or unrecognized — the
/// same read-with-default idiom as [`resolve_port`].
fn resolve_enabled(ctx: &AppContext) -> bool {
    if let Ok(Some(raw)) = ctx.app_settings.app_setting_get(MCP_SERVER_ENABLED_KEY) {
        match raw.trim() {
            "true" => return true,
            "false" => return false,
            _ => {}
        }
    }
    let _ = ctx
        .app_settings
        .app_setting_set(MCP_SERVER_ENABLED_KEY, "false");
    false
}

/// Bind `127.0.0.1:port` as a plain, synchronous std socket promoted to a
/// tokio listener, rather than `tokio::net::TcpListener::bind`'s async form
/// — callers need the socket already accepting connections before they
/// return, including [`start_if_enabled`] from a synchronous
/// composition-root context with no ambient runtime to `.await` on.
fn bind_loopback(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    std_listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(std_listener)
}

/// Bind the configured port and start serving on `runtime`, returning the
/// listener record for the caller to store. Shared by [`start_if_enabled`]
/// and [`set_enabled`]'s turn-on path; neither touches [`LISTENER`] here —
/// each holds the lock across its own decide-and-act sequence instead, so
/// the check and the store are one critical section rather than two. A bind
/// failure disables the MCP surface for the run instead of retrying on a
/// different port: a silently-different port would break the
/// canonical-URI-stability property token audience checks rely on.
fn bind_listener(ctx: AppContext, runtime: &tokio::runtime::Handle) -> Option<RunningListener> {
    let port = resolve_port(&ctx);
    let listener = match bind_loopback(port) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!(
                "[Mcp] failed to bind 127.0.0.1:{port}: {e} — MCP surface disabled for this run"
            );
            return None;
        }
    };
    let addr: SocketAddr = match listener.local_addr() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[Mcp] bound listener has no local address: {e} — MCP surface disabled for this run");
            return None;
        }
    };
    let uri = canonical_uri_for(addr);
    eprintln!("[Mcp] listening on {uri}");

    let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
    let app = router(ctx);
    let handle = runtime.spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
        if let Err(e) = result {
            eprintln!("[Mcp] listener exited: {e}");
        }
    });

    Some(RunningListener {
        canonical_uri: uri,
        shutdown,
        handle,
    })
}

/// Composition-time gate: start the MCP HTTP listener on `runtime`, but only
/// when the persisted `mcp_server_enabled` setting is on. Caller's
/// responsibility to only call this under `ExecutionMode::Router` (see
/// `composition::mod`) — this function has no opinion on execution mode
/// itself. Holds the [`LISTENER`] lock across the is-it-already-running
/// check and the store, same as [`set_enabled`], so two overlapping callers
/// can't both decide to bind.
pub fn start_if_enabled(ctx: AppContext, runtime: &tokio::runtime::Handle) {
    if !resolve_enabled(&ctx) {
        return;
    }
    let mut listener = listener_guard();
    if listener.is_some() {
        return;
    }
    *listener = bind_listener(ctx, runtime);
}

/// The live-toggle seam a Tauri command calls: persists `enabled`, then
/// binds or stops the listener in this process to match. Turning on while
/// already running, or off while already stopped, is a no-op. Holds the
/// [`LISTENER`] lock across the whole persist-decide-act sequence — two
/// overlapping calls (e.g. `true` then `false` in quick succession) would
/// otherwise interleave their separate check/store steps and leave the
/// persisted setting disagreeing with whether a listener is actually bound.
pub fn set_enabled(ctx: AppContext, runtime: &tokio::runtime::Handle, enabled: bool) {
    let mut listener = listener_guard();
    let _ = ctx.app_settings.app_setting_set(
        MCP_SERVER_ENABLED_KEY,
        if enabled { "true" } else { "false" },
    );

    if enabled {
        if listener.is_some() {
            return;
        }
        *listener = bind_listener(ctx, runtime);
    } else if let Some(running) = listener.take() {
        let _ = running.shutdown.send(());
        // Join the serve task in the background rather than aborting it
        // outright, so an in-flight request gets to finish under the
        // graceful-shutdown signal just sent rather than being cut off.
        runtime.spawn(async move {
            let _ = running.handle.await;
        });
    }
}
