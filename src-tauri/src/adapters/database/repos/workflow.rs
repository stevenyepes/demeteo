use rusqlite::params;

use crate::domain::ids::WorkflowId;
use crate::domain::models::{Workflow, WorkflowVersion};
use crate::ports::db::WorkflowRepository;

use super::super::SqliteAdapter;

impl WorkflowRepository for SqliteAdapter {
    fn get(&self, id: &WorkflowId) -> Result<Option<Workflow>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, is_starter, created_at, updated_at
                 FROM workflows WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
                Ok(Workflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    is_starter: row.get::<_, i32>(3)? != 0,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
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
                "SELECT id, name, description, is_starter, created_at, updated_at
                 FROM workflows ORDER BY is_starter DESC, created_at ASC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map([], |row| {
                Ok(Workflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    is_starter: row.get::<_, i32>(3)? != 0,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
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
        conn.execute(
            "INSERT INTO workflows (id, name, description, is_starter, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![w.id, w.name, w.description, w.is_starter as i32, w.created_at, w.updated_at],
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
            params![v.id, v.workflow_id, v.version, v.steps_json, v.note, v.created_at],
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
}
