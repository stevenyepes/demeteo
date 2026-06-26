use rusqlite::params;

use crate::domain::ids::ProjectId;
use crate::domain::memory::{
    blob_to_embedding, embedding_to_blob, MemorySource, MemoryType, ProjectMemoryEntry,
};
use crate::ports::memory::ProjectMemoryPort;

use super::super::SqliteAdapter;

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<ProjectMemoryEntry> {
    let source_str: String = row.get(4)?;
    let memory_type: Option<String> = row.get(6)?;
    let embedding_blob: Option<Vec<u8>> = row.get(8)?;
    Ok(ProjectMemoryEntry {
        id: row.get(0)?,
        project_id: row.get(1)?,
        key: row.get(2)?,
        value: row.get(3)?,
        source: MemorySource::from_str(&source_str),
        confidence: row.get(5)?,
        memory_type: memory_type.as_deref().and_then(MemoryType::from_str),
        statement: row.get(7)?,
        embedding: embedding_blob.map(|b| blob_to_embedding(&b)),
        embedding_model: row.get(9)?,
        last_used_at: row.get(10)?,
        use_count: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

const SELECT_COLS: &str = "id, project_id, key, value, source, confidence, memory_type, \
     statement, embedding, embedding_model, last_used_at, use_count, created_at, updated_at";

impl ProjectMemoryPort for SqliteAdapter {
    fn memory_upsert(&self, entry: ProjectMemoryEntry) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let embedding_blob = entry.embedding.as_ref().map(|e| embedding_to_blob(e));
        conn.execute(
            "INSERT OR REPLACE INTO project_memory
                (id, project_id, key, value, source, confidence, memory_type, statement,
                 embedding, embedding_model, last_used_at, use_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                entry.id,
                entry.project_id,
                entry.key,
                entry.value,
                entry.source.as_str(),
                entry.confidence,
                entry.memory_type.map(|t| t.as_str()),
                entry.statement,
                embedding_blob,
                entry.embedding_model,
                entry.last_used_at,
                entry.use_count,
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
        let sql = format!(
            "SELECT {SELECT_COLS}
                 FROM project_memory
                 WHERE project_id = ?1
                 ORDER BY confidence DESC, updated_at DESC
                 LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id, limit], row_to_entry)
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

    fn memory_mark_used(&self, ids: &[String], now: i64) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock()?;
        for id in ids {
            conn.execute(
                "UPDATE project_memory
                 SET use_count = use_count + 1, last_used_at = ?2
                 WHERE id = ?1",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
