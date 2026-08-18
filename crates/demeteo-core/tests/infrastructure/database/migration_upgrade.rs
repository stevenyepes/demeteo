//! Opening a database whose `refinery_schema_history` was written by an
//! *older* refinery must not be a hard failure.
//!
//! refinery records a checksum per applied migration and, by default, aborts
//! when a stored checksum disagrees with the embedded migration — the
//! "divergent migration" error. That default is wrong for a shipped desktop
//! app: the disagreement is between two library versions, not between two
//! schemas, and aborting turns a dependency bump into an app that refuses to
//! start against every database already on disk. `migration::run` therefore
//! sets `set_abort_divergent(false)`.
//!
//! Nothing in a fresh-database test can catch a regression here, because a
//! fresh database has no prior history to disagree with. These tests migrate
//! for real and then rewrite the recorded checksums, which is what a database
//! carried across a refinery upgrade actually looks like: the schema is
//! present and correct, only the bookkeeping disagrees.

use crate::adapters::database::{migration, SqliteAdapter};
use rusqlite::Connection;

/// Replace every recorded checksum with one no embedded migration can
/// produce — the worst case an upgrade could present, and the one that trips
/// refinery's default abort-on-divergent behaviour.
fn forge_foreign_checksums(conn: &Connection) {
    let rewritten = conn
        .execute(
            "UPDATE refinery_schema_history SET checksum = '0000000000000000000'",
            [],
        )
        .unwrap();
    assert!(
        rewritten > 0,
        "no history rows to diverge — the migration run recorded nothing"
    );
}

fn applied_versions(conn: &Connection) -> Vec<i32> {
    conn.prepare("SELECT version FROM refinery_schema_history ORDER BY version ASC")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn a_history_of_foreign_checksums_does_not_block_startup() {
    let mut conn = Connection::open_in_memory().unwrap();
    migration::run(&mut conn).unwrap();
    let before = applied_versions(&conn);

    forge_foreign_checksums(&conn);

    migration::run(&mut conn).expect("divergent stored checksums must not abort the migration run");
    assert_eq!(
        before,
        applied_versions(&conn),
        "a divergent history must not re-apply or drop versions"
    );
}

#[test]
fn the_schema_survives_a_run_over_a_divergent_history() {
    let mut conn = Connection::open_in_memory().unwrap();
    migration::run(&mut conn).unwrap();
    forge_foreign_checksums(&conn);
    migration::run(&mut conn).unwrap();

    // A table from the tail of the migration chain: proves the second run
    // carried the schema through rather than stopping at the first
    // disagreement and leaving a half-built database.
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'step_executions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "step_executions missing after the divergent run");

    // And the current tail of the chain, which is the half a divergent run can
    // actually stop short of.
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'sync_sessions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "sync_sessions missing after the divergent run");
}

#[test]
fn the_adapter_opens_against_a_divergent_history() {
    let mut conn = Connection::open_in_memory().unwrap();
    migration::run(&mut conn).unwrap();
    forge_foreign_checksums(&conn);

    // The full adapter path, not just the migration runner: this is what the
    // app actually does on launch.
    SqliteAdapter::new(conn).expect("adapter must open a database migrated by an older refinery");
}
