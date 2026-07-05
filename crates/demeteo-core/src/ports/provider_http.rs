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

/// A namespace (personal account, GitHub org, or GitLab group/subgroup)
/// a repo can be created under.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceSummary {
    /// GitHub: org login or the user login for personal.
    /// GitLab: numeric namespace/group id as a string.
    pub id: String,
    /// Human-facing label (org name / group full_path / "Personal").
    pub name: String,
    /// One of: "personal" | "org" | "group".
    pub kind: String,
}

/// Request describing a repository to create on a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRepoRequest {
    /// The namespace the repo is created under (personal / org / group).
    pub namespace: NamespaceSummary,
    /// Validated repo slug.
    pub name: String,
    /// Whether the repo should be private.
    pub private: bool,
    /// When true the repo is seeded with an initial commit / README so a
    /// default branch exists before clone (GitHub `auto_init`, GitLab
    /// `initialize_with_readme`).
    pub auto_init: bool,
}

/// Result of creating a repository on a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedRepo {
    /// GitHub `full_name` or GitLab `path_with_namespace`, e.g. "org/slug".
    pub full_name: String,
    /// The repo's default branch as reported by the provider (e.g. "main").
    pub default_branch: String,
    /// HTTPS clone url (informational; clone still routes through git ops).
    pub clone_url: String,
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

    /// Lists the namespaces (personal account + orgs/groups) a repo can be
    /// created under for a given PAT.
    async fn list_namespaces(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
    ) -> Result<Vec<NamespaceSummary>, AppError>;

    /// Creates a new repository on the provider under the requested
    /// namespace and returns its canonical name / default branch / clone url.
    async fn create_repo(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
        req: &CreateRepoRequest,
    ) -> Result<CreatedRepo, AppError>;
}
