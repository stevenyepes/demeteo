use rusqlite::params;

use crate::domain::ids::ProjectId;
use crate::domain::memory::{MemorySource, ProjectMemoryEntry};
use crate::ports::memory::ProjectMemoryPort;

use super::super::SqliteAdapter;

impl ProjectMemoryPort for SqliteAdapter {
    fn memory_upsert(&self, entry: ProjectMemoryEntry) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let source_str = match entry.source {
            MemorySource::Agent => "agent",
            MemorySource::Human => "human",
        };
        conn.execute(
            "INSERT OR REPLACE INTO project_memory (id, project_id, key, value, source, confidence, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.id,
                entry.project_id,
                entry.key,
                entry.value,
                source_str,
                entry.confidence,
                entry.created_at,
                entry.updated_at,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn memory_list(
        &self,
        project_id: &ProjectId,
        limit: usize,
    ) -> Result<Vec<ProjectMemoryEntry>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, key, value, source, confidence, created_at, updated_at
                 FROM project_memory
                 WHERE project_id = ?1
                 ORDER BY confidence DESC, updated_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id, limit], |row| {
                let source_str: String = row.get(4)?;
                let source = match source_str.as_str() {
                    "agent" => MemorySource::Agent,
                    _ => MemorySource::Human,
                };
                Ok(ProjectMemoryEntry {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    key: row.get(2)?,
                    value: row.get(3)?,
                    source,
                    confidence: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn memory_delete(&self, id: &str) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM project_memory WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
