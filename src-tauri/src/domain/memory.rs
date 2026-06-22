use crate::domain::ids::ProjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectMemoryEntry {
    pub id: String,
    pub project_id: ProjectId,
    pub key: String,
    pub value: String,
    pub source: MemorySource,
    pub confidence: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    Agent,
    Human,
}
