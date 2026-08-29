//! SQL for `ask_thread` and `ask_message` (V51). Column shape mirrors
//! [`discovery`](super::discovery), minus the decomposition surface
//! [`AskPort`] deliberately omits.

use rusqlite::params;

use crate::domain::ids::{AskThreadId, ProjectId};
use crate::domain::models::{
    AskMessage, AskStatus, AskThread, CanvasPathVerdict, EffortLevel, MessageRole, TurnActivity,
};
use crate::ports::ask::{AskPort, AskThreadPatch};

use super::super::SqliteAdapter;

const COLUMNS: &str = "id, project_id, title, status, agent_kind, model, effort, machine_id,
     worktree_path, session_id, turn_count, cost_usd, tokens, created_at, updated_at";

const MESSAGE_COLUMNS: &str = "id, thread_id, role, text, cost_usd, tokens, turn_activity_json,
     canvas_paths_json, checked_commit_sha, created_at";

fn row_to_thread(row: &rusqlite::Row) -> rusqlite::Result<AskThread> {
    let status: String = row.get(3)?;
    let effort: Option<String> = row.get(6)?;
    Ok(AskThread {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        // A status this build cannot name is not one it knows how to keep
        // open, and `closed` is the state that keeps the transcript.
        status: AskStatus::parse(&status).unwrap_or(AskStatus::Closed),
        agent_kind: row.get(4)?,
        model: row.get(5)?,
        effort: effort.as_deref().and_then(EffortLevel::parse),
        machine_id: row.get(7)?,
        worktree_path: row.get(8)?,
        session_id: row.get(9)?,
        turn_count: row.get(10)?,
        cost_usd: row.get(11)?,
        tokens: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn row_to_message(row: &rusqlite::Row) -> rusqlite::Result<AskMessage> {
    let role: String = row.get(2)?;
    Ok(AskMessage {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        // A role this build cannot name is read as the assistant's, never the
        // user's, mirrors `discovery::row_to_message`.
        role: MessageRole::parse(&role).unwrap_or(MessageRole::Assistant),
        text: row.get(3)?,
        cost_usd: row.get(4)?,
        tokens: row.get(5)?,
        turn_activity: decode_activity(row.get(6)?),
        canvas_paths: decode_canvas_paths(row.get(7)?),
        checked_commit_sha: row.get(8)?,
        created_at: row.get(9)?,
    })
}

/// A summary this build cannot read is read as no summary at all, same terms
/// as `discovery::decode_activity`.
fn decode_activity(raw: Option<String>) -> Option<TurnActivity> {
    raw.as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
}

/// Same degrade-to-`None` convention as [`decode_activity`].
fn decode_canvas_paths(raw: Option<String>) -> Option<Vec<CanvasPathVerdict>> {
    raw.as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
}

impl AskPort for SqliteAdapter {
    fn create(&self, thread: &AskThread) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            &format!(
                "INSERT INTO ask_thread ({COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"
            ),
            params![
                thread.id,
                thread.project_id,
                thread.title,
                thread.status.as_str(),
                thread.agent_kind,
                thread.model,
                thread.effort.map(EffortLevel::as_str),
                thread.machine_id,
                thread.worktree_path,
                thread.session_id,
                thread.turn_count,
                thread.cost_usd,
                thread.tokens,
                thread.created_at,
                thread.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get(&self, id: &AskThreadId) -> Result<Option<AskThread>, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM ask_thread WHERE id = ?1"),
            params![id.0],
            row_to_thread,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e.to_string()),
        })
    }

    fn list_for_project(&self, project_id: &ProjectId) -> Result<Vec<AskThread>, String> {
        let conn = self.conn.lock()?;
        let sql = format!(
            "SELECT {COLUMNS} FROM ask_thread
              WHERE project_id = ?1
              ORDER BY updated_at DESC"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id.0], row_to_thread)
            .map_err(|e| e.to_string())?;
        iter.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    fn update(&self, id: &AskThreadId, patch: &AskThreadPatch, now: i64) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE ask_thread
                SET title         = COALESCE(?2, title),
                    status        = COALESCE(?3, status),
                    worktree_path = CASE WHEN ?4 THEN ?5 ELSE worktree_path END,
                    session_id    = CASE WHEN ?6 THEN ?7 ELSE session_id END,
                    turn_count    = turn_count + ?8,
                    cost_usd      = cost_usd + ?9,
                    tokens        = tokens + ?10,
                    updated_at    = ?11
              WHERE id = ?1",
            params![
                id.0,
                patch.title,
                patch.status.map(AskStatus::as_str),
                patch.worktree_path.is_some(),
                patch.worktree_path.clone().flatten(),
                patch.session_id.is_some(),
                patch.session_id.clone().flatten(),
                patch.add_turns,
                patch.add_cost_usd,
                patch.add_tokens,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete(&self, id: &AskThreadId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM ask_thread WHERE id = ?1", params![id.0])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn append_message(&self, message: &AskMessage) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO ask_message ({MESSAGE_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            params![
                message.id,
                message.thread_id,
                message.role.as_str(),
                message.text,
                message.cost_usd,
                message.tokens,
                message
                    .turn_activity
                    .as_ref()
                    .and_then(|a| serde_json::to_string(a).ok()),
                message
                    .canvas_paths
                    .as_ref()
                    .and_then(|p| serde_json::to_string(p).ok()),
                message.checked_commit_sha,
                message.created_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list_messages(&self, id: &AskThreadId) -> Result<Vec<AskMessage>, String> {
        let conn = self.conn.lock()?;
        // `id` breaks the tie: two messages of one turn can share a
        // timestamp, and a transcript re-seeded out of order is a different
        // conversation.
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM ask_message
             WHERE thread_id = ?1
             ORDER BY created_at ASC, id ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![id.0], row_to_message)
            .map_err(|e| e.to_string())?;
        iter.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/ask.rs"]
mod tests;
