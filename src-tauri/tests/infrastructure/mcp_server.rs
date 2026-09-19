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
