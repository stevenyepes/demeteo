use crate::domain::ids::ProjectId;
use crate::domain::memory::ProjectMemoryEntry;

pub trait ProjectMemoryPort: Send + Sync {
    fn memory_upsert(&self, entry: ProjectMemoryEntry) -> Result<(), String>;
    fn memory_list(
        &self,
        project_id: &ProjectId,
        limit: usize,
    ) -> Result<Vec<ProjectMemoryEntry>, String>;
    fn memory_delete(&self, id: &str) -> Result<(), String>;
    /// Bump `use_count` and set `last_used_at` for the given memory ids. Used by
    /// semantic retrieval to track which memories actually get injected.
    fn memory_mark_used(&self, ids: &[String], now: i64) -> Result<(), String>;
}
