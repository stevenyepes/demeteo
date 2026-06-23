use crate::error::AppError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// User info fetched from a provider (GitHub/GitLab) validation endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUserInfo {
    pub username: String,
    pub avatar_url: String,
}

/// A simplified repository description returned from a provider list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSummary {
    pub full_name: String,
}

/// Hexagonal port for making external HTTP requests to provider APIs.
#[async_trait]
pub trait ProviderHttpPort: Send + Sync {
    /// Validates a Personal Access Token (PAT) for a given provider host.
    async fn validate_pat(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
    ) -> Result<ProviderUserInfo, AppError>;

    /// Lists the repositories accessible by a given PAT.
    async fn list_repos(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
    ) -> Result<Vec<RepoSummary>, AppError>;
}
