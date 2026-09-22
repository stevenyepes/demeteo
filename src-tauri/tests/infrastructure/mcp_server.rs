// Tests extracted from `src-tauri/src/commands/mcp_server.rs` (mirrored-tests
// convention). `super` = that module.
//
// `write_mcp_server_enabled` takes `ctx: AppContext` because its live-toggle
// half (`demeteo_core::adapters::mcp::set_enabled`) needs the whole context
// to bind the `/mcp` router — same reason `tests/infrastructure/oauth.rs`
// states for not constructing a Tauri `State`, one level further in: even
// the plain `AppContext` here is more than this ticket's command-core test
// should build (AGENTS.md §3: never construct the twenty-odd-port thing in
// a test when the code under test only reads one port). That live-bind
// effect is proven directly against `set_enabled` in
// `crates/demeteo-core/tests/adapters/mcp/gating.rs`. What this file pins
// is `read_mcp_server_status`'s decode of the exact setting `set_enabled`
// persists (`mcp_server_enabled`, `"true"`/`"false"`) — seeded here the same
// way, against a real `SqliteAdapter`.

use super::*;
use crate::adapters::database::SqliteAdapter;
use rusqlite::Connection;

fn store() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

#[test]
fn absent_enabled_key_reports_disabled_with_no_url() {
    let db = store();
    let status = read_mcp_server_status(&db);
    assert_eq!(
        status,
        McpServerStatus {
            enabled: false,
            url: None,
        }
    );
}

#[test]
fn enabled_true_with_nothing_bound_reports_enabled_with_no_url() {
    let db = store();
    db.app_setting_set(MCP_SERVER_ENABLED_KEY, "true").unwrap();

    let status = read_mcp_server_status(&db);

    assert_eq!(
        status,
        McpServerStatus {
            enabled: true,
            url: None,
        }
    );
}

#[test]
fn enabled_false_reports_disabled_with_no_url() {
    let db = store();
    db.app_setting_set(MCP_SERVER_ENABLED_KEY, "false").unwrap();

    let status = read_mcp_server_status(&db);

    assert_eq!(
        status,
        McpServerStatus {
            enabled: false,
            url: None,
        }
    );
}

#[test]
fn unrecognized_enabled_value_defaults_to_disabled() {
    let db = store();
    db.app_setting_set(MCP_SERVER_ENABLED_KEY, "yes").unwrap();

    assert!(!read_mcp_server_status(&db).enabled);
}

fn unique_temp_path(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "demeteo_mcp_skill_test_{}_{}_{}_{}",
        nanos,
        std::process::id(),
        count,
        label,
    ))
}

#[test]
fn write_mcp_skill_writes_frontmatter_fenced_markdown() {
    let dest = unique_temp_path("skill_md");

    write_mcp_skill(&dest).unwrap();

    let written = std::fs::read_to_string(&dest).unwrap();
    let _ = std::fs::remove_file(&dest);
    assert!(!written.is_empty());
    assert!(written.starts_with("---"));
}

#[test]
fn write_mcp_skill_errors_on_missing_parent_dir() {
    let dest = unique_temp_path("missing_parent")
        .join("nested")
        .join("skill.md");

    let result = write_mcp_skill(&dest);

    assert!(result.is_err());
}

/// Binds an ephemeral port, reads a raw HTTP/1.1 request, writes back
/// `body` verbatim as the response, then drops the listener — a minimal
/// hand-rolled responder rather than pulling in a mock-HTTP dev-dependency
/// for three cases.
async fn respond_once(status_line: &str, body: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener.local_addr().expect("listener has a local addr");
    let response = format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept one connection");
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.shutdown().await;
    });
    format!("http://{addr}/mcp")
}

#[tokio::test]
async fn unreachable_connection_reports_the_error_reason() {
    // Bind then immediately drop: the port is very likely still free, so a
    // connection attempt gets refused rather than accepted.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral loopback port");
    let addr = listener.local_addr().expect("listener has a local addr");
    drop(listener);

    let client = reqwest::Client::new();
    let result = probe_mcp_connection(&client, &format!("http://{addr}/mcp")).await;

    assert!(matches!(result, McpConnectionTest::Unreachable { .. }));
}

#[tokio::test]
async fn reachable_response_reports_the_tool_count() {
    let url = respond_once(
        "HTTP/1.1 200 OK",
        r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"a"},{"name":"b"}]}}"#,
    )
    .await;

    let client = reqwest::Client::new();
    let result = probe_mcp_connection(&client, &url).await;

    assert_eq!(result, McpConnectionTest::Reachable { tool_count: 2 });
}

#[tokio::test]
async fn unexpected_response_shape_reports_unreachable() {
    let url = respond_once("HTTP/1.1 200 OK", r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).await;

    let client = reqwest::Client::new();
    let result = probe_mcp_connection(&client, &url).await;

    assert!(matches!(result, McpConnectionTest::Unreachable { .. }));
}
