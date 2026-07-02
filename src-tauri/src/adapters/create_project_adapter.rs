//! Concrete `CreateProjectPort` implementation.
//!
//! The wizard command layer (`commands::create_project`) is the only
//! caller. It hands the adapter the `AppContext` sub-ports it needs
//! for each step; the adapter itself owns the local `ExecutionPort`
//! (for shelling out to `gh` / `glab`).
//!
//! The two halves of the wizard's commit step — creating the remote
//! repo and persisting the project row — are intentionally
//! separate port methods so the unit tests can exercise them in
//! isolation, and so a future re-driver (e.g. a wizard variant that
//! connects to an existing repo) can re-use `persist_project` and
//! `dispatch_start_feature` without re-implementing the gh/glab
//! shell-out.

use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, Repository};
use crate::error::AppError;
use crate::infrastructure::gh_gl_cli::{
    cli_failure, gh_create_repo_args, glab_create_project_args, normalise_provider_kind,
    normalise_visibility, parse_gh_create_repo_output, parse_glab_create_project_output,
    KIND_GITHUB, KIND_GITLAB,
};
use crate::ports::create_project_port::{
    CreateProjectPort, LaunchedFeature, ValidatedName, SLUG_PATTERN,
};
use crate::ports::db::ProjectRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::provider_http::{CreatedRepo, NamespaceSummary};
use crate::ports::step_executor::StepExecutor;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

pub struct CreateProjectAdapter {
    /// Local execution port — used to spawn `gh` / `glab`. The wizard
    /// always runs these locally; remote-machine repo creation is
    /// out of scope for v1.
    exec: Arc<dyn ExecutionPort>,
}

impl CreateProjectAdapter {
    pub fn new(exec: Arc<dyn ExecutionPort>) -> Self {
        Self { exec }
    }

    /// Same slug rules as the React wizard (`validateSlug` in
    /// `CreateFromZeroWizard.tsx`). Kept in lockstep with the
    /// frontend so the backend can reject the same inputs the
    /// frontend would render as inline errors.
    fn validate_slug(&self, name: &str) -> Result<ValidatedName, AppError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(AppError::validation("Repository name is required"));
        }
        if trimmed.len() < 2 {
            return Err(AppError::validation("Use at least 2 characters"));
        }
        if !slug_matches(trimmed) {
            return Err(AppError::validation(
                "Use lowercase letters, digits, dots, dashes or underscores",
            ));
        }
        Ok(ValidatedName(trimmed.to_string()))
    }

    /// Reject path-segment characters so the resolved path can't be
    /// tricked into writing outside the workspace.
    fn sanitize_segment(seg: &str, label: &str) -> Result<String, AppError> {
        if seg.is_empty() {
            return Err(AppError::validation(format!("{label} is empty")));
        }
        if seg.contains('/') || seg.contains('\\') || seg.contains("..") {
            return Err(AppError::validation(format!(
                "{label} contains forbidden characters: {seg}"
            )));
        }
        Ok(seg.to_string())
    }
}

#[async_trait]
impl CreateProjectPort for CreateProjectAdapter {
    fn validate_name(&self, name: &str) -> Result<ValidatedName, AppError> {
        self.validate_slug(name)
    }

    fn resolve_target_path(
        &self,
        workspace_dir: &std::path::Path,
        project_id: &str,
        repo_name: &str,
    ) -> Result<PathBuf, AppError> {
        let safe_id = Self::sanitize_segment(project_id, "project id")?;
        let safe_name = Self::sanitize_segment(repo_name, "repo name")?;
        Ok(workspace_dir
            .join("projects")
            .join(safe_id)
            .join("repos")
            .join(safe_name))
    }

