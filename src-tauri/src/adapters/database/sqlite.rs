use crate::domain::models::{
    AgentConfig, AgentProfile, ChatMessage, ChatSession, Machine, Message, SessionHistory,
    ThreadSession, WorkingMemoryEntry,
};
use crate::ports::db::DatabasePort;
use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct SqliteAdapter {
    pub conn: Mutex<Connection>,
}

impl SqliteAdapter {
    pub fn new(conn: Connection) -> Self {
        let _ = conn.execute("ALTER TABLE machines ADD COLUMN agents TEXT;", []);
        let _ = conn.execute("ALTER TABLE machines ADD COLUMN auto_approved_rules TEXT;", []);
        let _ = conn.execute("ALTER TABLE thread_sessions ADD COLUMN agent_kind TEXT;", []);

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS thread_sessions (
                id TEXT PRIMARY KEY,
                machine_id TEXT NOT NULL,
                title TEXT NOT NULL,
                mode TEXT NOT NULL,
                branch TEXT,
                repo_path TEXT,
                sandbox_path TEXT,
                status TEXT NOT NULL,
                agent_kind TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
            );",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS thread_working_memory (
                thread_id      TEXT NOT NULL,
                file_path      TEXT NOT NULL,
                line_count     INTEGER,
                size_bytes     INTEGER,
                modified_at    INTEGER,
                first_read_at  INTEGER NOT NULL,
                last_read_at   INTEGER NOT NULL,
                PRIMARY KEY (thread_id, file_path),
                FOREIGN KEY (thread_id) REFERENCES thread_sessions(id) ON DELETE CASCADE
            );",
            [],
        );

        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_twm_thread_last_read
             ON thread_working_memory(thread_id, last_read_at DESC);",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS app_session (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
            [],
        );

        let _ = conn.execute(
            "ALTER TABLE thread_sessions ADD COLUMN updated_at INTEGER;",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id         TEXT PRIMARY KEY,
                thread_id  TEXT NOT NULL,
                role       TEXT NOT NULL,
                content    TEXT NOT NULL DEFAULT '',
                metadata   TEXT,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (thread_id) REFERENCES thread_sessions(id) ON DELETE CASCADE
            );",
            [],
        );

        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_thread
             ON messages(thread_id, created_at ASC);",
            [],
        );

        // Drop the old thread_events table — we no longer use it.
        let _ = conn.execute("DROP TABLE IF EXISTS thread_events;", []);

        let adapter = Self { conn: Mutex::new(conn) };
        adapter.migrate_machine_agents();
        adapter
    }

    /// Migrate legacy `Machine.agents` JSON. Pre-v1 stored a bare string
    /// array (e.g. `["OpenCode", "Claude Code"]`); v1 wants structured
    /// `{kind, enabled}` records. Bare strings for known agent kinds
    /// (`opencode`, `hermes`) become `enabled: true`; everything else becomes
    /// `enabled: false` so the UI hides them but the user can re-enable once
    /// a real adapter exists.
    fn migrate_machine_agents(&self) {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return,
        };

        let machines: Vec<(String, Option<String>)> = match conn
            .prepare("SELECT id, agents FROM machines")
            .and_then(|mut s| {
                s.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))
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
                            // Bogus kind (e.g. the string "[object object]" left
                            // behind by an earlier round-trip of an object
                            // through a code path that Stringified it). Drop it.
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

            // Dedupe by kind (keep the first occurrence). Repeats from past
            // double-writes (e.g. the user re-saved the same form twice in
            // quick succession) should collapse to a single record.
            let mut seen_kinds: std::collections::HashSet<String> = std::collections::HashSet::new();
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
                params![machine_id, serialized],
            );
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
        let mut stmt = conn.prepare("SELECT id, machine_id, title, mode, branch, repo_path, sandbox_path, status, agent_kind, updated_at FROM thread_sessions WHERE machine_id = ?1 ORDER BY COALESCE(updated_at, CAST(strftime('%s', created_at) AS INTEGER) * 1000) DESC")
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
                agent_kind: row.get(8)?,
                updated_at: row.get(9)?,
            })
        }).map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for t in thread_iter {
            list.push(t.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn get_thread_sessions_for_thread(&self, thread_id: &str) -> Result<Vec<ThreadSession>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, machine_id, title, mode, branch, repo_path, sandbox_path, status, agent_kind, updated_at FROM thread_sessions WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        let thread_iter = stmt.query_map(params![thread_id], |row| {
            Ok(ThreadSession {
                id: row.get(0)?,
                machine_id: row.get(1)?,
                title: row.get(2)?,
                mode: row.get(3)?,
                branch: row.get(4)?,
                repo_path: row.get(5)?,
                sandbox_path: row.get(6)?,
                status: row.get(7)?,
                agent_kind: row.get(8)?,
                updated_at: row.get(9)?,
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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        conn.execute(
            "INSERT INTO thread_sessions (id, machine_id, title, mode, branch, repo_path, sandbox_path, status, agent_kind, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![t.id, t.machine_id, t.title, t.mode, t.branch, t.repo_path, t.sandbox_path, t.status, t.agent_kind, now],
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


    fn get_agent_configs(&self, machine_id: &str) -> Result<Vec<AgentConfig>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let raw: Option<String> = conn
            .query_row(
                "SELECT agents FROM machines WHERE id = ?1",
                params![machine_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let parsed: Vec<AgentConfig> = raw
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        Ok(parsed)
    }

    fn set_agent_configs(&self, machine_id: &str, agents_json: &str) -> Result<(), String> {
        // Validate it's a parseable array before writing.
        let _: Vec<AgentConfig> = serde_json::from_str(agents_json)
            .map_err(|e| format!("Invalid agents JSON: {}", e))?;
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "UPDATE machines SET agents = ?2 WHERE id = ?1",
            params![machine_id, agents_json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn upsert_working_memory_entry(
        &self,
        thread_id: &str,
        entry: WorkingMemoryEntry,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        // UPSERT: if a row exists, preserve the original first_read_at and
        // refresh last_read_at + metadata. If not, insert.
        conn.execute(
            "INSERT INTO thread_working_memory
                (thread_id, file_path, line_count, size_bytes, modified_at, first_read_at, last_read_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(thread_id, file_path) DO UPDATE SET
                line_count  = excluded.line_count,
                size_bytes  = excluded.size_bytes,
                modified_at = excluded.modified_at,
                last_read_at = excluded.last_read_at",
            params![
                thread_id,
                entry.file_path,
                entry.line_count.map(|n| n as i64),
                entry.size_bytes.map(|n| n as i64),
                entry.modified_at,
                entry.first_read_at,
                entry.last_read_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_working_memory(&self, thread_id: &str) -> Result<Vec<WorkingMemoryEntry>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT file_path, line_count, size_bytes, modified_at, first_read_at, last_read_at
                 FROM thread_working_memory WHERE thread_id = ?1
                 ORDER BY last_read_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![thread_id], |row| {
                Ok(WorkingMemoryEntry {
                    file_path: row.get(0)?,
                    line_count: row.get::<_, Option<i64>>(1)?.map(|n| n as u32),
                    size_bytes: row.get::<_, Option<i64>>(2)?.map(|n| n as u64),
                    modified_at: row.get(3)?,
                    first_read_at: row.get(4)?,
                    last_read_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn clear_working_memory(&self, thread_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "DELETE FROM thread_working_memory WHERE thread_id = ?1",
            params![thread_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_app_session(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let result: Option<String> = conn
            .query_row(
                "SELECT value FROM app_session WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .ok();
        Ok(result)
    }

    fn set_app_session(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO app_session (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_app_session(&self, key: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute("DELETE FROM app_session WHERE key = ?1", params![key])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_messages(&self, thread_id: &str) -> Result<Vec<Message>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, thread_id, role, content, metadata, created_at FROM messages WHERE thread_id = ?1 ORDER BY created_at ASC")
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![thread_id], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    thread_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    metadata: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn append_message(&self, msg: &Message) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO messages (id, thread_id, role, content, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![msg.id, msg.thread_id, msg.role, msg.content, msg.metadata, msg.created_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_messages(&self, thread_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute("DELETE FROM messages WHERE thread_id = ?1", params![thread_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_thread_timestamp(&self, id: &str) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "UPDATE thread_sessions SET updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
