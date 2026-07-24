use rusqlite::params;

use crate::domain::attachment::AttachedFile;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, WorkflowId};
use crate::domain::models::{EffortLevel, Feature, StepExecution};
use crate::ports::attachment_store::AttachmentJsonPort;
use crate::ports::db::{FeaturePatch, FeatureRepository, StepExecutionPatch};

use super::super::SqliteAdapter;

/// Read the `effort` column. An unknown/stale string degrades to `None`
/// (inherit) rather than failing the whole row — see [`EffortLevel::parse`].
fn effort_from_row(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<Option<EffortLevel>> {
    let raw: Option<String> = row.get(idx)?;
    Ok(raw.as_deref().and_then(EffortLevel::parse))
}

impl AttachmentJsonPort for SqliteAdapter {
    fn get_attachments(&self, feature_id: &FeatureId) -> Result<Vec<AttachedFile>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare("SELECT attachments_json FROM features WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query(params![feature_id.0])
            .map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => {
                let json: Option<String> = row.get(0).map_err(|e| e.to_string())?;
                Ok(json
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default())
            }
            None => Ok(Vec::new()),
        }
    }

    fn set_attachments(
        &self,
        feature_id: &FeatureId,
        attachments: &[AttachedFile],
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let json: Option<String> = if attachments.is_empty() {
            None
        } else {
            Some(serde_json::to_string(attachments).map_err(|e| e.to_string())?)
        };
        conn.execute(
            "UPDATE features SET attachments_json = ?2 WHERE id = ?1",
            params![feature_id.0, json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl FeatureRepository for SqliteAdapter {
    fn get_active(&self, project_id: &ProjectId) -> Result<Vec<Feature>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state, commit_artifacts, loop_iterations, step_overrides_json, attachments_json, description, pr_title, pr_body, effort, max_budget_usd, workflow_version_id
                 FROM features WHERE project_id = ?1 AND status NOT IN ('archived', 'deleted') ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id.0], |row| {
                let commit_artifacts: Option<i64> = row.get(13)?;
                let loop_iterations: Option<i64> = row.get(14)?;
                let step_overrides_json: Option<String> = row.get(15)?;
                let attachments_json: Option<String> = row.get(16)?;
                let attachments: Vec<AttachedFile> = attachments_json
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                Ok(Feature {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    workflow_id: row.get(2)?,
                    workflow_version_id: row.get(22)?,
                    title: row.get(3)?,
                    description: row.get(17)?,
                    status: row.get(4)?,
                    total_cost: row.get(5)?,
                    duration: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                    agent_kind: row.get(9)?,
                    model: row.get(10)?,
                    effort: effort_from_row(row, 20)?,
                    mr_url: row.get(11)?,
                    mr_state: row.get(12)?,
                    pr_title: row.get(18)?,
                    pr_body: row.get(19)?,
                    commit_artifacts: commit_artifacts.map(|v| v != 0),
                    loop_iterations: loop_iterations.map(|v| v as u32),
                    max_budget_usd: row.get(21)?,
                    step_overrides: step_overrides_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    attachments,
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
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state, commit_artifacts, loop_iterations, step_overrides_json, attachments_json, description, pr_title, pr_body, effort, max_budget_usd, workflow_version_id
                 FROM features WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
                let commit_artifacts: Option<i64> = row.get(13)?;
                let loop_iterations: Option<i64> = row.get(14)?;
                let step_overrides_json: Option<String> = row.get(15)?;
                let attachments_json: Option<String> = row.get(16)?;
                let attachments: Vec<AttachedFile> = attachments_json
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                Ok(Feature {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    workflow_id: row.get(2)?,
                    workflow_version_id: row.get(22)?,
                    title: row.get(3)?,
                    description: row.get(17)?,
                    status: row.get(4)?,
                    total_cost: row.get(5)?,
                    duration: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                    agent_kind: row.get(9)?,
                    model: row.get(10)?,
                    effort: effort_from_row(row, 20)?,
                    mr_url: row.get(11)?,
                    mr_state: row.get(12)?,
                    pr_title: row.get(18)?,
                    pr_body: row.get(19)?,
                    commit_artifacts: commit_artifacts.map(|v| v != 0),
                    loop_iterations: loop_iterations.map(|v| v as u32),
                    max_budget_usd: row.get(21)?,
                    step_overrides: step_overrides_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    attachments,
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
        let commit_artifacts: Option<i64> = f.commit_artifacts.map(|v| if v { 1 } else { 0 });
        let loop_iterations: Option<i64> = f.loop_iterations.map(|v| v as i64);
        let step_overrides_json: Option<String> = if f.step_overrides.is_empty() {
            None
        } else {
            serde_json::to_string(&f.step_overrides).ok()
        };
        let attachments_json: Option<String> = if f.attachments.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&f.attachments).map_err(|e| e.to_string())?)
        };
        conn.execute(
            "INSERT INTO features (id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state, commit_artifacts, loop_iterations, step_overrides_json, attachments_json, description, effort, max_budget_usd, workflow_version_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            params![
                f.id, f.project_id, f.workflow_id, f.title, f.status,
                f.total_cost, f.duration, f.tokens, f.created_at, f.agent_kind, f.model,
                f.mr_url, f.mr_state, commit_artifacts, loop_iterations, step_overrides_json,
                attachments_json, f.description, f.effort.map(|e| e.as_str()), f.max_budget_usd,
                f.workflow_version_id
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
        let commit_artifacts: Option<Option<bool>> = patch.commit_artifacts;

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
        if let Some(e) = patch.effort {
            sets.push("effort=?");
            // `Some(None)` clears the pin back to "inherit" (SQL NULL).
            binds.push(Box::new(e.map(|v| v.as_str())));
        }
        if let Some(url) = mr_url {
            sets.push("mr_url=?");
            binds.push(Box::new(url));
        }
        if let Some(state) = mr_state {
            sets.push("mr_state=?");
            binds.push(Box::new(state));
        }
        if let Some(ca) = commit_artifacts {
            sets.push("commit_artifacts=?");
            // Mirror `add`: bool → 0/1, `None` (inherit) → SQL NULL.
            binds.push(Box::new(ca.map(|v| if v { 1i64 } else { 0i64 })));
        }
        if let Some(t) = patch.pr_title.clone() {
            sets.push("pr_title=?");
            binds.push(Box::new(t));
        }
        if let Some(b) = patch.pr_body.clone() {
            sets.push("pr_body=?");
            binds.push(Box::new(b));
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

    fn pin_workflow_version(
        &self,
        id: &FeatureId,
        version_id: &crate::domain::ids::WorkflowVersionId,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        // Pin-once: an already-pinned feature keeps its version — the pin
        // is what guarantees a running graph can never change under a run.
        conn.execute(
            "UPDATE features SET workflow_version_id = ?2
             WHERE id = ?1 AND workflow_version_id IS NULL",
            params![id.0, version_id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list_with_open_mr(&self) -> Result<Vec<Feature>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state, commit_artifacts, loop_iterations, step_overrides_json, attachments_json, description, pr_title, pr_body, effort, max_budget_usd, workflow_version_id
                 FROM features WHERE mr_state = 'open' AND mr_url IS NOT NULL ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map([], |row| {
                let commit_artifacts: Option<i64> = row.get(13)?;
                let loop_iterations: Option<i64> = row.get(14)?;
                let step_overrides_json: Option<String> = row.get(15)?;
                let attachments_json: Option<String> = row.get(16)?;
                let attachments: Vec<AttachedFile> = attachments_json
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                Ok(Feature {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    workflow_id: row.get(2)?,
                    workflow_version_id: row.get(22)?,
                    title: row.get(3)?,
                    description: row.get(17)?,
                    status: row.get(4)?,
                    total_cost: row.get(5)?,
                    duration: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                    agent_kind: row.get(9)?,
                    model: row.get(10)?,
                    effort: effort_from_row(row, 20)?,
                    mr_url: row.get(11)?,
                    mr_state: row.get(12)?,
                    pr_title: row.get(18)?,
                    pr_body: row.get(19)?,
                    commit_artifacts: commit_artifacts.map(|v| v != 0),
                    loop_iterations: loop_iterations.map(|v| v as u32),
                    max_budget_usd: row.get(21)?,
                    step_overrides: step_overrides_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    attachments,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
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

    fn attempt_open(
        &self,
        step_execution_id: &StepExecutionId,
        now: i64,
        workspace_fingerprint: Option<&str>,
    ) -> Result<u32, String> {
        super::step_attempts::attempt_open(self, step_execution_id, now, workspace_fingerprint)
    }

    fn attempt_close(
        &self,
        step_execution_id: &StepExecutionId,
        attempt_no: u32,
        status: &str,
        cost_usd: f64,
        tokens: i64,
        wall_clock_ms: u64,
        error_class: Option<&str>,
        failure_fingerprint: Option<&str>,
        applied_rule: Option<&str>,
        now: i64,
    ) -> Result<(), String> {
        super::step_attempts::attempt_close(
            self,
            step_execution_id,
            attempt_no,
            status,
            cost_usd,
            tokens,
            wall_clock_ms,
            error_class,
            failure_fingerprint,
            applied_rule,
            now,
        )
    }

    fn attempts_for_step(
        &self,
        step_execution_id: &StepExecutionId,
    ) -> Result<Vec<crate::domain::models::StepAttempt>, String> {
        super::step_attempts::attempts_for_step(self, step_execution_id)
    }

    fn sequence_checkpoint_get(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<Vec<String>, String> {
        super::sequence_state::sequence_checkpoint_get(self, feature_id, step_id)
    }

    fn sequence_checkpoint_record(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
        landed_task_ids: &[String],
        now: i64,
    ) -> Result<u32, String> {
        super::sequence_state::sequence_checkpoint_record(
            self,
            feature_id,
            step_id,
            landed_task_ids,
            now,
        )
    }

    fn sequence_checkpoint_clear(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<(), String> {
        super::sequence_state::sequence_checkpoint_clear(self, feature_id, step_id)
    }

    fn plan_cache_get(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<Option<String>, String> {
        super::sequence_state::plan_cache_get(self, feature_id, step_id)
    }

    fn plan_cache_put(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
        plan_json: &str,
        attempt_no: Option<u32>,
        now: i64,
    ) -> Result<(), String> {
        super::sequence_state::plan_cache_put(self, feature_id, step_id, plan_json, attempt_no, now)
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/feature.rs"]
mod tests;
