//! Integration coverage for the tray / background-notification task
//! (spec `artifacts/_context/implementation-spec.md` §5.2, AC-2).
//!
//! ## What this pins
//!
//! The `run_in_background` **preference round-trip** exercised end-to-end
//! against a real `AppSettingsRepository` (the SQLite adapter) backed by a
//! temp data dir, driving the **actual command core** —
//! `commands::app_session::{write_run_in_background, read_run_in_background}` —
//! not a re-implementation of it:
//!
//!   * `set true`  → the stored value reads back as `"true"`  → read `true`.
//!   * absent key  → `get_app_session` returns `None`         → read `false`.
//!   * `set false` → the stored value reads back as `"false"` → read `false`.
//!
//! The `#[tauri::command]` wrappers `get_run_in_background` /
//! `set_run_in_background` take `State<'_, AppContext>`, whose ~25-port
//! constructor is Tauri-internal and cannot be built from an external
//! integration test (see the same note in `create_project_orchestration.rs`).
//! They are therefore thin delegators to `read_run_in_background` /
//! `write_run_in_background`, which take the settings port directly. This test
//! calls those exact functions against a real store, so the key constant and
//! the `"true"`/`"false"` encode + `matches!(value, Some("true"))` decode are
//! covered by the code the commands run — a drift (e.g. switching to `"1"`/`"0"`)
//! resurfaces here rather than passing against a duplicated copy.
//!
//! The OS-notification **routing decision** (`os_notification_for`) is a private
//! helper of `adapters::tauri_ui::notification`, unit-tested in-module there
//! (the `#[cfg(test)]` block covering the Some/None cases of spec §5.1). The
//! window-visibility/focus gate that wraps it (spec AC-6 / Constraint 3) needs a
//! live Tauri window and is verified manually — see the checklist below.
//!
//! ## Manual verification (spec §5, "Manual happy-path + edge cases")
//!
//! * Happy path: enable the toggle → close the window → the process stays alive
//!   and the window hides → tray "Show" restores it → tray "Quit" exits cleanly.
//! * Edge: toggle OFF → closing the window behaves exactly as today
//!   (session cleanup + process exit).
//! * Edge: window hidden, a feature completes → exactly one OS notification
//!   appears; window visible **and** focused → none.
//! * Edge (Linux): tray backend unavailable → the app still starts, a warning is
//!   logged, and close behaves as if the preference were OFF.

use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::ports::db::AppSettingsRepository;
// The production command core — the same functions the `#[tauri::command]`
// wrappers delegate to. Testing these (not a local mirror) is what gives this
// file real protection over the command bodies.
use demeteo_lib::commands::app_session::{read_run_in_background, write_run_in_background};

const RUN_IN_BACKGROUND_KEY: &str = "run_in_background";

/// Build a real SQLite-backed `AppSettingsRepository` on an isolated temp data
/// dir, mirroring the on-disk store the app's `AppContext.app_settings` uses.
fn temp_app_settings() -> (SqliteAdapter, std::path::PathBuf) {
    // A per-test unique dir under the OS temp root. `Date::now`/rand are not
    // needed (and are unavailable in some harnesses) — the pid plus an atomic
    // counter is enough to keep parallel test threads from colliding.
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "demeteo-tray-notif-test-{}-{}",
        std::process::id(),
        n
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let conn = demeteo_core::db::init_db(dir.clone()).expect("init temp db");
    // `init_db` opens the file + PRAGMAs; `SqliteAdapter::new` runs migrations
    // so the `app_settings` KV table exists.
    let adapter = SqliteAdapter::new(conn).expect("build sqlite adapter");
    (adapter, dir)
}

#[test]
fn run_in_background_absent_key_defaults_to_false() {
    let (store, dir) = temp_app_settings();
    // Never written → background mode is opt-in (spec AC-2 / Constraint 5).
    assert!(!read_run_in_background(&store).expect("read"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn run_in_background_set_true_round_trips_to_true() {
    let (store, dir) = temp_app_settings();
    write_run_in_background(&store, true).expect("write true");
    // The exact on-disk encoding the command produces, asserted against the
    // raw store so a future re-encode (e.g. to `"1"`) is caught here.
    assert_eq!(
        store.get_app_session(RUN_IN_BACKGROUND_KEY).unwrap().as_deref(),
        Some("true"),
        "write_run_in_background(true) must persist the string \"true\""
    );
    assert!(read_run_in_background(&store).expect("read"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn run_in_background_set_false_round_trips_to_false() {
    let (store, dir) = temp_app_settings();
    // Turn it on, then back off — the OFF value must round-trip, not linger ON.
    write_run_in_background(&store, true).expect("write true");
    write_run_in_background(&store, false).expect("write false");
    assert_eq!(
        store.get_app_session(RUN_IN_BACKGROUND_KEY).unwrap().as_deref(),
        Some("false"),
        "write_run_in_background(false) must persist the string \"false\""
    );
    assert!(!read_run_in_background(&store).expect("read"));
    let _ = std::fs::remove_dir_all(dir);
}
