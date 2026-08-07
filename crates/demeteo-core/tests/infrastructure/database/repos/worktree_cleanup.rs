// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/worktree_cleanup.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::LOCAL_MACHINE;
use crate::ports::worktree_cleanup::MAX_AUTO_ATTEMPTS;
use rusqlite::Connection;

fn db() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

fn fail<'a>(path: &'a str, error: &'a str, now: i64) -> CleanupFailure<'a> {
    CleanupFailure {
        machine_id: LOCAL_MACHINE,
        path,
        feature_id: None,
        error,
        now,
    }
}

/// A file-backed database, so a test can prove a row survives the
/// adapter that wrote it.
struct TempDb(std::path::PathBuf);

impl TempDb {
    fn new(tag: &str) -> TempDb {
        let path = std::env::temp_dir().join(format!(
            "demeteo-{tag}-{}-{:?}.sqlite",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        TempDb(path)
    }

    fn open(&self) -> SqliteAdapter {
        SqliteAdapter::new(Connection::open(&self.0).unwrap()).unwrap()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The same stuck directory reported on three runs is one entry with
/// three attempts — the property the whole queue rests on, since a fresh
/// row per attempt would never reach the cap and never be reported.
#[test]
fn repeated_reports_of_one_path_stay_one_entry() {
    let db = db();
    let wt = "/wk/a1b2c3d4";

    let first = db.record_failure(fail(wt, "handle held", 100)).unwrap();
    assert_eq!(first.attempts, 1);
    assert_eq!(first.first_enqueued_at, 100);

    let second = db.record_failure(fail(wt, "still held", 200)).unwrap();
    assert_eq!(second.attempts, 2);
    assert_eq!(second.first_enqueued_at, 100, "stuck-since must not move");
    assert_eq!(second.last_attempt_at, 200);
    assert_eq!(second.last_error, "still held");

    assert_eq!(db.list(LOCAL_MACHINE).unwrap().len(), 1);
}

/// A trailing separator is the same directory. Teardown and a later
/// sweep spell it differently, and two rows retrying the same path would
/// each burn their own budget.
#[test]
fn a_trailing_separator_does_not_fork_the_entry() {
    let db = db();
    db.record_failure(fail("/wk/a1b2c3d4", "boom", 100))
        .unwrap();
    db.record_failure(fail("/wk/a1b2c3d4/", "boom", 200))
        .unwrap();
    db.record_failure(fail("C:\\wk\\a1b2c3d4", "boom", 100))
        .unwrap();
    db.record_failure(fail("C:\\wk\\a1b2c3d4\\", "boom", 200))
        .unwrap();

    let rows = db.list(LOCAL_MACHINE).unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|r| r.attempts == 2));

    assert!(db.record_success(LOCAL_MACHINE, "/wk/a1b2c3d4/").unwrap());
    assert!(db
        .record_success(LOCAL_MACHINE, "C:\\wk\\a1b2c3d4\\")
        .unwrap());
    assert!(db.list(LOCAL_MACHINE).unwrap().is_empty());
}

#[test]
fn a_root_path_is_left_alone() {
    assert_eq!(normalize_queue_path("/"), "/");
    assert_eq!(normalize_queue_path("C:\\"), "C:\\");
    assert_eq!(normalize_queue_path(""), "");
    assert_eq!(normalize_queue_path("/wk/a//"), "/wk/a");
}

/// The same path string on two machines is two different directories,
/// and clearing one must not clear the other.
#[test]
fn entries_are_scoped_to_their_machine() {
    let db = db();
    let wt = "/wk/a1b2c3d4";
    db.record_failure(fail(wt, "boom", 100)).unwrap();
    db.record_failure(CleanupFailure {
        machine_id: "builder-01",
        path: wt,
        feature_id: None,
        error: "boom",
        now: 100,
    })
    .unwrap();

    assert!(db.record_success(LOCAL_MACHINE, wt).unwrap());
    assert!(db.list(LOCAL_MACHINE).unwrap().is_empty());
    assert_eq!(db.list("builder-01").unwrap().len(), 1);
}

/// Deleting a path that was never stuck is the ordinary teardown, so it
/// must be a quiet `false`, not an error.
#[test]
fn clearing_an_unqueued_path_is_not_an_error() {
    let db = db();
    assert!(!db.record_success(LOCAL_MACHINE, "/wk/never-stuck").unwrap());
}

/// The cap is the whole point: it stops being swept and starts asking
/// for a human, without ever leaving the list the user is shown.
#[test]
fn past_the_cap_an_entry_stops_retrying_but_stays_visible() {
    let db = db();
    let stuck = "/wk/stuck";
    let flaky = "/wk/flaky";

    for i in 0..MAX_AUTO_ATTEMPTS as i64 {
        db.record_failure(fail(stuck, "locked", 100 + i)).unwrap();
    }
    db.record_failure(fail(flaky, "locked", 500)).unwrap();

    let listed: Vec<String> = db
        .list(LOCAL_MACHINE)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert_eq!(listed, vec![stuck.to_string(), flaky.to_string()]);

    let due: Vec<String> = db
        .due_for_retry(LOCAL_MACHINE)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert_eq!(due, vec![flaky.to_string()]);

    let entry = db
        .list(LOCAL_MACHINE)
        .unwrap()
        .into_iter()
        .find(|e| e.path == stuck)
        .unwrap();
    assert!(entry.needs_attention());
    assert_eq!(entry.attempts, MAX_AUTO_ATTEMPTS);
}

/// A user-asked retry re-enters the sweep without rewriting how many
/// times the path has actually been tried.
#[test]
fn a_reset_grants_a_new_budget_and_keeps_the_history() {
    let db = db();
    let stuck = "/wk/stuck";
    for i in 0..MAX_AUTO_ATTEMPTS as i64 {
        db.record_failure(fail(stuck, "locked", 100 + i)).unwrap();
    }
    assert!(db.due_for_retry(LOCAL_MACHINE).unwrap().is_empty());

    db.reset_attempts(LOCAL_MACHINE, stuck, 900).unwrap();

    let entry = db.due_for_retry(LOCAL_MACHINE).unwrap().pop().unwrap();
    assert_eq!(entry.path, stuck);
    assert!(!entry.needs_attention());
    assert_eq!(entry.auto_attempts(), 0);
    assert_eq!(
        entry.attempts, MAX_AUTO_ATTEMPTS,
        "the user is shown every attempt, not the attempts since they asked"
    );

    let entry = db.record_failure(fail(stuck, "locked", 1000)).unwrap();
    assert_eq!(entry.attempts, MAX_AUTO_ATTEMPTS + 1);
    assert_eq!(entry.auto_attempts(), 1);
}

/// The feature a leftover belonged to is the only thing that names it —
/// the on-disk segment is an 8-hex prefix. A sweep re-reporting the same
/// path has no feature in hand and must not erase it.
#[test]
fn a_later_report_without_a_feature_keeps_the_one_it_had() {
    let db = db();
    let wt = "/wk/a1b2c3d4";
    db.record_failure(CleanupFailure {
        machine_id: LOCAL_MACHINE,
        path: wt,
        feature_id: Some("f-42"),
        error: "boom",
        now: 100,
    })
    .unwrap();

    let entry = db.record_failure(fail(wt, "boom again", 200)).unwrap();
    assert_eq!(entry.feature_id.as_deref(), Some("f-42"));
}

/// "Retried at startup" is a claim about a *new process*, so the row has
/// to outlive the adapter that wrote it, on a real database file.
#[test]
fn an_entry_survives_a_restart() {
    let tmp = TempDb::new("cleanup-queue");
    let wt = "/wk/a1b2c3d4";
    {
        let db = tmp.open();
        db.record_failure(fail(wt, "handle held", 100)).unwrap();
    }

    let db = tmp.open();
    let due = db.due_for_retry(LOCAL_MACHINE).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].path, wt);
    assert_eq!(due[0].last_error, "handle held");
    assert_eq!(due[0].attempts, 1);
}
