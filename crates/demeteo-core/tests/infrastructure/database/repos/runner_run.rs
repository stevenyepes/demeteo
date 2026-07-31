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
