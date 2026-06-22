use rusqlite::params;

use crate::domain::ids::{ProjectId, WorkflowId};
use crate::domain::models::{Workflow, WorkflowSchedule, WorkflowVersion};
use crate::ports::db::WorkflowRepository;

use super::super::SqliteAdapter;

impl WorkflowRepository for SqliteAdapter {
    fn get(&self, id: &WorkflowId) -> Result<Option<Workflow>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, is_starter, created_at, updated_at,
                        schedule_cron, schedule_title_template, schedule_next_run_at, schedule_project_id
                 FROM workflows WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
                let cron: Option<String> = row.get(6)?;
                let title_template: Option<String> = row.get(7)?;
                let next_run_at: Option<i64> = row.get(8)?;
                let project_id_str: Option<String> = row.get(9)?;

                let schedule = if let (Some(cron), Some(title_template), Some(p_id)) =
                    (cron, title_template, project_id_str)
                {
                    Some(WorkflowSchedule {
                        cron,
                        title_template,
                        project_id: ProjectId(p_id),
                        next_run_at,
                    })
                } else {
                    None
                };

                Ok(Workflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    is_starter: row.get::<_, i32>(3)? != 0,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    schedule,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(w)) => Ok(Some(w)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn list(&self) -> Result<Vec<Workflow>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, is_starter, created_at, updated_at,
                        schedule_cron, schedule_title_template, schedule_next_run_at, schedule_project_id
                 FROM workflows ORDER BY is_starter DESC, created_at ASC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map([], |row| {
                let cron: Option<String> = row.get(6)?;
                let title_template: Option<String> = row.get(7)?;
                let next_run_at: Option<i64> = row.get(8)?;
                let project_id_str: Option<String> = row.get(9)?;

                let schedule = if let (Some(cron), Some(title_template), Some(p_id)) =
                    (cron, title_template, project_id_str)
                {
                    Some(WorkflowSchedule {
                        cron,
                        title_template,
                        project_id: ProjectId(p_id),
                        next_run_at,
                    })
                } else {
                    None
                };

                Ok(Workflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    is_starter: row.get::<_, i32>(3)? != 0,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    schedule,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn create(&self, w: Workflow) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let (cron, title_template, next_run_at, project_id) = if let Some(ref s) = w.schedule {
            (
                Some(s.cron.clone()),
                Some(s.title_template.clone()),
                s.next_run_at,
                Some(s.project_id.0.clone()),
            )
        } else {
            (None, None, None, None)
        };
        conn.execute(
            "INSERT INTO workflows (id, name, description, is_starter, created_at, updated_at,
                                    schedule_cron, schedule_title_template, schedule_next_run_at, schedule_project_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                w.id,
                w.name,
                w.description,
                w.is_starter as i32,
                w.created_at,
                w.updated_at,
                cron,
                title_template,
                next_run_at,
                project_id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_meta(&self, id: &WorkflowId, name: &str, description: &str) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE workflows SET name=?2, description=?3, updated_at=?4 WHERE id=?1",
            params![id.0, name, description, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete(&self, id: &WorkflowId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let is_starter: i32 = conn
            .query_row(
                "SELECT is_starter FROM workflows WHERE id=?1",
                params![id.0],
                |r| r.get(0),
            )
            .map_err(|_| "Workflow not found".to_string())?;
        if is_starter == 1 {
            return Err(
                "Cannot delete a starter pack workflow. Use 'Revert to Default' instead."
                    .to_string(),
            );
        }
        conn.execute("DELETE FROM workflows WHERE id=?1", params![id.0])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn save_version(&self, v: WorkflowVersion) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO workflow_versions (id,workflow_id,version,steps_json,note,created_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                v.id,
                v.workflow_id,
                v.version,
                v.steps_json,
                v.note,
                v.created_at
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn latest_version(&self, workflow_id: &WorkflowId) -> Result<Option<WorkflowVersion>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id,workflow_id,version,steps_json,note,created_at
                 FROM workflow_versions WHERE workflow_id=?1
                 ORDER BY version DESC LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![workflow_id.0], |row| {
                Ok(WorkflowVersion {
                    id: row.get(0)?,
                    workflow_id: row.get(1)?,
                    version: row.get::<_, u32>(2)?,
                    steps_json: row.get(3)?,
                    note: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(v)) => Ok(Some(v)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn versions(&self, workflow_id: &WorkflowId) -> Result<Vec<WorkflowVersion>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id,workflow_id,version,steps_json,note,created_at
                 FROM workflow_versions WHERE workflow_id=?1 ORDER BY version ASC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![workflow_id.0], |row| {
                Ok(WorkflowVersion {
                    id: row.get(0)?,
                    workflow_id: row.get(1)?,
                    version: row.get::<_, u32>(2)?,
                    steps_json: row.get(3)?,
                    note: row.get(4)?,
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

    fn count(&self) -> Result<u32, String> {
        let conn = self.conn.lock()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM workflows", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count as u32)
    }

    fn update_schedule(
        &self,
        id: &WorkflowId,
        schedule: Option<WorkflowSchedule>,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let (cron, title_template, next_run_at, project_id) = if let Some(ref s) = schedule {
            (
                Some(s.cron.clone()),
                Some(s.title_template.clone()),
                s.next_run_at,
                Some(s.project_id.0.clone()),
            )
        } else {
            (None, None, None, None)
        };
        conn.execute(
            "UPDATE workflows
             SET schedule_cron=?2, schedule_title_template=?3, schedule_next_run_at=?4, schedule_project_id=?5
             WHERE id=?1",
            params![id.0, cron, title_template, next_run_at, project_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_schedule_next_run(
        &self,
        id: &WorkflowId,
        next_run_at: Option<i64>,
    ) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE workflows SET schedule_next_run_at=?2 WHERE id=?1",
            params![id.0, next_run_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list_scheduled(&self) -> Result<Vec<Workflow>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, is_starter, created_at, updated_at,
                        schedule_cron, schedule_title_template, schedule_next_run_at, schedule_project_id
                 FROM workflows
                 WHERE schedule_cron IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map([], |row| {
                let cron: Option<String> = row.get(6)?;
                let title_template: Option<String> = row.get(7)?;
                let next_run_at: Option<i64> = row.get(8)?;
                let project_id_str: Option<String> = row.get(9)?;

                let schedule = if let (Some(cron), Some(title_template), Some(p_id)) =
                    (cron, title_template, project_id_str)
                {
                    Some(WorkflowSchedule {
                        cron,
                        title_template,
                        project_id: ProjectId(p_id),
                        next_run_at,
                    })
                } else {
                    None
                };

                Ok(Workflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    is_starter: row.get::<_, i32>(3)? != 0,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    schedule,
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
