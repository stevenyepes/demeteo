// The MCP listener only binds when
// `mcp_server_enabled` is on, and the live toggle (`set_enabled`) can
// start/stop it within one process.
//
// A standalone integration-test binary (see the `[[test]]` entry in
// `Cargo.toml`), not the `#[path]`-into-`src/` mirrored-tests convention
// every other `tests/adapters/mcp/*.rs` file uses. `adapters::mcp`'s
// listener state is a process-wide static; every other MCP test primes it
// once via `record_canonical_uri` and assumes it never changes again for
// the rest of the run. This file's whole point is to bind and stop real
// listeners on that same static, which would race those tests if it ran in
// their shared `--lib` binary — its own OS process gives it a private copy
// instead. Within *this* binary, `enabled_setting...` is deliberately the
// only test that touches the listener, so it doesn't need to race itself
// either.

use std::net::Ipv4Addr;
use std::sync::Arc;

use demeteo_core::adapters::mcp::{canonical_uri, set_enabled, start_if_enabled};
use demeteo_core::adapters::notification_noop::NoopNotificationAdapter;
use demeteo_core::composition::{build_core_context, CoreConfig, ExecutionMode};
use demeteo_core::state::AppContext;

// Mirrors the private `MCP_SERVER_PORT_KEY` / `MCP_SERVER_ENABLED_KEY`
// constants in `src/adapters/mcp/mod.rs` — not reachable from an
// integration test, which only sees the crate's public API.
const MCP_SERVER_PORT_KEY: &str = "mcp_server_port";
const MCP_SERVER_ENABLED_KEY: &str = "mcp_server_enabled";

fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-gating-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    )
}

/// A free loopback port, discovered by binding an ephemeral one and
/// releasing it immediately. Each test seeds its own `mcp_server_port`
/// setting with this rather than relying on the default, so a real bind
/// never collides with a dev instance or another test.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral loopback port")
        .local_addr()
        .expect("bound listener has a local address")
        .port()
}

async fn assert_nothing_listening(port: u16, why: &str) {
    let result = tokio::net::TcpStream::connect(("127.0.0.1", port)).await;
    assert!(
        result.is_err(),
        "connected to 127.0.0.1:{port} despite {why}"
    );
}

#[tokio::test]
async fn unset_or_false_mcp_server_enabled_refuses_connections() {
    let ctx = fixture("unset");
    let port = free_port();
    ctx.app_settings
        .app_setting_set(MCP_SERVER_PORT_KEY, &port.to_string())
        .expect("seed mcp_server_port");
    // mcp_server_enabled deliberately left unset.
    start_if_enabled(ctx, &tokio::runtime::Handle::current());
    assert_nothing_listening(port, "mcp_server_enabled being unset").await;

    let ctx = fixture("false");
    let port = free_port();
    ctx.app_settings
        .app_setting_set(MCP_SERVER_PORT_KEY, &port.to_string())
        .expect("seed mcp_server_port");
    ctx.app_settings
        .app_setting_set(MCP_SERVER_ENABLED_KEY, "false")
        .expect("seed mcp_server_enabled");
    start_if_enabled(ctx, &tokio::runtime::Handle::current());
    assert_nothing_listening(port, "mcp_server_enabled=false").await;
}

#[tokio::test]
async fn enabled_setting_binds_loopback_and_the_live_toggle_starts_and_stops_it() {
    let ctx = fixture("enabled");
    let port = free_port();
    ctx.app_settings
        .app_setting_set(MCP_SERVER_PORT_KEY, &port.to_string())
        .expect("seed mcp_server_port");
    ctx.app_settings
        .app_setting_set(MCP_SERVER_ENABLED_KEY, "true")
        .expect("seed mcp_server_enabled");
    let runtime = tokio::runtime::Handle::current();

    start_if_enabled(ctx.clone(), &runtime);

    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to the resolved port once mcp_server_enabled=true");

    let resp = reqwest::Client::new()
        .get(format!(
            "http://127.0.0.1:{port}/.well-known/oauth-protected-resource"
        ))
        .send()
        .await
        .expect("request the protected-resource metadata endpoint");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let uri = canonical_uri().expect("a listener is bound once enabled");
    let bound: std::net::SocketAddr = uri
        .strip_prefix("http://")
        .expect("canonical_uri is an http:// URL")
        .parse()
        .expect("canonical_uri's authority parses as a socket address");
    assert_eq!(
        bound.ip(),
        Ipv4Addr::LOCALHOST,
        "listener must never bind 0.0.0.0"
    );
    assert_eq!(bound.port(), port);

    // The live-toggle seam: turning it off stops the listener in this same
    // process, and turning it back on rebinds the same port.
    set_enabled(ctx.clone(), &runtime, false);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_nothing_listening(port, "set_enabled(false)").await;
    assert!(
        canonical_uri().is_none(),
        "canonical_uri must clear on stop"
    );

    set_enabled(ctx.clone(), &runtime, true);
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("set_enabled(true) rebinds the same port");

    set_enabled(ctx, &runtime, false);
}
