use rusqlite::params;

use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, WorkflowId};
use crate::domain::models::{Feature, StepExecution};
use crate::ports::db::{FeaturePatch, FeatureRepository, StepExecutionPatch};

use super::super::SqliteAdapter;

impl FeatureRepository for SqliteAdapter {
    fn get_active(&self, project_id: &ProjectId) -> Result<Vec<Feature>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state
                 FROM features WHERE project_id = ?1 AND status NOT IN ('archived', 'deleted') ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id.0], |row| {
                Ok(Feature {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    workflow_id: row.get(2)?,
                    title: row.get(3)?,
                    status: row.get(4)?,
                    total_cost: row.get(5)?,
                    duration: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                    agent_kind: row.get(9)?,
                    model: row.get(10)?,
                    mr_url: row.get(11)?,
                    mr_state: row.get(12)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn get(&self, id: &FeatureId) -> Result<Option<Feature>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state
                 FROM features WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
                Ok(Feature {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    workflow_id: row.get(2)?,
                    title: row.get(3)?,
                    status: row.get(4)?,
                    total_cost: row.get(5)?,
                    duration: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                    agent_kind: row.get(9)?,
                    model: row.get(10)?,
                    mr_url: row.get(11)?,
                    mr_state: row.get(12)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(f)) => Ok(Some(f)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn add(&self, f: Feature) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO features (id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                f.id, f.project_id, f.workflow_id, f.title, f.status,
                f.total_cost, f.duration, f.tokens, f.created_at, f.agent_kind, f.model,
                f.mr_url, f.mr_state
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update(&self, id: &FeatureId, patch: &FeaturePatch) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let cost: Option<f64> = patch.total_cost.flatten();
        let dur: Option<String> = patch.duration.clone().flatten();
        let tokens: Option<i64> = patch.tokens.flatten();
        let agent_kind: Option<Option<String>> = patch.agent_kind.clone();
        let model: Option<Option<String>> = patch.model.clone();
        let mr_url: Option<Option<String>> = patch.mr_url.clone();
        let mr_state: Option<Option<String>> = patch.mr_state.clone();

        // Build the SET clause dynamically so a `None` field on the patch
        // actually means "leave the column alone". The previous code
        // always bound total_cost / duration when status was set, which
        // collapsed `None` → `NULL` and tripped the NOT NULL constraints
        // (see migration V1, features.total_cost / duration). step_retry
        // hit this because it intentionally preserves the existing cost
        // when re-running a failed step.
        let mut sets: Vec<&str> = Vec::new();
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = &patch.status {
            sets.push("status=?");
            binds.push(Box::new(s.clone()));
        }
        if let Some(c) = cost {
            sets.push("total_cost=?");
            binds.push(Box::new(c));
        }
        if let Some(d) = &dur {
            sets.push("duration=?");
            binds.push(Box::new(d.clone()));
        }
        if let Some(t) = tokens {
            sets.push("tokens=?");
            binds.push(Box::new(t));
        }
        if let Some(ak) = agent_kind {
            sets.push("agent_kind=?");
            binds.push(Box::new(ak));
        }
        if let Some(m) = model {
            sets.push("model=?");
            binds.push(Box::new(m));
        }
        if let Some(url) = mr_url {
            sets.push("mr_url=?");
            binds.push(Box::new(url));
        }
        if let Some(state) = mr_state {
            sets.push("mr_state=?");
            binds.push(Box::new(state));
        }
        if sets.is_empty() {
            return Ok(());
        }
        let sql = format!("UPDATE features SET {} WHERE id=?", sets.join(", "));
        binds.push(Box::new(id.0.clone()));

        conn.execute(&sql, rusqlite::params_from_iter(binds.iter()))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_workflow_id(&self, id: &FeatureId, workflow_id: &WorkflowId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE features SET workflow_id = ?2 WHERE id = ?1",
            params![id.0, workflow_id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn step_create(&self, s: StepExecution) -> Result<(), String> {
        super::feature_steps::step_create(self, s)
    }

    fn step_get(&self, id: &StepExecutionId) -> Result<Option<StepExecution>, String> {
        super::feature_steps::step_get(self, id)
    }

    fn step_update(&self, id: &StepExecutionId, patch: &StepExecutionPatch) -> Result<(), String> {
        super::feature_steps::step_update(self, id, patch)
    }

    fn steps_for_feature(&self, feature_id: &FeatureId) -> Result<Vec<StepExecution>, String> {
        super::feature_steps::steps_for_feature(self, feature_id)
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/feature.rs"]
mod tests;
