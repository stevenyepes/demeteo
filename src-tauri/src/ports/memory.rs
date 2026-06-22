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
}
