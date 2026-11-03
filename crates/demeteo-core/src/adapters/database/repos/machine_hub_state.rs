//! SQL for `machine_hub_state` (V69).

use std::collections::HashMap;

use rusqlite::{params, OptionalExtension};

use crate::domain::hub_socket::RunnerHubState;
use crate::ports::db::MachineHubStateRepository;

use super::super::SqliteAdapter;

fn row_to_state(row: &rusqlite::Row) -> rusqlite::Result<(String, RunnerHubState)> {
    Ok((
        row.get(0)?,
        RunnerHubState {
            paired: row.get(1)?,
            hub_url: row.get(2)?,
            instance_id: row.get(3)?,
            checked_at: row.get(4)?,
        },
    ))
}

const SELECT: &str =
    "SELECT machine_id, paired, hub_url, instance_id, checked_at FROM machine_hub_state";

impl MachineHubStateRepository for SqliteAdapter {
    fn get(&self, machine_id: &str) -> Result<Option<RunnerHubState>, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!("{SELECT} WHERE machine_id = ?1"),
            params![machine_id],
            row_to_state,
        )
        .optional()
        .map(|row| row.map(|(_, state)| state))
        .map_err(|e| e.to_string())
    }

    fn put(&self, machine_id: &str, state: &RunnerHubState) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO machine_hub_state (machine_id, paired, hub_url, instance_id, checked_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (machine_id) DO UPDATE SET paired = excluded.paired,
                                                    hub_url = excluded.hub_url,
                                                    instance_id = excluded.instance_id,
                                                    checked_at = excluded.checked_at",
            params![
                machine_id,
                state.paired,
                state.hub_url,
                state.instance_id,
                state.checked_at
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn all(&self) -> Result<HashMap<String, RunnerHubState>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn.prepare(SELECT).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_state)
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<HashMap<_, _>>>()
            .map_err(|e| e.to_string());
        rows
    }
}
