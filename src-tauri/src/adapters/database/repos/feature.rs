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
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, created_at, agent_kind, model
                 FROM features WHERE project_id = ?1 ORDER BY created_at DESC",
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
                    created_at: row.get(7)?,
                    agent_kind: row.get(8)?,
                    model: row.get(9)?,
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
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, created_at, agent_kind, model
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
                    created_at: row.get(7)?,
                    agent_kind: row.get(8)?,
                    model: row.get(9)?,
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
            "INSERT INTO features (id, project_id, workflow_id, title, status, total_cost, duration, created_at, agent_kind, model)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                f.id, f.project_id, f.workflow_id, f.title, f.status,
                f.total_cost, f.duration, f.created_at, f.agent_kind, f.model
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update(&self, id: &FeatureId, patch: &FeaturePatch) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let cost: Option<f64> = patch.total_cost.flatten();
        let dur: Option<String> = patch.duration.clone().flatten();
        let agent_kind: Option<Option<String>> = patch.agent_kind.clone();
        let model: Option<Option<String>> = patch.model.clone();

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
        if let Some(ak) = agent_kind {
            sets.push("agent_kind=?");
            binds.push(Box::new(ak));
        }
        if let Some(m) = model {
            sets.push("model=?");
            binds.push(Box::new(m));
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
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO step_executions (id,feature_id,step_id,step_index,step_kind,status,cost_usd,wall_clock_secs,artifact_path,error_message,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                s.id, s.feature_id, s.step_id, s.step_index, s.step_kind, s.status,
                s.cost_usd, s.wall_clock_secs.map(|v| v as i64),
                s.artifact_path, s.error_message, s.created_at, s.updated_at
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn step_get(&self, id: &StepExecutionId) -> Result<Option<StepExecution>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id,feature_id,step_id,step_index,step_kind,status,cost_usd,wall_clock_secs,artifact_path,error_message,created_at,updated_at
                 FROM step_executions WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
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
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(s)) => Ok(Some(s)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn step_update(&self, id: &StepExecutionId, patch: &StepExecutionPatch) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let conn = self.conn.lock()?;
        if let Some(status) = &patch.status {
            conn.execute(
                "UPDATE step_executions SET status=?2,cost_usd=?3,wall_clock_secs=?4,artifact_path=?5,error_message=?6,updated_at=?7 WHERE id=?1",
                params![
                    id.0, status,
                    patch.cost_usd.flatten(),
                    patch.wall_clock_secs.flatten().map(|v| v as i64),
                    patch.artifact_path.clone().flatten(),
                    patch.error_message.clone().flatten(),
                    now
                ],
            ).map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE step_executions SET cost_usd=?2,wall_clock_secs=?3,artifact_path=?4,error_message=?5,updated_at=?6 WHERE id=?1",
                params![
                    id.0,
                    patch.cost_usd.flatten(),
                    patch.wall_clock_secs.flatten().map(|v| v as i64),
                    patch.artifact_path.clone().flatten(),
                    patch.error_message.clone().flatten(),
                    now
                ],
            ).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn steps_for_feature(&self, feature_id: &FeatureId) -> Result<Vec<StepExecution>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id,feature_id,step_id,step_index,step_kind,status,cost_usd,wall_clock_secs,artifact_path,error_message,created_at,updated_at
                 FROM step_executions WHERE feature_id=?1 ORDER BY step_index ASC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![feature_id.0], |row| {
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
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }
}
