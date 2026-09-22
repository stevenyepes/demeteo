// `set_mcp_server_enabled` (`commands/mcp_server.rs`) and `start_if_enabled`
// from Tauri's `.setup()` (`lib.rs`) both call into `bind_listener` from a
// plain thread holding only a `tokio::runtime::Handle`, never itself
// `block_on`/`enter`-ing that runtime. `bind_loopback`'s
// `TcpListener::from_std` panics without a thread-local runtime context set
// (tokio's own doc on `PollEvented::new`: "This function panics if
// thread-local runtime is not set"). Every other MCP test runs inside
// `#[tokio::test]`, whose test-body thread already has that context entered
// via `block_on` — so none of them can catch a regression here. This test's
// own thread never enters the runtime it hands to `set_enabled`, matching
// the real caller's shape.
//
// Its own OS process for the same reason `mcp_gating.rs` gets one:
// `adapters::mcp::LISTENER` is a process-wide static, and this test binds a
// real listener on it.

use std::sync::Arc;

use demeteo_core::adapters::mcp::set_enabled;
use demeteo_core::adapters::notification_noop::NoopNotificationAdapter;
use demeteo_core::composition::{build_core_context, CoreConfig, ExecutionMode};

const MCP_SERVER_PORT_KEY: &str = "mcp_server_port";

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral loopback port")
        .local_addr()
        .expect("bound listener has a local address")
        .port()
}

#[test]
fn set_enabled_does_not_panic_off_the_runtime_thread() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build a private tokio runtime");
    let handle = rt.handle().clone();

    let dir = std::env::temp_dir().join(format!(
        "demeteo-mcp-off-runtime-thread-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        handle.clone(),
    );
    ctx.app_settings
        .app_setting_set(MCP_SERVER_PORT_KEY, &free_port().to_string())
        .expect("seed mcp_server_port");

    // Neither this test function's own thread nor `set_enabled` itself ever
    // calls `handle.block_on(..)` or `handle.enter()` — reproducing the
    // Tauri command-handler thread exactly.
    set_enabled(ctx, &handle, true);
}