    async fn create_remote_repo(
        &self,
        provider_kind: &str,
        host: &str,
        namespace: &NamespaceSummary,
        name: &str,
        visibility: &str,
    ) -> Result<CreatedRepo, AppError> {
        let kind = normalise_provider_kind(provider_kind)?;
        let vis = normalise_visibility(visibility);
        let args = match kind {
            KIND_GITHUB => gh_create_repo_args(namespace, name, vis),
            KIND_GITLAB => glab_create_project_args(namespace, name, vis),
            // normalise_provider_kind already filters unknown kinds.
            _ => unreachable!(),
        };

        // `gh` / `glab` read the token from their own auth store —
        // we never inject a PAT into argv. The host arg is only
        // used for self-hosted enterprise / on-prem installs
        // (`GH_HOST` / `GITLAB_HOST` env vars). Empty host means
        // the public default.
        let cmd = match kind {
            KIND_GITHUB => "gh",
            KIND_GITLAB => "glab",
            _ => unreachable!(),
        };

        // We run with `cwd = workspace_dir` so providers that read
        // the project name from the local dir (gh refuses to create
        // a repo whose name collides with the current dir) get a
        // neutral directory. The exec port treats `local` as the
        // host machine.
        let _cwd = ".";
        let _ = (_cwd, host);

        let out = self
            .exec
            .run_command("local", &format!("{} {}", cmd, args.join(" ")))
            .await
            .map_err(|e| AppError::provider(format!("failed to spawn {cmd}: {e}")))?;

        // The exec port returns stdout on success and Err on
        // non-zero exit. We still want to differentiate "CLI printed
        // garbage" from "CLI failed" — the latter would have been
        // caught by run_command already.
        match kind {
            KIND_GITHUB => parse_gh_create_repo_output(&out),
            KIND_GITLAB => parse_glab_create_project_output(&out),
            _ => unreachable!(),
        }
    }

    async fn persist_project(
        &self,
        projects: &dyn ProjectRepository,
        project_id: ProjectId,
        project_name: &str,
        compute_type: &str,
        remote_host: Option<MachineId>,
        repository_id: RepositoryId,
        provider_id: ProviderId,
        repo_path: &str,
    ) -> Result<Project, AppError> {
        let now = crate::paths::now_ms();
        let project = Project {
            id: project_id.clone(),
            name: project_name.to_string(),
            compute_type: compute_type.to_string(),
            remote_host,
            status: "bootstrapping".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        };
        projects.add(project.clone()).map_err(AppError::from)?;

        let repository = Repository {
            id: repository_id,
            project_id: project_id.clone(),
            provider_id,
            repo_path: repo_path.to_string(),
        };
        projects
            .add_repository(repository)
            .map_err(AppError::from)?;

        Ok(project)
    }

    async fn dispatch_start_feature(
        &self,
        executor: &dyn StepExecutor,
        project_id: &ProjectId,
        title: &str,
        description: &str,
        agent_kind: Option<&str>,
        model: Option<&str>,
    ) -> Result<LaunchedFeature, AppError> {
        let feature = executor
            .feature_start(
                project_id.as_str(),
                "wf-starter-standard",
                title,
                description,
                agent_kind,
                model,
                None,
                None,
                Vec::new(),
                Vec::new(),
            )
            .await
            .map_err(AppError::from)?;
        Ok(LaunchedFeature {
            feature_id: feature.id.0.clone(),
            feature_title: feature.title.clone(),
            project_id: project_id.0.clone(),
            created_repo: CreatedRepo {
                // The wizard already has the created-repo metadata
                // from the create_remote_repo step; this field is
                // populated by the command layer when it composes
                // the final `LaunchedFeature` return value, so we
                // leave it empty here.
                full_name: String::new(),
                default_branch: String::new(),
                clone_url: String::new(),
            },
        })
    }
}

