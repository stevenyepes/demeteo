//! SQL for `discoveries` and `discovery_messages` (V47). The contract — the
//! transcript is the authority, the resume id is a cache — is stated once on
//! [`DiscoveryPort`].

use rusqlite::params;

use crate::domain::attachment::AttachedFile;
use crate::domain::ids::{DiscoveryId, ProjectId};
use crate::domain::models::{
    Discovery, DiscoveryMessage, DiscoveryStatus, EffortLevel, MessageRole, TurnActivity,
};
use crate::ports::discovery::{DiscoveryListRow, DiscoveryPatch, DiscoveryPort};

use super::super::SqliteAdapter;

const COLUMNS: &str = "id, project_id, title, status, machine_id, agent_kind, model, effort,
     resume_session_id, worktree_path, attachments_json, total_cost, tokens, created_at,
     updated_at";

const MESSAGE_COLUMNS: &str =
    "id, discovery_id, role, content, cost_usd, tokens, activity_json, created_at";

fn encode_attachments(attachments: &[AttachedFile]) -> Result<String, String> {
    serde_json::to_string(attachments).map_err(|e| e.to_string())
}

fn row_to_discovery(row: &rusqlite::Row) -> rusqlite::Result<Discovery> {
    let status: String = row.get(3)?;
    let effort: Option<String> = row.get(7)?;
    Ok(Discovery {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        // A status this build cannot name is not one it knows how to conduct
        // an interview in, and `closed` is the state that keeps everything.
        status: DiscoveryStatus::parse(&status).unwrap_or(DiscoveryStatus::Closed),
        machine_id: row.get(4)?,
        agent_kind: row.get(5)?,
        model: row.get(6)?,
        effort: effort.as_deref().and_then(EffortLevel::parse),
        resume_session_id: row.get(8)?,
        worktree_path: row.get(9)?,
        attachments: decode_attachments(row.get(10)?),
        total_cost: row.get(11)?,
        tokens: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

/// A manifest this build cannot read is read as no attachments at all: the
/// bytes are still on disk under the Discovery's id, and a prompt that names a
/// file it cannot describe is worse than one that names none.
fn decode_attachments(raw: Option<String>) -> Vec<AttachedFile> {
    raw.as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default()
}

fn row_to_message(row: &rusqlite::Row) -> rusqlite::Result<DiscoveryMessage> {
    let role: String = row.get(2)?;
    Ok(DiscoveryMessage {
        id: row.get(0)?,
        discovery_id: row.get(1)?,
        // A role this build cannot name is read as the assistant's, never the
        // user's: a re-seeded transcript feeds its own prior output back as
        // context, and mis-attributing it to the user turns whatever it says
        // into an instruction.
        role: MessageRole::parse(&role).unwrap_or(MessageRole::Assistant),
        content: row.get(3)?,
        cost_usd: row.get(4)?,
        tokens: row.get(5)?,
        activity: decode_activity(row.get(6)?),
        created_at: row.get(7)?,
    })
}

/// A summary this build cannot read is read as no summary at all — the same
/// terms as [`decode_attachments`], and for the same reason: the meta line can
/// say nothing, and must never say zero.
fn decode_activity(raw: Option<String>) -> Option<TurnActivity> {
    raw.as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
}

impl DiscoveryPort for SqliteAdapter {
    fn list_for_project(&self, project_id: &ProjectId) -> Result<Vec<DiscoveryListRow>, String> {
        let conn = self.conn.lock()?;
        // The count is a correlated subquery rather than a `LEFT JOIN … GROUP
        // BY` so the row mapper stays the one above, positional and shared
        // with `get`: a grouped join would put the aggregate somewhere in the
        // middle of the projection and give this query a second mapper to keep
        // in step with `COLUMNS`.
        let sql = format!(
            "SELECT {COLUMNS},
                    (SELECT COUNT(*) FROM discovery_messages m WHERE m.discovery_id = d.id)
               FROM discoveries d
              WHERE project_id = ?1
              ORDER BY updated_at DESC"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id.0], |row| {
                Ok(DiscoveryListRow {
                    discovery: row_to_discovery(row)?,
                    message_count: row.get(15)?,
                })
            })
            .map_err(|e| e.to_string())?;
        iter.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    fn get(&self, id: &DiscoveryId) -> Result<Option<Discovery>, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM discoveries WHERE id = ?1"),
            params![id.0],
            row_to_discovery,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e.to_string()),
        })
    }

    fn create(&self, discovery: &Discovery) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            &format!(
                "INSERT INTO discoveries ({COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"
            ),
            params![
                discovery.id,
                discovery.project_id,
                discovery.title,
                discovery.status.as_str(),
                discovery.machine_id,
                discovery.agent_kind,
                discovery.model,
                discovery.effort.map(EffortLevel::as_str),
                discovery.resume_session_id,
                discovery.worktree_path,
                encode_attachments(&discovery.attachments)?,
                discovery.total_cost,
                discovery.tokens,
                discovery.created_at,
                discovery.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update(&self, id: &DiscoveryId, patch: &DiscoveryPatch, now: i64) -> Result<(), String> {
        let attachments = patch
            .attachments
            .as_deref()
            .map(encode_attachments)
            .transpose()?;
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE discoveries
                SET title             = COALESCE(?2, title),
                    status            = COALESCE(?3, status),
                    model             = CASE WHEN ?4 THEN ?5 ELSE model END,
                    effort            = CASE WHEN ?6 THEN ?7 ELSE effort END,
                    resume_session_id = CASE WHEN ?8 THEN ?9 ELSE resume_session_id END,
                    worktree_path     = CASE WHEN ?10 THEN ?11 ELSE worktree_path END,
                    attachments_json  = COALESCE(?12, attachments_json),
                    pending_proposal_json = CASE WHEN ?13 THEN ?14 ELSE pending_proposal_json END,
                    total_cost        = total_cost + ?15,
                    tokens            = tokens + ?16,
                    updated_at        = ?17
              WHERE id = ?1",
            params![
                id.0,
                patch.title,
                patch.status.map(DiscoveryStatus::as_str),
                patch.model.is_some(),
                patch.model.clone().flatten(),
                patch.effort.is_some(),
                patch.effort.flatten().map(EffortLevel::as_str),
                patch.resume_session_id.is_some(),
                patch.resume_session_id.clone().flatten(),
                patch.worktree_path.is_some(),
                patch.worktree_path.clone().flatten(),
                attachments,
                patch.pending_proposal.is_some(),
                patch.pending_proposal.clone().flatten(),
                patch.add_cost,
                patch.add_tokens,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete(&self, id: &DiscoveryId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM discoveries WHERE id = ?1", params![id.0])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn append_message(&self, message: &DiscoveryMessage) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            &format!(
                "INSERT OR REPLACE INTO discovery_messages ({MESSAGE_COLUMNS})
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            ),
            params![
                message.id,
                message.discovery_id,
                message.role.as_str(),
                message.content,
                message.cost_usd,
                message.tokens,
                message
                    .activity
                    .as_ref()
                    .and_then(|a| serde_json::to_string(a).ok()),
                message.created_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn pending_proposal(&self, id: &DiscoveryId) -> Result<Option<String>, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            "SELECT pending_proposal_json FROM discoveries WHERE id = ?1",
            params![id.0],
            |row| row.get::<_, Option<String>>(0),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e.to_string()),
        })
    }

    fn list_messages(&self, id: &DiscoveryId) -> Result<Vec<DiscoveryMessage>, String> {
        let conn = self.conn.lock()?;
        // `id` breaks the tie: two messages of one turn are written a
        // millisecond apart at most, and a transcript re-seeded in the wrong
        // order is a different conversation.
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM discovery_messages
             WHERE discovery_id = ?1
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
#[path = "../../../../tests/infrastructure/database/repos/discovery.rs"]
mod tests;
