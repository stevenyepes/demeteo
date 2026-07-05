use rusqlite::params;

use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::GateDecision;
use crate::ports::db::GateRepository;

use super::super::SqliteAdapter;

impl GateRepository for SqliteAdapter {
    fn create(&self, g: GateDecision) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO gate_decisions (id,step_execution_id,decision,feedback,created_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                g.id,
                g.step_execution_id,
                g.decision,
                g.feedback,
                g.created_at
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn decide(
        &self,
        step_execution_id: &StepExecutionId,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE gate_decisions SET decision=?2, feedback=?3 WHERE step_execution_id=?1",
            params![step_execution_id.0, decision, feedback],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn upsert_decision(
        &self,
        step_execution_id: &StepExecutionId,
        decision: &str,
        feedback: Option<&str>,
        created_at: i64,
    ) -> Result<(), String> {
        // The row is keyed on step_execution_id (UNIQUE), so UPSERT
        // collapses both "first delivery" and "re-delivery after driver
        // restart" into a single atomic statement. Created_at is set on
        // insert and preserved on update so we can tell *when* the gate
        // was originally surfaced (vs when the decision arrived).
        let id = format!("gd-{}", step_execution_id.0);
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO gate_decisions (id, step_execution_id, decision, feedback, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(step_execution_id) DO UPDATE SET
                 decision = excluded.decision,
                 feedback = excluded.feedback",
            params![id, step_execution_id.0, decision, feedback, created_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn latest_for_step(
        &self,
        step_execution_id: &StepExecutionId,
    ) -> Result<Option<GateDecision>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, step_execution_id, decision, feedback, created_at
                 FROM gate_decisions
                 WHERE step_execution_id = ?1
                 ORDER BY created_at DESC LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![step_execution_id.0], |row| {
                Ok(GateDecision {
                    id: row.get(0)?,
                    step_execution_id: row.get(1)?,
                    decision: row.get(2)?,
                    feedback: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(g)) => Ok(Some(g)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn pending_for_feature(&self, feature_id: &FeatureId) -> Result<Option<GateDecision>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT gd.id,gd.step_execution_id,gd.decision,gd.feedback,gd.created_at
                 FROM gate_decisions gd
                 JOIN step_executions se ON se.id = gd.step_execution_id
                 WHERE se.feature_id=?1 AND gd.decision IS NULL
                 ORDER BY gd.created_at DESC LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![feature_id.0], |row| {
                Ok(GateDecision {
                    id: row.get(0)?,
                    step_execution_id: row.get(1)?,
                    decision: row.get(2)?,
                    feedback: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(g)) => Ok(Some(g)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn reset_for_step_execution(&self, step_execution_id: &StepExecutionId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "DELETE FROM gate_decisions WHERE step_execution_id=?1",
            params![step_execution_id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn latest_decided_for_feature(
        &self,
        feature_id: &FeatureId,
    ) -> Result<Option<GateDecision>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT gd.id,gd.step_execution_id,gd.decision,gd.feedback,gd.created_at
                 FROM gate_decisions gd
                 JOIN step_executions se ON se.id = gd.step_execution_id
                 WHERE se.feature_id=?1 AND gd.decision IS NOT NULL
                 ORDER BY gd.created_at DESC LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![feature_id.0], |row| {
                Ok(GateDecision {
                    id: row.get(0)?,
                    step_execution_id: row.get(1)?,
                    decision: row.get(2)?,
                    feedback: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(g)) => Ok(Some(g)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }
}
