// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/thread.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::models::AgentConfig;
use rusqlite::Connection;

fn db() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

/// `"local"` is the built-in sentinel `machine_resolver::resolve_machine`
/// hands back for a local-compute project — `migrate_local_machines` runs on
/// every `SqliteAdapter::new` and deletes any `machines` row that could ever
/// match it, so no row with `id = 'local'` can exist by the time a command
/// reaches this repo. `get_agent_configs`/`set_agent_configs` therefore
/// route the `"local"` id to the `local_agent_config` singleton table (V38)
/// instead of `machines`, which `migrate_local_machines` never touches.
///
/// This is "disable an agent in Project Settings, click Save, and it
/// doesn't persist" from the bug report.
#[test]
fn disabling_an_agent_and_saving_persists_for_the_local_machine() {
    let db = db();
    let machine_id = MachineId::from("local".to_string());

    let agents = vec![
        AgentConfig {
            kind: "opencode".to_string(),
            enabled: true,
        },
        AgentConfig {
            kind: "claude-code".to_string(),
            enabled: false,
        },
    ];
    let json = serde_json::to_string(&agents).unwrap();

    ThreadRepository::set_agent_configs(&db, &machine_id, &json)
        .expect("set_agent_configs should not error even though it silently no-ops");

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

/// `migrate_local_machines` runs `DELETE FROM machines WHERE auth_type =
/// 'local'` on every `SqliteAdapter::new` — a config stored under the
/// `"local"` id must survive that call, not just an in-process round trip,
/// or reopening the app (a fresh adapter over the same file) would still
/// reset the toggles to the hardcoded seed.
#[test]
fn local_agent_config_survives_a_fresh_sqlite_adapter() {
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-local-agent-config-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("db.sqlite");
    let machine_id = MachineId::from("local".to_string());

    {
        let db = SqliteAdapter::new(Connection::open(&path).unwrap()).unwrap();
        let agents = vec![AgentConfig {
            kind: "claude-code".to_string(),
            enabled: false,
        }];
        let json = serde_json::to_string(&agents).unwrap();
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
    let _ = std::fs::remove_dir_all(&tmp);
}
