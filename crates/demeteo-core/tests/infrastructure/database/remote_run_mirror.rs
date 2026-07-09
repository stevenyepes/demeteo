use rusqlite::Connection;

use super::super::super::SqliteAdapter;
use crate::ports::remote_run_mirror::RemoteRunMirrorPort;

fn setup() -> SqliteAdapter {
    let conn = Connection::open_in_memory().unwrap();
    SqliteAdapter::new(conn).unwrap()
}

#[test]
fn upsert_submitted_is_idempotent_by_machine_and_run() {
    let adapter = setup();
    let a = adapter
        .upsert_submitted("m1", "r1", Some("p1"), Some("f-eager"), "Add OAuth", 1000)
        .unwrap();
    assert_eq!(a.status, "pending");
    assert_eq!(a.project_id.as_deref(), Some("p1"));
    // The eager shadow feature id is present from submit time — the
    // run is navigable before the first reconcile.
    assert_eq!(a.feature_id.as_deref(), Some("f-eager"));

    // Re-submitting the same (machine_id, run_id) is a no-op — same
    // semantics as the runner's own idempotent `submit_run` (R9).
    let b = adapter
        .upsert_submitted(
            "m1",
            "r1",
            Some("different-project"),
            Some("f-other"),
            "different title",
            2000,
        )
        .unwrap();
    assert_eq!(b.project_id.as_deref(), Some("p1"));
    assert_eq!(b.feature_id.as_deref(), Some("f-eager"));
    assert_eq!(b.title, "Add OAuth");
    assert_eq!(b.created_at, 1000);

    assert_eq!(adapter.list().unwrap().len(), 1);
}

#[test]
fn same_run_id_on_different_machines_is_distinct() {
    let adapter = setup();
    adapter
        .upsert_submitted("m1", "r1", None, None, "on m1", 1000)
        .unwrap();
    adapter
        .upsert_submitted("m2", "r1", None, None, "on m2", 1000)
        .unwrap();
    assert_eq!(adapter.list().unwrap().len(), 2);
    assert!(adapter.get("m1", "r1").unwrap().is_some());
    assert!(adapter.get("m2", "r1").unwrap().is_some());
}

#[test]
fn update_status_preserves_fields_not_passed() {
    let adapter = setup();
    adapter
        .upsert_submitted("m1", "r1", Some("p1"), Some("f-eager"), "Add OAuth", 1000)
        .unwrap();

    adapter
        .update_status("m1", "r1", "running", None, None, None, None, 3, 1500)
        .unwrap();
    let row = adapter.get("m1", "r1").unwrap().unwrap();
    assert_eq!(row.status, "running");
    assert_eq!(row.last_offset, 3);
    // A `None` feature_id in update_status never clears the eager id
    // set at submit time (COALESCE semantics).
    assert_eq!(row.feature_id.as_deref(), Some("f-eager"));

    adapter
        .update_status(
            "m1",
            "r1",
            "awaiting_mr",
            None,
            Some("f1"),
            Some("https://example.com/pr/1"),
            Some("feature/add-oauth"),
            7,
            2000,
        )
        .unwrap();
    let row = adapter.get("m1", "r1").unwrap().unwrap();
    assert_eq!(row.status, "awaiting_mr");
    assert_eq!(row.feature_id.as_deref(), Some("f1"));
    assert_eq!(row.pr_url.as_deref(), Some("https://example.com/pr/1"));
    assert_eq!(row.pushed_branch.as_deref(), Some("feature/add-oauth"));
    assert_eq!(row.last_offset, 7);

    // A later call with a *lower* offset (e.g. a stale poll response
    // arriving out of order) must never regress `last_offset`.
    adapter
        .update_status("m1", "r1", "awaiting_mr", None, None, None, None, 2, 2500)
        .unwrap();
    let row = adapter.get("m1", "r1").unwrap().unwrap();
    assert_eq!(row.last_offset, 7);
    // feature_id/pr_url stick around even though this call passed `None`.
    assert_eq!(row.feature_id.as_deref(), Some("f1"));
    assert_eq!(row.pr_url.as_deref(), Some("https://example.com/pr/1"));
}

#[test]
fn mark_notified_is_independent_of_status_updates() {
    let adapter = setup();
    adapter
        .upsert_submitted("m1", "r1", None, None, "Add OAuth", 1000)
        .unwrap();
    adapter
        .update_status(
            "m1",
            "r1",
            "failed",
            Some("boom"),
            None,
            None,
            None,
            0,
            1500,
        )
        .unwrap();
    adapter.mark_notified("m1", "r1", "failed").unwrap();

    let row = adapter.get("m1", "r1").unwrap().unwrap();
    assert_eq!(row.status, "failed");
    assert_eq!(row.error.as_deref(), Some("boom"));
    assert_eq!(row.last_notified_status.as_deref(), Some("failed"));
}

#[test]
fn get_returns_none_for_unknown_run() {
    let adapter = setup();
    assert!(adapter.get("m1", "does-not-exist").unwrap().is_none());
}
