// Tests extracted from `crates/demeteo-core/src/ports/create_project_port.rs` (mirrored-tests convention). `super` = that module.

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
        _pat: &str,
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
