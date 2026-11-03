// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/runner_run.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use rusqlite::Connection;

#[test]
fn update_status_scrubs_secrets_from_error() {
    // The `error` column is surfaced verbatim in the laptop's return
    // inbox, so it's the most-visible secret sink — a token in a
    // failed-run error must not survive the write (M7.2, §6). This
    // also guards the direct `rpc/` failure-path writer, which
    // doesn't go through `run::emit`.
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    adapter.get_or_create("run-1", "{}", "", 1).unwrap();
    let leaky = "push rejected: glpat-ABCDEF0123456789wxyz was revoked";
    adapter
        .update_status("run-1", "failed", None, None, Some(leaky), None, 2)
        .unwrap();

    let stored = adapter.get("run-1").unwrap().unwrap().error.unwrap();
    assert!(
        !stored.contains("glpat-ABCDEF0123456789wxyz"),
        "token leaked: {stored}"
    );
    assert!(stored.contains("***"));
}

#[test]
fn get_or_create_stamps_owner_and_never_rehomes_on_resubmit() {
    // MC-D2 (P0.2): the owning client is stamped at creation and an
    // idempotent re-submit (R9) must NOT re-home the run to a second
    // client — a run stays owned by whoever created it.
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    let first = adapter.get_or_create("run-1", "{}", "client-A", 1).unwrap();
    assert_eq!(first.owner_client_id, "client-A");

    // Re-submit of the same run_id by a different client is a no-op
    // (INSERT OR IGNORE) — owner is unchanged.
    let again = adapter.get_or_create("run-1", "{}", "client-B", 2).unwrap();
    assert_eq!(again.owner_client_id, "client-A");
}

#[test]
fn legacy_rows_read_back_empty_owner() {
    // A run created with no client id (old client) reads back the ""
    // legacy tenant — the documented single bucket, not a boundary.
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    let run = adapter.get_or_create("run-legacy", "{}", "", 1).unwrap();
    assert_eq!(run.owner_client_id, "");
}

/// Counted from the run row whatever its status, so a Hub start that failed
/// before its Feature row existed still counts; a desktop's run, which
/// records nothing, and a run older than the window do not.
#[test]
fn the_hub_committed_total_sums_recorded_caps_inside_the_window() {
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    adapter.get_or_create("run-old", "{}", "hub", 10).unwrap();
    adapter.record_hub_commitment("run-old", 700).unwrap();
    adapter
        .get_or_create("run-failed", "{}", "hub", 100)
        .unwrap();
    adapter.record_hub_commitment("run-failed", 250).unwrap();
    adapter
        .update_status(
            "run-failed",
            "failed",
            None,
            None,
            Some("clone failed"),
            None,
            101,
        )
        .unwrap();
    adapter.get_or_create("run-hub", "{}", "hub", 200).unwrap();
    adapter.record_hub_commitment("run-hub", 125).unwrap();
    adapter
        .get_or_create("run-desktop", "{}", "laptop-1", 300)
        .unwrap();

    assert_eq!(adapter.hub_committed_cents_since(100), Ok(375));
    assert_eq!(adapter.hub_committed_cents_since(301), Ok(0));
}

#[test]
fn recording_a_commitment_on_no_run_is_an_error() {
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    assert!(adapter.record_hub_commitment("run-missing", 100).is_err());
    assert_eq!(adapter.hub_committed_cents_since(0), Ok(0));
}
