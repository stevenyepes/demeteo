//! Hexagonal port for the "create a project from zero" wizard.
//!
//! The wizard collects seven one-decision-per-screen inputs (Name →
//! Provider → Group → Machine → Agent → Model → Description) and then
//! commits by:
//!
//! 1. delegating to `ProviderHttpPort::create_repo` to create the
//!    repository on the provider the user picked (no shell-out,
//!    no `gh`/`glab` argv construction);
//! 2. inserting the `projects` + `repositories` rows through the
//!    existing `ProjectRepository` (so the bootstrap step has a
//!    project to clone);
//! 3. enqueuing a standard `Feature` against the
//!    `wf-starter-standard` workflow, with the user-supplied
//!    description as the prompt body.
//!
//! The port is intentionally narrow: each method does one thing and
//! is independently testable. The wizard command layer is the only
//! place that calls them in sequence — see
//! `commands::create_project::submit_create_project_step`.

use async_trait::async_trait;
use std::path::PathBuf;

use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::Project;
use crate::error::AppError;
use crate::ports::provider_http::{CreatedRepo, NamespaceSummary};

/// One user-entered value, after slug validation. Carries the
/// canonical (lower-case, no leading/trailing whitespace) form so
/// downstream commands don't have to re-normalise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedName(pub String);

impl ValidatedName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ValidatedName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Slug validation rules. Mirrors the wizard's `validateSlug` helper
/// on the React side and the GitHub / GitLab repository-naming rules
/// (lowercase, alphanumeric, hyphens, dots, underscores; must start
/// with an alphanumeric; 1–100 chars).
pub const SLUG_PATTERN: &str = r"^[a-z0-9][a-z0-9._-]{0,99}$";

/// Compact view of a successfully-launched feature, returned to the
/// frontend so it can navigate to the `Detail` view.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LaunchedFeature {
    pub feature_id: String,
    pub feature_title: String,
    pub project_id: String,
    pub created_repo: CreatedRepo,
}

/// Hexagonal port for the create-from-zero wizard. The adapter lives
/// in `adapters::create_project_adapter`; the command layer in
/// `commands::create_project` is the only direct caller.
#[async_trait]
pub trait CreateProjectPort: Send + Sync {
    /// Validate a user-entered repo slug against the GitHub/GitLab
    /// naming rules. Empty / too-short / disallowed-character inputs
    /// surface as `AppError::Validation`; the caller is expected to
    /// show the message inline on the Name step and **not** advance
    /// the wizard.
    fn validate_name(&self, name: &str) -> Result<ValidatedName, AppError>;

    /// Resolve the absolute target directory the project will live in
    /// for a given workspace root + project id + repo slug. The wizard
    /// uses this to display the planned path on the Description step
    /// before the user commits.
    ///
    /// Path scheme mirrors `paths::project_root_local`:
    ///   `<workspace_dir>/projects/<project_id>/repos/<repo_name>`
    fn resolve_target_path(
        &self,
        workspace_dir: &std::path::Path,
        project_id: &str,
        repo_name: &str,
    ) -> Result<PathBuf, AppError>;

    /// Create the repository on the connected provider by delegating
    /// to `ProviderHttpPort::create_repo`. No shell command is
    /// constructed at any point — `namespace.id`, `name`, and the
    /// resolved `host` are all passed straight into URL / JSON-body
    /// fields where shell metacharacters have no special meaning.
    ///
    /// - `provider_kind` is `"github"` or `"gitlab"`.
    /// - `host` is the empty string when the wizard targets the
    ///   provider's public default (api.github.com / gitlab.com),
    ///   or a fully-qualified hostname for self-hosted enterprise
    ///   installs. `provider_http.api_base` interprets the empty
    ///   string as "public default".
    /// - `pat` is the keyring-resolved PAT for the connected
    ///   provider. **The PAT never crosses the IPC boundary** — the
    ///   command layer resolves it through
    ///   `credential_cache::get_or_fetch` and forwards it as an
    ///   `&str`.
    /// - `namespace` is the namespace summary the user picked on the
    ///   Group step (`"personal"` for the user's own account, an
    ///   org login, or a numeric GitLab group id).
    /// - `name` is the validated slug.
    /// - `visibility` is `"private"` / `"public"`; the adapter maps
    ///   it to the provider-specific `private: bool` (private is the
    ///   documented default for unknown / empty values).
    ///
    /// `auto_init: true` is always set so the repo has a default
    /// branch + first commit before clone (per the empty-repo
    /// bootstrap tolerance constraint).
    async fn create_remote_repo(
        &self,
        provider_kind: &str,
        host: &str,
        pat: &str,
        namespace: &NamespaceSummary,
        name: &str,
        visibility: &str,
    ) -> Result<CreatedRepo, AppError>;

    /// Insert a `Project` row + its `Repository` row, both with the
    /// freshly-created provider repo's `full_name` (or
    /// `path_with_namespace` for GitLab) as `repo_path`. The project
    /// is left at `status = "bootstrapping"`; the caller is expected
    /// to run `bootstrap_project` and `save_project_settings` after
    /// this returns.
    #[allow(clippy::too_many_arguments)]
    async fn persist_project(
        &self,
        projects: &dyn crate::ports::db::ProjectRepository,
        project_id: ProjectId,
        project_name: &str,
        compute_type: &str,
        remote_host: Option<MachineId>,
        repository_id: RepositoryId,
        provider_id: ProviderId,
        repo_path: &str,
    ) -> Result<Project, AppError>;

    /// Enqueue a standard feature against the freshly-bootstrapped
    /// project, using the `wf-starter-standard` workflow and the
    /// user-typed description as the prompt body. Returns the launched
    /// feature so the wizard can navigate to its `Detail` view.
    ///
    /// `agent_kind` / `model` are the user's selections from the Agent
    /// and Model steps. Either can be `None` to inherit the project's
    /// defaults (or the workflow's defaults when no project default
    /// is set).
    async fn dispatch_start_feature(
        &self,
        executor: &dyn crate::ports::step_executor::StepExecutor,
        project_id: &ProjectId,
        title: &str,
        description: &str,
        agent_kind: Option<&str>,
        model: Option<&str>,
    ) -> Result<LaunchedFeature, AppError>;
}

#[cfg(test)]
#[path = "../../tests/ports/create_project_port.rs"]
mod tests;
