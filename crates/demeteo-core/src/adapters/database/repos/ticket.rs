//! SQL for `tickets` and `ticket_feature_attempts` (V47). The contract — the
//! stored vocabulary is three states and everything else is derived — is
//! stated once on [`TicketPort`].

use rusqlite::params;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::domain::ids::{DiscoveryId, FeatureId, TicketId};
use crate::domain::models::{EffortLevel, Ticket, TicketFeatureAttempt, TicketState};
use crate::ports::discovery::{TicketPatch, TicketPort};

use super::super::SqliteAdapter;

const COLUMNS: &str = "id, discovery_id, seq, title, description, acceptance_json, files_json,
     blocked_by_json, test_command, workflow_id, agent_kind, model, effort, attachments_json,
     state, drop_reason, force_start_reason, force_started_at, feature_id, created_at, updated_at";

const ATTEMPT_COLUMNS: &str = "ticket_id, feature_id, started_at, superseded_at";

fn encode<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

fn decode<T: Default + DeserializeOwned>(raw: Option<String>) -> T {
    raw.as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default()
}

fn row_to_ticket(row: &rusqlite::Row) -> rusqlite::Result<Ticket> {
    let effort: Option<String> = row.get(12)?;
    let state: String = row.get(14)?;
    Ok(Ticket {
        id: row.get(0)?,
        discovery_id: row.get(1)?,
        seq: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        acceptance: decode(row.get(5)?),
        files: decode(row.get(6)?),
        blocked_by: decode(row.get(7)?),
        test_command: row.get(8)?,
        workflow_id: row.get(9)?,
        agent_kind: row.get(10)?,
        model: row.get(11)?,
        effort: effort.as_deref().and_then(EffortLevel::parse),
        attachments: decode(row.get(13)?),
        // A state this build cannot name reads as started, which is the only
        // one of the three that is safe to be wrong about: it holds the row
        // immutable against a re-decomposition (§5.3) and satisfies no
        // dependent, where `dropped` would release the whole graph below it.
        state: TicketState::parse(&state).unwrap_or(TicketState::Started),
        drop_reason: row.get(15)?,
        force_start_reason: row.get(16)?,
        force_started_at: row.get(17)?,
        feature_id: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}

fn row_to_attempt(row: &rusqlite::Row) -> rusqlite::Result<TicketFeatureAttempt> {
    Ok(TicketFeatureAttempt {
        ticket_id: row.get(0)?,
        feature_id: row.get(1)?,
        started_at: row.get(2)?,
        superseded_at: row.get(3)?,
    })
}

fn insert_ticket(conn: &rusqlite::Connection, ticket: &Ticket) -> Result<(), String> {
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO tickets ({COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?21)"
        ),
        params![
            ticket.id,
            ticket.discovery_id,
            ticket.seq,
            ticket.title,
            ticket.description,
            encode(&ticket.acceptance)?,
            encode(&ticket.files)?,
            encode(&ticket.blocked_by)?,
            ticket.test_command,
            ticket.workflow_id,
            ticket.agent_kind,
            ticket.model,
            ticket.effort.map(EffortLevel::as_str),
            encode(&ticket.attachments)?,
            ticket.state.as_str(),
            ticket.drop_reason,
            ticket.force_start_reason,
            ticket.force_started_at,
            ticket.feature_id,
            ticket.created_at,
            ticket.updated_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

impl TicketPort for SqliteAdapter {
    fn list_for_discovery(&self, discovery_id: &DiscoveryId) -> Result<Vec<Ticket>, String> {
        let conn = self.conn.lock()?;
        let sql = format!(
            "SELECT {COLUMNS} FROM tickets
             WHERE discovery_id = ?1
             ORDER BY seq ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![discovery_id.0], row_to_ticket)
            .map_err(|e| e.to_string())?;
        iter.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    fn get(&self, id: &TicketId) -> Result<Option<Ticket>, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM tickets WHERE id = ?1"),
            params![id.0],
            row_to_ticket,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e.to_string()),
        })
    }

    fn upsert_batch(&self, tickets: &[Ticket]) -> Result<(), String> {
        let conn = self.conn.lock()?;
        for ticket in tickets {
            insert_ticket(&conn, ticket)?;
        }
        Ok(())
    }

    fn update(&self, id: &TicketId, patch: &TicketPatch, now: i64) -> Result<(), String> {
        let acceptance = patch.acceptance.as_ref().map(encode).transpose()?;
        let files = patch.files.as_ref().map(encode).transpose()?;
        let blocked_by = patch.blocked_by.as_ref().map(encode).transpose()?;
        let attachments = patch.attachments.as_ref().map(encode).transpose()?;
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE tickets
                SET title              = COALESCE(?2, title),
                    description        = COALESCE(?3, description),
                    acceptance_json    = COALESCE(?4, acceptance_json),
                    files_json         = COALESCE(?5, files_json),
                    blocked_by_json    = COALESCE(?6, blocked_by_json),
                    test_command       = CASE WHEN ?7 THEN ?8 ELSE test_command END,
                    workflow_id        = CASE WHEN ?9 THEN ?10 ELSE workflow_id END,
                    agent_kind         = CASE WHEN ?11 THEN ?12 ELSE agent_kind END,
                    model              = CASE WHEN ?13 THEN ?14 ELSE model END,
                    effort             = CASE WHEN ?15 THEN ?16 ELSE effort END,
                    attachments_json   = COALESCE(?17, attachments_json),
                    state              = COALESCE(?18, state),
                    drop_reason        = CASE WHEN ?19 THEN ?20 ELSE drop_reason END,
                    force_start_reason = CASE WHEN ?21 THEN ?22 ELSE force_start_reason END,
                    force_started_at   = CASE WHEN ?23 THEN ?24 ELSE force_started_at END,
                    feature_id         = CASE WHEN ?25 THEN ?26 ELSE feature_id END,
                    updated_at         = ?27
              WHERE id = ?1",
            params![
                id.0,
                patch.title,
                patch.description,
                acceptance,
                files,
                blocked_by,
                patch.test_command.is_some(),
                patch.test_command.clone().flatten(),
                patch.workflow_id.is_some(),
                patch.workflow_id.clone().flatten(),
                patch.agent_kind.is_some(),
                patch.agent_kind.clone().flatten(),
                patch.model.is_some(),
                patch.model.clone().flatten(),
                patch.effort.is_some(),
                patch.effort.flatten().map(EffortLevel::as_str),
                attachments,
                patch.state.map(TicketState::as_str),
                patch.drop_reason.is_some(),
                patch.drop_reason.clone().flatten(),
                patch.force_start_reason.is_some(),
                patch.force_start_reason.clone().flatten(),
                patch.force_started_at.is_some(),
                patch.force_started_at.flatten(),
                patch.feature_id.is_some(),
                patch.feature_id.clone().flatten(),
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete(&self, id: &TicketId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM tickets WHERE id = ?1", params![id.0])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn next_seq(&self, discovery_id: &DiscoveryId) -> Result<i64, String> {
        let conn = self.conn.lock()?;
        conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM tickets WHERE discovery_id = ?1",
            params![discovery_id.0],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    }

    fn for_feature(&self, feature_id: &FeatureId) -> Result<Vec<Ticket>, String> {
        let conn = self.conn.lock()?;
        let sql = format!(
            "SELECT {COLUMNS} FROM tickets
             WHERE feature_id = ?1
             ORDER BY seq ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![feature_id.0], row_to_ticket)
            .map_err(|e| e.to_string())?;
        iter.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }

    fn record_attempt(
        &self,
        ticket_id: &TicketId,
        feature_id: &FeatureId,
        now: i64,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        // `DO NOTHING` rather than a replace: re-recording the attempt a ticket
        // is already on must not move its `started_at` forward, and must not
        // resurrect one that `supersede_attempts` has closed.
        conn.execute(
            &format!(
                "INSERT INTO ticket_feature_attempts ({ATTEMPT_COLUMNS})
                 VALUES (?1, ?2, ?3, NULL)
                 ON CONFLICT(ticket_id, feature_id) DO NOTHING"
            ),
            params![ticket_id.0, feature_id.0, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn supersede_attempts(&self, ticket_id: &TicketId, now: i64) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE ticket_feature_attempts
                SET superseded_at = ?2
              WHERE ticket_id = ?1 AND superseded_at IS NULL",
            params![ticket_id.0, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list_attempts(&self, ticket_id: &TicketId) -> Result<Vec<TicketFeatureAttempt>, String> {
        let conn = self.conn.lock()?;
        let sql = format!(
            "SELECT {ATTEMPT_COLUMNS} FROM ticket_feature_attempts
             WHERE ticket_id = ?1
             ORDER BY started_at ASC, feature_id ASC"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![ticket_id.0], row_to_attempt)
            .map_err(|e| e.to_string())?;
        iter.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/repos/ticket.rs"]
mod tests;
