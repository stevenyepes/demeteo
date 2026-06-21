use rusqlite::params;

use crate::domain::ids::{MachineId, ThreadId};
use crate::domain::models::{AgentConfig, Message, ThreadSession, WorkingMemoryEntry};
use crate::ports::db::{ThreadPatch, ThreadRepository};

use super::super::SqliteAdapter;

impl ThreadRepository for SqliteAdapter {
    fn get_thread_sessions(&self, machine_id: &MachineId) -> Result<Vec<ThreadSession>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, machine_id, title, mode, branch, repo_path,
                        sandbox_path, status, agent_kind, model, updated_at
                 FROM thread_sessions WHERE machine_id = ?1
                 ORDER BY COALESCE(updated_at, CAST(strftime('%s', created_at) AS INTEGER) * 1000) DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![machine_id.0], |row| {
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
            })
            .map_err(|e| e.to_string())?;
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn get_thread_sessions_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> Result<Vec<ThreadSession>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, machine_id, title, mode, branch, repo_path,
                        sandbox_path, status, agent_kind, model, updated_at
                 FROM thread_sessions WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![thread_id.0], |row| {
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
            })
            .map_err(|e| e.to_string())?;
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn add_thread_session(&self, t: ThreadSession) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO thread_sessions (id, machine_id, title, mode, branch, repo_path, sandbox_path, status, agent_kind, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![t.id, t.machine_id, t.title, t.mode, t.branch, t.repo_path, t.sandbox_path, t.status, t.agent_kind, t.model, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_thread_session(&self, id: &ThreadId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM thread_sessions WHERE id = ?1", params![id.0])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_thread(&self, id: &ThreadId, patch: &ThreadPatch) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let conn = self.conn.lock()?;
        if let Some(status) = &patch.status {
            conn.execute(
                "UPDATE thread_sessions SET status = ?2 WHERE id = ?1",
                params![id.0, status],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(model) = &patch.model {
            if let Some(m) = model {
                conn.execute(
                    "UPDATE thread_sessions SET model = ?2 WHERE id = ?1",
                    params![id.0, m],
                )
                .map_err(|e| e.to_string())?;
            } else {
                conn.execute(
                    "UPDATE thread_sessions SET model = NULL WHERE id = ?1",
                    params![id.0],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        if patch.touch_timestamp {
            conn.execute(
                "UPDATE thread_sessions SET updated_at = ?2 WHERE id = ?1",
                params![id.0, now],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn get_messages(&self, thread_id: &ThreadId) -> Result<Vec<Message>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, thread_id, role, content, metadata, created_at
                 FROM messages WHERE thread_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![thread_id.0], |row| {
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
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn append_message(&self, msg: &Message) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO messages (id, thread_id, role, content, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                msg.id,
                msg.thread_id,
                msg.role,
                msg.content,
                msg.metadata,
                msg.created_at
            ],
        )
        .map_err(|e| e.to_string())?;
        // Bump the thread's updated_at so the sidebar reorders.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        conn.execute(
            "UPDATE thread_sessions SET updated_at = ?2 WHERE id = ?1",
            params![msg.thread_id.0, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_messages(&self, thread_id: &ThreadId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "DELETE FROM messages WHERE thread_id = ?1",
            params![thread_id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_agent_configs(&self, machine_id: &MachineId) -> Result<Vec<AgentConfig>, String> {
        let conn = self.conn.lock()?;
        let raw: Option<String> = conn
            .query_row(
                "SELECT agents FROM machines WHERE id = ?1",
                params![machine_id.0],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        let parsed: Vec<AgentConfig> = raw
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        Ok(parsed)
    }

    fn set_agent_configs(&self, machine_id: &MachineId, agents_json: &str) -> Result<(), String> {
        let _: Vec<AgentConfig> =
            serde_json::from_str(agents_json).map_err(|e| format!("Invalid agents JSON: {}", e))?;
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE machines SET agents = ?2 WHERE id = ?1",
            params![machine_id.0, agents_json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn upsert_working_memory_entry(
        &self,
        thread_id: &ThreadId,
        entry: WorkingMemoryEntry,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
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
                thread_id.0,
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

    fn get_working_memory(&self, thread_id: &ThreadId) -> Result<Vec<WorkingMemoryEntry>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT file_path, line_count, size_bytes, modified_at, first_read_at, last_read_at
                 FROM thread_working_memory WHERE thread_id = ?1
                 ORDER BY last_read_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![thread_id.0], |row| {
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
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn clear_working_memory(&self, thread_id: &ThreadId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "DELETE FROM thread_working_memory WHERE thread_id = ?1",
            params![thread_id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
