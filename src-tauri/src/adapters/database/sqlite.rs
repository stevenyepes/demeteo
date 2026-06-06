use crate::ports::db::DatabasePort;
use crate::domain::models::{Machine, AgentProfile, ChatSession, ChatMessage, SessionHistory, ThreadSession};
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct SqliteAdapter {
    pub conn: Mutex<Connection>,
}

impl SqliteAdapter {
    pub fn new(conn: Connection) -> Self {
        // Run migration checks on initialization
        let _ = conn.execute("ALTER TABLE machines ADD COLUMN agents TEXT;", []);
        let _ = conn.execute("ALTER TABLE machines ADD COLUMN auto_approved_rules TEXT;", []);
        
        let _ = conn.execute("CREATE TABLE IF NOT EXISTS thread_sessions (
            id TEXT PRIMARY KEY,
            machine_id TEXT NOT NULL,
            title TEXT NOT NULL,
            mode TEXT NOT NULL,
            branch TEXT,
            repo_path TEXT,
            sandbox_path TEXT,
            status TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
        );", []);
        
        Self {
            conn: Mutex::new(conn),
        }
    }
}

impl DatabasePort for SqliteAdapter {
    fn get_machines(&self) -> Result<Vec<Machine>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, name, host, port, username, auth_type, key_path, agents, auto_approved_rules FROM machines ORDER BY created_at DESC")
            .map_err(|e| e.to_string())?;
        let machine_iter = stmt.query_map([], |row| {
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
            })
        }).map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for machine in machine_iter {
            list.push(machine.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn add_machine(&self, m: Machine) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO machines (id, name, host, port, username, auth_type, key_path, agents, auto_approved_rules)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![m.id, m.name, m.host, m.port, m.username, m.auth_type, m.key_path, m.agents, m.auto_approved_rules],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_machine(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute("DELETE FROM machines WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_machine(&self, m: Machine) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "UPDATE machines 
             SET name = ?2, host = ?3, port = ?4, username = ?5, auth_type = ?6, key_path = ?7, agents = ?8, auto_approved_rules = ?9 
             WHERE id = ?1",
            params![m.id, m.name, m.host, m.port, m.username, m.auth_type, m.key_path, m.agents, m.auto_approved_rules],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_agent_profiles(&self, machine_id: &str) -> Result<Vec<AgentProfile>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, machine_id, name, agent_type, command, work_dir, port, ready_check FROM agent_profiles WHERE machine_id = ?1")
            .map_err(|e| e.to_string())?;
        let agent_iter = stmt.query_map(params![machine_id], |row| {
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
        }).map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for agent in agent_iter {
            list.push(agent.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn add_agent_profile(&self, a: AgentProfile) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO agent_profiles (id, machine_id, name, agent_type, command, work_dir, port, ready_check)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![a.id, a.machine_id, a.name, a.agent_type, a.command, a.work_dir, a.port, a.ready_check],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_agent_profile(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute("DELETE FROM agent_profiles WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn create_chat_session(&self, id: &str, agent_id: &str, title: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO chat_sessions (id, agent_id, title) VALUES (?1, ?2, ?3)",
            params![id, agent_id, title],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_chat_sessions(&self, agent_id: &str) -> Result<Vec<ChatSession>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, agent_id, title, created_at FROM chat_sessions WHERE agent_id = ?1 ORDER BY created_at DESC")
            .map_err(|e| e.to_string())?;
        let session_iter = stmt.query_map(params![agent_id], |row| {
            Ok(ChatSession {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                title: row.get(2)?,
                created_at: row.get(3)?,
            })
        }).map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for s in session_iter {
            list.push(s.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn add_chat_message(&self, id: &str, session_id: &str, sender: &str, content: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO chat_messages (id, session_id, sender, content) VALUES (?1, ?2, ?3, ?4)",
            params![id, session_id, sender, content],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_chat_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, session_id, sender, content, timestamp FROM chat_messages WHERE session_id = ?1 ORDER BY timestamp ASC")
            .map_err(|e| e.to_string())?;
        let msg_iter = stmt.query_map(params![session_id], |row| {
            Ok(ChatMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                sender: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
            })
        }).map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for m in msg_iter {
            list.push(m.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn add_session_history(&self, id: &str, machine_id: &str, session_type: &str, title: &str, content: Option<&str>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO session_history (id, machine_id, session_type, title, content) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, machine_id, session_type, title, content],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_session_history(&self, machine_id: &str) -> Result<Vec<SessionHistory>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, machine_id, session_type, title, content, created_at FROM session_history WHERE machine_id = ?1 ORDER BY created_at DESC")
            .map_err(|e| e.to_string())?;
        let history_iter = stmt.query_map(params![machine_id], |row| {
            Ok(SessionHistory {
                id: row.get(0)?,
                machine_id: row.get(1)?,
                session_type: row.get(2)?,
                title: row.get(3)?,
                content: row.get(4)?,
                created_at: row.get(5)?,
            })
        }).map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for h in history_iter {
            list.push(h.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn get_thread_sessions(&self, machine_id: &str) -> Result<Vec<ThreadSession>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, machine_id, title, mode, branch, repo_path, sandbox_path, status FROM thread_sessions WHERE machine_id = ?1 ORDER BY created_at DESC")
            .map_err(|e| e.to_string())?;
        let thread_iter = stmt.query_map(params![machine_id], |row| {
            Ok(ThreadSession {
                id: row.get(0)?,
                machine_id: row.get(1)?,
                title: row.get(2)?,
                mode: row.get(3)?,
                branch: row.get(4)?,
                repo_path: row.get(5)?,
                sandbox_path: row.get(6)?,
                status: row.get(7)?,
            })
        }).map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for t in thread_iter {
            list.push(t.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn add_thread_session(&self, t: ThreadSession) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO thread_sessions (id, machine_id, title, mode, branch, repo_path, sandbox_path, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![t.id, t.machine_id, t.title, t.mode, t.branch, t.repo_path, t.sandbox_path, t.status],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_thread_status(&self, id: &str, status: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "UPDATE thread_sessions SET status = ?2 WHERE id = ?1",
            params![id, status],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_thread_session(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute("DELETE FROM thread_sessions WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }
}
