use crate::domain::ids::ProviderId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProviderInstance {
    pub id: ProviderId,
    pub kind: String, // 'github' | 'gitlab'
    pub host: String,
    pub username: String,
    pub avatar_url: String,
    pub created_at: i64,
}
