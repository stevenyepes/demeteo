// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/thread.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::models::AgentConfig;
use rusqlite::Connection;

fn db() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

/// A directory removed when the binding drops, so a failing assertion below
/// leaks nothing. `std::fs::remove_dir_all` on the last line of a test only
/// runs when the test passes, which is the case that needed it least.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "demeteo-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn agents_json(entries: &[(&str, bool)]) -> String {
    let configs: Vec<AgentConfig> = entries
        .iter()
        .map(|(kind, enabled)| AgentConfig {
            kind: kind.to_string(),
            enabled: *enabled,
        })
        .collect();
    serde_json::to_string(&configs).unwrap()
}

/// "Disable an agent in Project Settings, click Save, and it doesn't persist"
/// from the bug report, at the layer that dropped the write.
///
/// The local host's list is not a `machines` row and cannot be one — see the
/// header of `migrations/V38__local_agent_config.sql` for why.
#[test]
fn disabling_an_agent_and_saving_persists_for_the_local_machine() {
    let db = db();
    let machine_id = MachineId::from("local".to_string());
    let json = agents_json(&[("opencode", true), ("claude-code", false)]);

    ThreadRepository::set_agent_configs(&db, &machine_id, &json)
        .expect("saving the local machine's agent list must succeed");

    let persisted = ThreadRepository::get_agent_configs(&db, &machine_id)
        .expect("the config just saved for the local machine must be readable back");

    let claude_code = persisted
        .iter()
        .find(|c| c.kind == "claude-code")
        .expect("claude-code must still be present in the persisted config");
    assert!(
        !claude_code.enabled,
        "the disabled state the user just saved must survive the round trip"
    );
}

/// The empty machine id is the other spelling of "this host" — several ports
/// are reached with it by callers that never had a machine to name. It must
/// land in the same place `"local"` does, or the toggle silently saves to a
/// `machines` row that does not exist.
#[test]
fn the_empty_machine_id_reads_and_writes_the_same_local_config() {
    let db = db();
    let json = agents_json(&[("codex", false)]);

    ThreadRepository::set_agent_configs(&db, &MachineId::from(String::new()), &json)
        .expect("an empty machine id must be accepted as the local host");

    let persisted =
        ThreadRepository::get_agent_configs(&db, &MachineId::from("local".to_string())).unwrap();
    let codex = persisted
        .iter()
        .find(|c| c.kind == "codex")
        .expect("what the empty id wrote must be what `local` reads");
    assert!(!codex.enabled);
}

/// The `machines` half of the fork still works. A remote machine *does* have a
/// row, so its list stays there — V38 is for the host that has no row, not a
/// replacement for per-machine storage.
#[test]
fn a_remote_machine_round_trips_through_its_own_machines_row() {
    let db = db();
    let machine_id = MachineId::from("m-remote".to_string());
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO machines (id, name, host, port, username, auth_type)
             VALUES (?1, 'build box', 'build.example', 22, 'ci', 'key')",
            rusqlite::params![machine_id.0],
        )
        .unwrap();
    }

    ThreadRepository::set_agent_configs(&db, &machine_id, &agents_json(&[("hermes", false)]))
        .unwrap();

    let persisted = ThreadRepository::get_agent_configs(&db, &machine_id).unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].kind, "hermes");
    assert!(!persisted[0].enabled);

    let local = ThreadRepository::get_agent_configs(&db, &MachineId::from("local".to_string()))
        .expect("the local list is a separate store, not this machine's");
    assert!(
        local.is_empty(),
        "a remote machine's list must not leak into the local one"
    );
}

/// A machine with no stored list yet is an empty list, not an error — the
/// caller cannot tell a first run from a failed read otherwise.
#[test]
fn a_machine_with_no_stored_config_reads_as_empty() {
    let db = db();
    let persisted = ThreadRepository::get_agent_configs(&db, &MachineId::from("m-unknown"))
        .expect("an unknown machine is an empty config, not a read failure");
    assert!(persisted.is_empty());
}

/// `migrate_local_machines` runs `DELETE FROM machines WHERE auth_type =
/// 'local'` on every `SqliteAdapter::new` — a config stored under the
/// `"local"` id must survive that call, not just an in-process round trip,
/// or reopening the app (a fresh adapter over the same file) would still
/// reset the toggles.
#[test]
fn local_agent_config_survives_a_fresh_sqlite_adapter() {
    let tmp = TempDir::new("local-agent-config");
    let path = tmp.0.join("db.sqlite");
    let machine_id = MachineId::from("local".to_string());

    {
        let db = SqliteAdapter::new(Connection::open(&path).unwrap()).unwrap();
        let json = agents_json(&[("claude-code", false)]);
        ThreadRepository::set_agent_configs(&db, &machine_id, &json).unwrap();
    }
    {
        let db = SqliteAdapter::new(Connection::open(&path).unwrap()).unwrap();
        let persisted = ThreadRepository::get_agent_configs(&db, &machine_id).unwrap();
        let claude_code = persisted
            .iter()
            .find(|c| c.kind == "claude-code")
            .expect("claude-code must survive a fresh adapter over the same file");
        assert!(
            !claude_code.enabled,
            "migrate_local_machines must not reset the persisted local config"
        );
    }
}
