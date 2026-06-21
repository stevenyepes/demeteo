use rusqlite::params;

use crate::domain::ids::{AgentProfileId, MachineId};
use crate::domain::models::{AgentProfile, Machine};
use crate::ports::db::MachineRepository;

use super::super::SqliteAdapter;

impl MachineRepository for SqliteAdapter {
    fn get_machines(&self) -> Result<Vec<Machine>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, host, port, username, auth_type, key_path,
                        agents, auto_approved_rules, use_login_shell, setup_commands
                 FROM machines ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map([], |row| {
                Ok(Machine {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    host: row.get(2)?,
                    port: row.get(3)?,
                    username: row.get(4)?,
                    auth_type: row.get(5)?,
                    key_path: row.get(6)?,
                    agents: row.get(7)?,
                    auto_approved_rules: row.get(8)?,
                    use_login_shell: row.get(9)?,
                    setup_commands: row.get(10)?,
                })
            })
            .map_err(|e| e.to_string())?;
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn get_machine(&self, id: &MachineId) -> Result<Option<Machine>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, host, port, username, auth_type, key_path,
                        agents, auto_approved_rules, use_login_shell, setup_commands
                 FROM machines WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
                Ok(Machine {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    host: row.get(2)?,
                    port: row.get(3)?,
                    username: row.get(4)?,
                    auth_type: row.get(5)?,
                    key_path: row.get(6)?,
                    agents: row.get(7)?,
                    auto_approved_rules: row.get(8)?,
                    use_login_shell: row.get(9)?,
                    setup_commands: row.get(10)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(m)) => Ok(Some(m)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn add(&self, m: Machine) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO machines (id, name, host, port, username, auth_type, key_path, agents, auto_approved_rules, use_login_shell, setup_commands)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![m.id, m.name, m.host, m.port, m.username, m.auth_type, m.key_path, m.agents, m.auto_approved_rules, m.use_login_shell, m.setup_commands],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update(&self, m: Machine) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE machines
             SET name = ?2, host = ?3, port = ?4, username = ?5,
                 auth_type = ?6, key_path = ?7, agents = ?8, auto_approved_rules = ?9,
                 use_login_shell = ?10, setup_commands = ?11
             WHERE id = ?1",
            params![
                m.id,
                m.name,
                m.host,
                m.port,
                m.username,
                m.auth_type,
                m.key_path,
                m.agents,
                m.auto_approved_rules,
                m.use_login_shell,
                m.setup_commands
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete(&self, id: &MachineId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM machines WHERE id = ?1", params![id.0])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_agent_profiles(&self, machine_id: &MachineId) -> Result<Vec<AgentProfile>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, machine_id, name, agent_type, command, work_dir, port, ready_check
                 FROM agent_profiles WHERE machine_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![machine_id.0], |row| {
                Ok(AgentProfile {
                    id: row.get(0)?,
                    machine_id: row.get(1)?,
                    name: row.get(2)?,
                    agent_type: row.get(3)?,
                    command: row.get(4)?,
                    work_dir: row.get(5)?,
                    port: row.get(6)?,
                    ready_check: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn add_agent_profile(&self, a: AgentProfile) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO agent_profiles (id, machine_id, name, agent_type, command, work_dir, port, ready_check)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![a.id, a.machine_id, a.name, a.agent_type, a.command, a.work_dir, a.port, a.ready_check],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_agent_profile(&self, id: &AgentProfileId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM agent_profiles WHERE id = ?1", params![id.0])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