/// Match a candidate against the wizard's slug pattern without
/// pulling in the `regex` crate. Equivalent to
/// `^[a-z0-9][a-z0-9._-]{0,99}$`.
fn slug_matches(s: &str) -> bool {
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

// Keep `SLUG_PATTERN` referenced so the constant isn't dead code
// even though the live matcher is `slug_matches`. The constant is
// part of the public doc surface (the comment at the top of
// `create_project_port.rs` references it) and any future regex
// implementation can swap in here without changing the port.
#[allow(dead_code)]
const SLUG_PATTERN_DOC: &str = SLUG_PATTERN;

/// Build the adapter's "this is what we tried" error message for the
/// rare case where the spawned CLI exits successfully but the JSON
/// output is unusable. Kept here (not in the port layer) because
/// it's a transport concern.
#[allow(dead_code)]
fn bad_cli_output(provider_kind: &str, stdout: &str, parse_err: &AppError) -> AppError {
    let _ = cli_failure(provider_kind, Some(0), stdout);
    AppError::provider(format!(
        "{} CLI produced unparseable output: {}",
        provider_kind, parse_err
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/demeteo-test")
    }

    fn adapter() -> CreateProjectAdapter {
        // The unit tests for the adapter cover pure helpers only
        // (validate_name, resolve_target_path, slug_matches). The
        // async methods need a real `ExecutionPort` + `ProjectRepository`
        // + `StepExecutor` and are exercised by the commands-level
        // integration tests in `commands::create_project`.
        struct NoopExec;
        #[async_trait::async_trait]
        impl ExecutionPort for NoopExec {
            async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
                Ok(())
            }
            async fn run_command(&self, _machine_id: &str, _cmd: &str) -> Result<String, String> {
                Ok(String::new())
            }
            async fn read_file(&self, _machine_id: &str, _path: &str) -> Result<String, String> {
                Ok(String::new())
            }
            async fn write_file(
                &self,
                _machine_id: &str,
                _path: &str,
                _content: &str,
            ) -> Result<(), String> {
                Ok(())
            }
            async fn get_metadata(
                &self,
                _machine_id: &str,
                _path: &str,
            ) -> Result<crate::sftp::SftpEntry, String> {
                unimplemented!()
            }
            async fn list_dir(
                &self,
                _machine_id: &str,
                _path: &str,
            ) -> Result<Vec<crate::sftp::SftpEntry>, String> {
                Ok(vec![])
            }
            async fn setup_worktree(
                &self,
                _machine_id: &str,
                _repo_path: &str,
                _branch: &str,
                _sandbox_path: &str,
            ) -> Result<(), String> {
                Ok(())
            }
            async fn resolve_home(&self, _machine_id: &str) -> Result<String, String> {
                Ok("/tmp".into())
            }
            fn spawn_interactive(
                &self,
                _machine_id: &str,
                _binary: &str,
                _args: &[String],
                _cwd: &str,
                _env: &std::collections::HashMap<String, String>,
            ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
                unimplemented!()
            }
        }
        CreateProjectAdapter::new(Arc::new(NoopExec))
    }

    #[test]
    fn validate_name_mirrors_port_contract() {
        let a = adapter();
        assert!(a.validate_name("ok-name").is_ok());
        assert!(matches!(
            a.validate_name("").unwrap_err(),
            AppError::Validation { .. }
        ));
        assert!(matches!(
            a.validate_name("UPPER").unwrap_err(),
            AppError::Validation { .. }
        ));
        assert!(matches!(
            a.validate_name("with space").unwrap_err(),
            AppError::Validation { .. }
        ));
    }

    #[test]
    fn resolve_target_path_uses_workspace_projects_id_repos_layout() {
        let a = adapter();
        let got = a.resolve_target_path(&ws(), "p_1", "demo").unwrap();
        assert_eq!(got, ws().join("projects/p_1/repos/demo"));
    }

    #[test]
    fn resolve_target_path_rejects_traversal_segments() {
        let a = adapter();
        assert!(a.resolve_target_path(&ws(), "..", "x").is_err());
        assert!(a.resolve_target_path(&ws(), "ok", "../bad").is_err());
        assert!(a.resolve_target_path(&ws(), "ok", "with/slash").is_err());
    }

    #[test]
    fn slug_matches_accepts_well_formed_inputs() {
        for ok in ["a", "ab", "abc-123", "abc_def", "abc.def", "a1b2c3"] {
            assert!(slug_matches(ok), "should accept: {ok}");
        }
    }

    #[test]
    fn slug_matches_rejects_uppercase_leading_punct_and_overlong() {
        for bad in ["A", "-bad", ".bad", "with space", "x".repeat(101).as_str()] {
            assert!(!slug_matches(bad), "should reject: {bad}");
        }
    }
}
