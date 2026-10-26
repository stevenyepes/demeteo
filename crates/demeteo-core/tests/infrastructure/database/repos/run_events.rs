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

fn offsets(events: &[RunEvent]) -> Vec<i64> {
    events.iter().map(|e| e.offset).collect()
}

/// Rows of two runs, interleaved: `[(a, 1), (b, 2), (a, 3), (b, 4)]`.
fn interleaved() -> (SqliteAdapter, Vec<i64>) {
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    let ids = ["feat-a", "feat-b", "feat-a", "feat-b"]
        .iter()
        .enumerate()
        .map(|(i, run)| {
            adapter
                .append(run, "step_progress", None, i as i64)
                .unwrap()
        })
        .collect();
    (adapter, ids)
}

#[test]
fn list_after_reads_every_run_in_offset_order() {
    let (adapter, ids) = interleaved();

    let events = adapter.list_after(0, 10).unwrap();

    assert_eq!(offsets(&events), ids);
    let runs: Vec<&str> = events.iter().map(|e| e.run_id.as_str()).collect();
    assert_eq!(runs, ["feat-a", "feat-b", "feat-a", "feat-b"]);
}

#[test]
fn list_after_stops_at_limit() {
    let (adapter, ids) = interleaved();

    assert_eq!(offsets(&adapter.list_after(0, 2).unwrap()), ids[..2]);
    assert_eq!(offsets(&adapter.list_after(ids[1], 2).unwrap()), ids[2..]);
}

#[test]
fn list_after_resumes_between_sparse_offsets() {
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    let first = adapter.append("feat-a", "step_progress", None, 1).unwrap();
    {
        let conn = adapter.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO run_events (id, run_id, kind, payload_json, created_at)
             VALUES (?1, 'feat-b', 'step_progress', NULL, 2)",
            params![first + 10],
        )
        .unwrap();
    }

    let events = adapter.list_after(first + 5, 10).unwrap();

    assert_eq!(offsets(&events), [first + 10]);
}

#[test]
fn list_after_past_the_end_is_empty() {
    let (adapter, ids) = interleaved();

    assert!(adapter.list_after(ids[3], 10).unwrap().is_empty());
    assert!(adapter.list_after(ids[3] + 100, 10).unwrap().is_empty());
}
