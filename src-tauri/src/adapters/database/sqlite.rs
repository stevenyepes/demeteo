use crate::domain::models::{
    AgentConfig, AgentProfile, ChatMessage, ChatSession, Machine, Message, SessionHistory,
    ThreadSession, WorkingMemoryEntry, ProviderInstance, Project, Repository, Feature,
    ProjectSettings, WorktreeStrategy, Workflow, WorkflowVersion, StepExecution, GateDecision,
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
                model TEXT,
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
            "ALTER TABLE thread_sessions ADD COLUMN model TEXT;",
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

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS provider_instances (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                host TEXT NOT NULL,
                username TEXT NOT NULL,
                avatar_url TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                compute_type TEXT NOT NULL,
                remote_host TEXT,
                status TEXT NOT NULL,
                nodes INTEGER NOT NULL,
                spend REAL NOT NULL,
                created_at INTEGER NOT NULL
            );",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS repositories (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );",
            [],
        );

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS features (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                workflow_id TEXT,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                total_cost REAL NOT NULL,
                duration TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );",
            [],
        );

        let _ = conn.execute(
            "ALTER TABLE features ADD COLUMN workflow_id TEXT;",
            [],
        );

        let _ = conn.execute("ALTER TABLE project_settings ADD COLUMN build_command TEXT;", []);
        let _ = conn.execute("ALTER TABLE project_settings ADD COLUMN coverage_command TEXT;", []);
        let _ = conn.execute("ALTER TABLE project_settings ADD COLUMN conventions_file TEXT;", []);

        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS project_settings (
                project_id TEXT PRIMARY KEY,
                default_branch TEXT NOT NULL,
                branch_prefix TEXT NOT NULL,
                test_command TEXT,
                build_command TEXT,
                coverage_command TEXT,
                conventions_file TEXT,
                pr_template TEXT,
                conflict_policy TEXT NOT NULL,
                feature_lifecycle TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
            );",
            [],
        );

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
        let mut stmt = conn.prepare("SELECT id, machine_id, title, mode, branch, repo_path, sandbox_path, status, agent_kind, model, updated_at FROM thread_sessions WHERE machine_id = ?1 ORDER BY COALESCE(updated_at, CAST(strftime('%s', created_at) AS INTEGER) * 1000) DESC")
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
                model: row.get(9)?,
                updated_at: row.get(10)?,
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
        let mut stmt = conn.prepare("SELECT id, machine_id, title, mode, branch, repo_path, sandbox_path, status, agent_kind, model, updated_at FROM thread_sessions WHERE id = ?1")
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
                model: row.get(9)?,
                updated_at: row.get(10)?,
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
            "INSERT INTO thread_sessions (id, machine_id, title, mode, branch, repo_path, sandbox_path, status, agent_kind, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![t.id, t.machine_id, t.title, t.mode, t.branch, t.repo_path, t.sandbox_path, t.status, t.agent_kind, t.model, now],
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

    fn update_thread_model(&self, id: &str, model: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "UPDATE thread_sessions SET model = ?2 WHERE id = ?1",
            params![id, model],
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

    fn add_provider_instance(&self, provider: ProviderInstance) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO provider_instances (id, kind, host, username, avatar_url, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![provider.id, provider.kind, provider.host, provider.username, provider.avatar_url, provider.created_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_provider_instances(&self) -> Result<Vec<ProviderInstance>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, kind, host, username, avatar_url, created_at FROM provider_instances ORDER BY created_at DESC").map_err(|e| e.to_string())?;
        let iter = stmt.query_map([], |row| {
            Ok(ProviderInstance {
                id: row.get(0)?,
                kind: row.get(1)?,
                host: row.get(2)?,
                username: row.get(3)?,
                avatar_url: row.get(4)?,
                created_at: row.get(5)?,
            })
        }).map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter { list.push(r.map_err(|e| e.to_string())?); }
        Ok(list)
    }

    fn delete_provider_instance(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute("DELETE FROM provider_instances WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn add_project(&self, project: Project) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO projects (id, name, compute_type, remote_host, status, nodes, spend, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![project.id, project.name, project.compute_type, project.remote_host, project.status, project.nodes, project.spend, project.created_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_projects(&self) -> Result<Vec<Project>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, name, compute_type, remote_host, status, nodes, spend, created_at FROM projects ORDER BY created_at DESC").map_err(|e| e.to_string())?;
        let iter = stmt.query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                compute_type: row.get(2)?,
                remote_host: row.get(3)?,
                status: row.get(4)?,
                nodes: row.get(5)?,
                spend: row.get(6)?,
                created_at: row.get(7)?,
            })
        }).map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter { list.push(r.map_err(|e| e.to_string())?); }
        Ok(list)
    }

    fn update_project_status(&self, id: &str, status: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "UPDATE projects SET status = ?2 WHERE id = ?1",
            params![id, status],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn add_repository(&self, repo: Repository) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO repositories (id, project_id, provider_id, repo_path) VALUES (?1, ?2, ?3, ?4)",
            params![repo.id, repo.project_id, repo.provider_id, repo.repo_path],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_repositories_for_project(&self, project_id: &str) -> Result<Vec<Repository>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, project_id, provider_id, repo_path FROM repositories WHERE project_id = ?1").map_err(|e| e.to_string())?;
        let iter = stmt.query_map(params![project_id], |row| {
            Ok(Repository {
                id: row.get(0)?,
                project_id: row.get(1)?,
                provider_id: row.get(2)?,
                repo_path: row.get(3)?,
            })
        }).map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter { list.push(r.map_err(|e| e.to_string())?); }
        Ok(list)
    }

    fn add_feature(&self, feature: Feature) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT INTO features (id, project_id, workflow_id, title, status, total_cost, duration, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![feature.id, feature.project_id, feature.workflow_id, feature.title, feature.status, feature.total_cost, feature.duration, feature.created_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_active_features(&self, project_id: &str) -> Result<Vec<Feature>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, project_id, workflow_id, title, status, total_cost, duration, created_at FROM features WHERE project_id = ?1 ORDER BY created_at DESC").map_err(|e| e.to_string())?;
        let iter = stmt.query_map(params![project_id], |row| {
            Ok(Feature {
                id: row.get(0)?,
                project_id: row.get(1)?,
                workflow_id: row.get(2)?,
                title: row.get(3)?,
                status: row.get(4)?,
                total_cost: row.get(5)?,
                duration: row.get(6)?,
                created_at: row.get(7)?,
            })
        }).map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter { list.push(r.map_err(|e| e.to_string())?); }
        Ok(list)
    }

    fn get_feature(&self, id: &str) -> Result<Option<Feature>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare("SELECT id, project_id, workflow_id, title, status, total_cost, duration, created_at FROM features WHERE id = ?1").map_err(|e| e.to_string())?;
        let mut iter = stmt.query_map(params![id], |row| {
            Ok(Feature {
                id: row.get(0)?,
                project_id: row.get(1)?,
                workflow_id: row.get(2)?,
                title: row.get(3)?,
                status: row.get(4)?,
                total_cost: row.get(5)?,
                duration: row.get(6)?,
                created_at: row.get(7)?,
            })
        }).map_err(|e| e.to_string())?;
        if let Some(r) = iter.next() { Ok(Some(r.map_err(|e| e.to_string())?)) } else { Ok(None) }
    }

    fn feature_update_workflow_id(&self, id: &str, workflow_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "UPDATE features SET workflow_id = ?2 WHERE id = ?1",
            params![id, workflow_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_project_settings(&self, project_id: &str) -> Result<Option<ProjectSettings>, String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        let mut stmt = conn.prepare(
            "SELECT project_id, default_branch, branch_prefix, test_command, pr_template, conflict_policy, feature_lifecycle, build_command, coverage_command, conventions_file
             FROM project_settings WHERE project_id = ?1"
        ).map_err(|e| e.to_string())?;

        let mut iter = stmt.query_map(params![project_id], |row| {
            Ok(ProjectSettings {
                project_id: row.get(0)?,
                worktree_strategy: WorktreeStrategy {
                    default_branch: row.get(1)?,
                    branch_prefix: row.get(2)?,
                    test_command: row.get(3)?,
                    build_command: row.get(7)?,
                    coverage_command: row.get(8)?,
                    conventions_file: row.get(9)?,
                    pr_template: row.get(4)?,
                },
                conflict_policy: row.get(5)?,
                feature_lifecycle: row.get(6)?,
            })
        }).map_err(|e| e.to_string())?;

        if let Some(res) = iter.next() {
            Ok(Some(res.map_err(|e| e.to_string())?))
        } else {
            Ok(None)
        }
    }

    fn save_project_settings(&self, s: ProjectSettings) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO project_settings
             (project_id, default_branch, branch_prefix, test_command, build_command, coverage_command, conventions_file, pr_template, conflict_policy, feature_lifecycle)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                s.project_id,
                s.worktree_strategy.default_branch,
                s.worktree_strategy.branch_prefix,
                s.worktree_strategy.test_command,
                s.worktree_strategy.build_command,
                s.worktree_strategy.coverage_command,
                s.worktree_strategy.conventions_file,
                s.worktree_strategy.pr_template,
                s.conflict_policy,
                s.feature_lifecycle
            ],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_project(&self, project: Project) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute(
            "UPDATE projects SET name = ?2, compute_type = ?3, remote_host = ?4, status = ?5, nodes = ?6 WHERE id = ?1",
            params![
                project.id,
                project.name,
                project.compute_type,
                project.remote_host,
                project.status,
                project.nodes
            ],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_project(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute("DELETE FROM projects WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_repositories_for_project(&self, project_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Failed to lock database".to_string())?;
        conn.execute("DELETE FROM repositories WHERE project_id = ?1", params![project_id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── Phase R3: Workflow catalog ─────────────────────────────────────────────

    fn workflow_create(&self, w: Workflow) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        conn.execute(
            "INSERT INTO workflows (id, name, description, is_starter, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![w.id, w.name, w.description, w.is_starter as i32, w.created_at, w.updated_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn workflow_update_meta(&self, id: &str, name: &str, description: &str) -> Result<(), String> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        conn.execute(
            "UPDATE workflows SET name=?2, description=?3, updated_at=?4 WHERE id=?1",
            params![id, name, description, now],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn workflow_delete(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        // Refuse to delete starter pack workflows
        let is_starter: i32 = conn.query_row(
            "SELECT is_starter FROM workflows WHERE id=?1", params![id], |r| r.get(0)
        ).map_err(|_| "Workflow not found".to_string())?;
        if is_starter == 1 {
            return Err("Cannot delete a starter pack workflow. Use 'Revert to Default' instead.".to_string());
        }
        conn.execute("DELETE FROM workflows WHERE id=?1", params![id]).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn workflow_get(&self, id: &str) -> Result<Option<Workflow>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id,name,description,is_starter,created_at,updated_at FROM workflows WHERE id=?1"
        ).map_err(|e| e.to_string())?;
        let mut iter = stmt.query_map(params![id], |row| {
            Ok(Workflow {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                is_starter: row.get::<_, i32>(3)? != 0,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        }).map_err(|e| e.to_string())?;
        if let Some(r) = iter.next() { Ok(Some(r.map_err(|e| e.to_string())?)) } else { Ok(None) }
    }

    fn workflow_list(&self) -> Result<Vec<Workflow>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id,name,description,is_starter,created_at,updated_at FROM workflows ORDER BY is_starter DESC, created_at ASC"
        ).map_err(|e| e.to_string())?;
        let iter = stmt.query_map([], |row| {
            Ok(Workflow {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                is_starter: row.get::<_, i32>(3)? != 0,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        }).map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter { list.push(r.map_err(|e| e.to_string())?); }
        Ok(list)
    }

    fn workflow_save_version(&self, v: WorkflowVersion) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        conn.execute(
            "INSERT INTO workflow_versions (id,workflow_id,version,steps_json,note,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![v.id, v.workflow_id, v.version, v.steps_json, v.note, v.created_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn workflow_latest_version(&self, workflow_id: &str) -> Result<Option<WorkflowVersion>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id,workflow_id,version,steps_json,note,created_at FROM workflow_versions WHERE workflow_id=?1 ORDER BY version DESC LIMIT 1"
        ).map_err(|e| e.to_string())?;
        let mut iter = stmt.query_map(params![workflow_id], |row| {
            Ok(WorkflowVersion {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                version: row.get::<_, u32>(2)?,
                steps_json: row.get(3)?,
                note: row.get(4)?,
                created_at: row.get(5)?,
            })
        }).map_err(|e| e.to_string())?;
        if let Some(r) = iter.next() { Ok(Some(r.map_err(|e| e.to_string())?)) } else { Ok(None) }
    }

    fn workflow_versions(&self, workflow_id: &str) -> Result<Vec<WorkflowVersion>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id,workflow_id,version,steps_json,note,created_at FROM workflow_versions WHERE workflow_id=?1 ORDER BY version ASC"
        ).map_err(|e| e.to_string())?;
        let iter = stmt.query_map(params![workflow_id], |row| {
            Ok(WorkflowVersion {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                version: row.get::<_, u32>(2)?,
                steps_json: row.get(3)?,
                note: row.get(4)?,
                created_at: row.get(5)?,
            })
        }).map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter { list.push(r.map_err(|e| e.to_string())?); }
        Ok(list)
    }

    fn workflow_count(&self) -> Result<u32, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM workflows", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count as u32)
    }

    // ── Phase R4: Step execution + gate ───────────────────────────────────────

    fn step_execution_create(&self, s: StepExecution) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        conn.execute(
            "INSERT INTO step_executions (id,feature_id,step_id,step_index,step_kind,status,cost_usd,wall_clock_secs,artifact_path,error_message,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![s.id,s.feature_id,s.step_id,s.step_index,s.step_kind,s.status,s.cost_usd,s.wall_clock_secs.map(|v| v as i64),s.artifact_path,s.error_message,s.created_at,s.updated_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn step_execution_get(&self, id: &str) -> Result<Option<StepExecution>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id,feature_id,step_id,step_index,step_kind,status,cost_usd,wall_clock_secs,artifact_path,error_message,created_at,updated_at FROM step_executions WHERE id=?1"
        ).map_err(|e| e.to_string())?;
        let mut iter = stmt.query_map(params![id], |row| {
            Ok(StepExecution {
                id: row.get(0)?,
                feature_id: row.get(1)?,
                step_id: row.get(2)?,
                step_index: row.get::<_, u32>(3)?,
                step_kind: row.get(4)?,
                status: row.get(5)?,
                cost_usd: row.get(6)?,
                wall_clock_secs: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                artifact_path: row.get(8)?,
                error_message: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
            })
        }).map_err(|e| e.to_string())?;
        if let Some(r) = iter.next() { Ok(Some(r.map_err(|e| e.to_string())?)) } else { Ok(None) }
    }

    fn step_execution_update_status(
        &self,
        id: &str,
        status: &str,
        cost_usd: Option<f64>,
        wall_clock_secs: Option<u64>,
        artifact_path: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), String> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        conn.execute(
            "UPDATE step_executions SET status=?2,cost_usd=?3,wall_clock_secs=?4,artifact_path=?5,error_message=?6,updated_at=?7 WHERE id=?1",
            params![id, status, cost_usd, wall_clock_secs.map(|v| v as i64), artifact_path, error_message, now],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn step_executions_for_feature(&self, feature_id: &str) -> Result<Vec<StepExecution>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        let mut stmt = conn.prepare(
            "SELECT id,feature_id,step_id,step_index,step_kind,status,cost_usd,wall_clock_secs,artifact_path,error_message,created_at,updated_at FROM step_executions WHERE feature_id=?1 ORDER BY step_index ASC"
        ).map_err(|e| e.to_string())?;
        let iter = stmt.query_map(params![feature_id], |row| {
            Ok(StepExecution {
                id: row.get(0)?,
                feature_id: row.get(1)?,
                step_id: row.get(2)?,
                step_index: row.get::<_, u32>(3)?,
                step_kind: row.get(4)?,
                status: row.get(5)?,
                cost_usd: row.get(6)?,
                wall_clock_secs: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                artifact_path: row.get(8)?,
                error_message: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
            })
        }).map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter { list.push(r.map_err(|e| e.to_string())?); }
        Ok(list)
    }

    fn update_feature_status(&self, id: &str, status: &str, total_cost: Option<f64>, duration: Option<&str>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        if let (Some(cost), Some(dur)) = (total_cost, duration) {
            conn.execute(
                "UPDATE features SET status=?2, total_cost=?3, duration=?4 WHERE id=?1",
                params![id, status, cost, dur],
            ).map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE features SET status=?2 WHERE id=?1",
                params![id, status],
            ).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn gate_create(&self, g: GateDecision) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        conn.execute(
            "INSERT INTO gate_decisions (id,step_execution_id,decision,feedback,created_at) VALUES (?1,?2,?3,?4,?5)",
            params![g.id, g.step_execution_id, g.decision, g.feedback, g.created_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn gate_decide(&self, step_execution_id: &str, decision: &str, feedback: Option<&str>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        conn.execute(
            "UPDATE gate_decisions SET decision=?2, feedback=?3 WHERE step_execution_id=?1",
            params![step_execution_id, decision, feedback],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn gate_pending_for_feature(&self, feature_id: &str) -> Result<Option<GateDecision>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        let mut stmt = conn.prepare(
            "SELECT gd.id,gd.step_execution_id,gd.decision,gd.feedback,gd.created_at
             FROM gate_decisions gd
             JOIN step_executions se ON se.id = gd.step_execution_id
             WHERE se.feature_id=?1 AND gd.decision IS NULL
             ORDER BY gd.created_at DESC LIMIT 1"
        ).map_err(|e| e.to_string())?;
        let mut iter = stmt.query_map(params![feature_id], |row| {
            Ok(GateDecision {
                id: row.get(0)?,
                step_execution_id: row.get(1)?,
                decision: row.get(2)?,
                feedback: row.get(3)?,
                created_at: row.get(4)?,
            })
        }).map_err(|e| e.to_string())?;
        if let Some(r) = iter.next() { Ok(Some(r.map_err(|e| e.to_string())?)) } else { Ok(None) }
    }

    // ── App settings ─────────────────────────────────────────────────────────

    fn app_setting_get(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        Ok(conn.query_row("SELECT value FROM app_settings WHERE key=?1", params![key], |r| r.get(0)).ok())
    }

    fn app_setting_set(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "db lock".to_string())?;
        conn.execute(
            "INSERT INTO app_settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_update_and_delete_project() {
        let conn = Connection::open_in_memory().unwrap();
        let adapter = SqliteAdapter::new(conn);

        // 1. Insert a test project
        let project = Project {
            id: "test_p1".to_string(),
            name: "Test Project".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 4,
            spend: 0.0,
            created_at: 123456,
        };
        adapter.add_project(project.clone()).unwrap();

        // Check it was inserted
        let projects = adapter.get_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Test Project");

        // 2. Add repository
        let repo = Repository {
            id: "test_r1".to_string(),
            project_id: "test_p1".to_string(),
            provider_id: "github".to_string(),
            repo_path: "org/repo".to_string(),
        };
        adapter.add_repository(repo).unwrap();

        let repos = adapter.get_repositories_for_project("test_p1").unwrap();
        assert_eq!(repos.len(), 1);

        // 3. Update project details
        let updated = Project {
            id: "test_p1".to_string(),
            name: "Updated Project".to_string(),
            compute_type: "remote".to_string(),
            remote_host: Some("machine_1".to_string()),
            status: "bootstrapping".to_string(),
            nodes: 8,
            spend: 10.5,
            created_at: 123456,
        };
        adapter.update_project(updated).unwrap();

        let projects = adapter.get_projects().unwrap();
        assert_eq!(projects[0].name, "Updated Project");
        assert_eq!(projects[0].compute_type, "remote");
        assert_eq!(projects[0].remote_host, Some("machine_1".to_string()));
        assert_eq!(projects[0].status, "bootstrapping");
        assert_eq!(projects[0].nodes, 8);

        // 4. Delete repositories for project
        adapter.delete_repositories_for_project("test_p1").unwrap();
        let repos = adapter.get_repositories_for_project("test_p1").unwrap();
        assert!(repos.is_empty());

        // Re-insert repository for delete cascade check
        let repo = Repository {
            id: "test_r1_cascade".to_string(),
            project_id: "test_p1".to_string(),
            provider_id: "github".to_string(),
            repo_path: "org/repo-cascade".to_string(),
        };
        adapter.add_repository(repo).unwrap();

        // 5. Delete project (should cascade delete repos)
        adapter.delete_project("test_p1").unwrap();
        let projects = adapter.get_projects().unwrap();
        assert!(projects.is_empty());

        // Check if repositories cascade deleted
        let repos = adapter.get_repositories_for_project("test_p1").unwrap();
        assert!(repos.is_empty());
    }
}
