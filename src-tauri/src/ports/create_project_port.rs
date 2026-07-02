//! Hexagonal port for the "create a project from zero" wizard.
//!
//! The wizard collects seven one-decision-per-screen inputs (Name →
//! Provider → Group → Machine → Agent → Model → Description) and then
//! commits by:
//!
//! 1. shelling out to `gh repo create` / `glab project create` to
//!    create the repository on the provider the user picked;
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

    /// Shell out to `gh repo create` or `glab project create` on the
    /// local host, returning the canonical name / default branch /
    /// clone URL of the newly-created repo. The adapter resolves the
    /// provider kind from the wizard's Provider step input.
    ///
    /// The PAT is **not** passed by the frontend — the adapter looks
    /// it up from the keyring via the existing
    /// `credential_cache::get_or_fetch` path before invoking the
    /// CLI. `gh` / `glab` itself reads the token from its own
    /// authentication store, so we don't need to inject it.
    async fn create_remote_repo(
        &self,
        provider_kind: &str,
        host: &str,
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
mod tests {
    use super::*;

    /// Trivial in-memory port used only to exercise the *pure* methods
    /// (validate_name, resolve_target_path) without needing a live DB
    /// or CLI. The async methods are stubbed to return
    /// `AppError::internal` and are not exercised here — their real
    /// behaviour is covered by the adapter's integration tests.
    struct StubPort;

    #[async_trait]
    impl CreateProjectPort for StubPort {
        fn validate_name(&self, name: &str) -> Result<ValidatedName, AppError> {
            // Inline copy of the canonical rule: must match the React
            // wizard's `validateSlug`. Kept here so this test module
            // compiles without depending on the adapter.
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(AppError::validation("Repository name is required"));
            }
            if trimmed.len() < 2 {
                return Err(AppError::validation("Use at least 2 characters"));
            }
            let pat = regex_lite_match(trimmed);
            if !pat {
                return Err(AppError::validation(
                    "Use lowercase letters, digits, dots, dashes or underscores",
                ));
            }
            Ok(ValidatedName(trimmed.to_string()))
        }

        fn resolve_target_path(
            &self,
            workspace_dir: &std::path::Path,
            project_id: &str,
            repo_name: &str,
        ) -> Result<PathBuf, AppError> {
            let safe_id = sanitize_segment(project_id)?;
            let safe_name = sanitize_segment(repo_name)?;
            Ok(workspace_dir
                .join("projects")
                .join(safe_id)
                .join("repos")
                .join(safe_name))
        }

        async fn create_remote_repo(
            &self,
            _provider_kind: &str,
            _host: &str,
            _namespace: &NamespaceSummary,
            _name: &str,
            _visibility: &str,
        ) -> Result<CreatedRepo, AppError> {
            Err(AppError::internal("stub: create_remote_repo"))
        }

        async fn persist_project(
            &self,
            _projects: &dyn crate::ports::db::ProjectRepository,
            _project_id: ProjectId,
            _project_name: &str,
            _compute_type: &str,
            _remote_host: Option<MachineId>,
            _repository_id: RepositoryId,
            _provider_id: ProviderId,
            _repo_path: &str,
        ) -> Result<Project, AppError> {
            Err(AppError::internal("stub: persist_project"))
        }

        async fn dispatch_start_feature(
            &self,
            _executor: &dyn crate::ports::step_executor::StepExecutor,
            _project_id: &ProjectId,
            _title: &str,
            _description: &str,
            _agent_kind: Option<&str>,
            _model: Option<&str>,
        ) -> Result<LaunchedFeature, AppError> {
            Err(AppError::internal("stub: dispatch_start_feature"))
        }
    }

    /// Tiny regex-subset matcher so the port tests don't have to
    /// pull in the full `regex` crate just for one pattern. The
    /// pattern is the same `^[a-z0-9][a-z0-9._-]{0,99}$` documented
    /// on `SLUG_PATTERN`.
    fn regex_lite_match(s: &str) -> bool {
        if s.is_empty() || s.len() > 100 {
            return false;
        }
        let bytes = s.as_bytes();
        let first = bytes[0];
        if !(first.is_ascii_digit() || first.is_ascii_lowercase()) {
            return false;
        }
        s.bytes().all(|b: u8| {
            b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'.' || b == b'_' || b == b'-'
        })
    }

    /// Reject path-separator characters so `resolve_target_path`
    /// can't be tricked into writing outside the workspace.
    fn sanitize_segment(seg: &str) -> Result<String, AppError> {
        if seg.is_empty() {
            return Err(AppError::validation("path segment is empty"));
        }
        if seg.contains('/') || seg.contains('\\') || seg.contains("..") {
            return Err(AppError::validation(format!(
                "path segment contains forbidden characters: {}",
                seg
            )));
        }
        Ok(seg.to_string())
    }

    #[test]
    fn validate_name_accepts_well_formed_slugs() {
        let p = StubPort;
        for ok in [
            "my-repo",
            "my.repo",
            "my_repo",
            "a1",
            "a1b2c3",
            "abc-123_def.0",
        ] {
            assert_eq!(
                p.validate_name(ok).unwrap().as_str(),
                ok,
                "should accept: {ok}"
            );
        }
    }

    #[test]
    fn validate_name_rejects_empty_and_short_inputs() {
        let p = StubPort;
        assert!(matches!(
            p.validate_name("").unwrap_err(),
            AppError::Validation { .. }
        ));
        assert!(matches!(
            p.validate_name("   ").unwrap_err(),
            AppError::Validation { .. }
        ));
        assert!(matches!(
            p.validate_name("a").unwrap_err(),
            AppError::Validation { .. }
        ));
    }

    #[test]
    fn validate_name_rejects_disallowed_characters() {
        let p = StubPort;
        for bad in [
            "My-Repo",         // uppercase
            "-leading",        // leading hyphen
            ".leading",        // leading dot
            "with space",      // space
            "with/slash",      // slash
            "emoji-\u{1F4A9}", // non-ASCII
        ] {
            assert!(p.validate_name(bad).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn validate_name_trims_surrounding_whitespace() {
        let p = StubPort;
        assert_eq!(p.validate_name("  my-repo  ").unwrap().as_str(), "my-repo");
    }

    #[test]
    fn resolve_target_path_builds_local_workspace_layout() {
        let p = StubPort;
        let ws = std::path::PathBuf::from("/tmp/demeteo");
        let got = p.resolve_target_path(&ws, "p_123", "my-repo").unwrap();
        assert_eq!(
            got,
            std::path::PathBuf::from("/tmp/demeteo/projects/p_123/repos/my-repo")
        );
    }

    #[test]
    fn resolve_target_path_rejects_path_traversal() {
        let p = StubPort;
        let ws = std::path::PathBuf::from("/tmp/demeteo");
        assert!(p.resolve_target_path(&ws, "../escape", "x").is_err());
        assert!(p.resolve_target_path(&ws, "ok", "../bad").is_err());
        assert!(p.resolve_target_path(&ws, "ok", "with/slash").is_err());
        assert!(p.resolve_target_path(&ws, "", "x").is_err());
    }
}
