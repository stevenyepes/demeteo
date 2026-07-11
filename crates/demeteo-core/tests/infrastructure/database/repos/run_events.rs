// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/run_events.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use rusqlite::Connection;

#[test]
fn append_scrubs_secrets_from_payload() {
    // The security-relevant invariant (M7.2, §6): a credential that
    // slips into an event payload upstream never reaches the
    // append-only, laptop-streamed log verbatim.
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    let leaky = "\"clone failed: https://x-access-token:ghp_0123456789abcdef0123456789abcdefABCD@github.com/o/r.git\"";
    adapter.append("run-1", "failed", Some(leaky), 1).unwrap();

    let events = adapter.list_since("run-1", 0).unwrap();
    let stored = events[0].payload_json.as_deref().unwrap();
    assert!(
        !stored.contains("ghp_0123456789abcdef"),
        "token leaked: {stored}"
    );
    assert!(stored.contains("***"));
}
