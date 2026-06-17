pub mod connection;
pub mod error;
pub mod migration;
pub mod repos;

#[cfg(test)]
pub mod tests;

use connection::SqliteConnection;

use rusqlite::Connection;

use crate::domain::models::AgentConfig;

pub struct SqliteAdapter {
    pub conn: SqliteConnection,
}

impl SqliteAdapter {
    pub fn new(mut conn: Connection) -> Result<Self, String> {
        migration::run(&mut conn).map_err(|e| e.to_string())?;

        let _ = conn.execute_batch("DROP TABLE IF EXISTS thread_events;");

        let adapter = Self {
            conn: SqliteConnection::new(conn),
        };
        adapter.migrate_machine_agents();
        Ok(adapter)
    }

    fn migrate_machine_agents(&self) {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return,
        };

        let machines: Vec<(String, Option<String>)> = match conn
            .prepare("SELECT id, agents FROM machines")
            .and_then(|mut s| {
                s.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .map(|iter| iter.filter_map(|r| r.ok()).collect())
            }) {
            Ok(v) => v,
            Err(_) => return,
        };

        for (machine_id, raw) in machines {
            let parsed: Vec<serde_json::Value> = match raw.as_deref() {
                Some(s) if !s.trim().is_empty() => match serde_json::from_str(s) {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                _ => continue,
            };

            let migrated: Vec<AgentConfig> = parsed
                .into_iter()
                .filter_map(|v| {
                    if let Some(s) = v.as_str() {
                        let kind = s.to_lowercase();
                        if matches!(kind.as_str(), "opencode" | "hermes") {
                            Some(AgentConfig { kind, enabled: true })
                        } else {
                            None
                        }
                    } else if let Some(obj) = v.as_object() {
                        let raw_kind = obj
                            .get("kind")
                            .and_then(|k| k.as_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if !matches!(raw_kind.as_str(), "opencode" | "hermes") {
                            return None;
                        }
                        Some(AgentConfig {
                            kind: raw_kind,
                            enabled: obj
                                .get("enabled")
                                .and_then(|e| e.as_bool())
                                .unwrap_or(false),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            let mut seen_kinds = std::collections::HashSet::new();
            let migrated: Vec<AgentConfig> = migrated
                .into_iter()
                .filter(|c| seen_kinds.insert(c.kind.clone()))
                .collect();

            let serialized = match serde_json::to_string(&migrated) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let _ = conn.execute(
                "UPDATE machines SET agents = ?2 WHERE id = ?1",
                rusqlite::params![machine_id, serialized],
            );
        }
    }
}
