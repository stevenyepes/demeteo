use crate::adapters::provider_http::{
    create_repo_body, create_repo_url, parse_created_repo, parse_namespaces, provider_http_error,
    ReqwestProviderHttpAdapter,
};
use crate::application::agent_probe::{fallback_models, model_supports_images_by_name};
use crate::application::providers::sanitize_host;
use crate::error::AppError;
use crate::ports::provider_http::{CreateRepoRequest, NamespaceSummary, ProviderHttpPort};
use serde_json::json;

// ── create_repo_url: endpoint routing per provider/namespace ─────────────

fn ns(id: &str, kind: &str) -> NamespaceSummary {
    NamespaceSummary {
        id: id.to_string(),
        name: id.to_string(),
        kind: kind.to_string(),
    }
}

#[test]
fn create_repo_url_github_personal_uses_user_repos() {
    let url = create_repo_url("github.com", "github", &ns("octocat", "personal")).unwrap();
    assert_eq!(url, "https://api.github.com/user/repos");
}

#[test]
fn create_repo_url_github_org_uses_orgs_endpoint() {
    let url = create_repo_url("github.com", "github", &ns("acme", "org")).unwrap();
    assert_eq!(url, "https://api.github.com/orgs/acme/repos");
}

#[test]
fn create_repo_url_github_enterprise_uses_api_v3_prefix() {
    // Never hardcode github.com — an enterprise host must route through /api/v3.
    let personal =
        create_repo_url("github.acme.dev", "github", &ns("octocat", "personal")).unwrap();
    assert_eq!(personal, "https://github.acme.dev/api/v3/user/repos");
    let org = create_repo_url("github.acme.dev", "github", &ns("acme", "org")).unwrap();
    assert_eq!(org, "https://github.acme.dev/api/v3/orgs/acme/repos");
}

#[test]
fn create_repo_url_gitlab_uses_projects_endpoint() {
    let url = create_repo_url("gitlab.com", "gitlab", &ns("42", "group")).unwrap();
    assert_eq!(url, "https://gitlab.com/api/v4/projects");
    // Self-hosted GitLab keeps its host and the /api/v4 prefix.
    let sh = create_repo_url("gitlab.acme.dev", "gitlab", &ns("42", "group")).unwrap();
    assert_eq!(sh, "https://gitlab.acme.dev/api/v4/projects");
}

#[test]
fn create_repo_url_unsupported_kind_is_validation() {
    let err = create_repo_url("host", "bitbucket", &ns("x", "personal")).unwrap_err();
    assert_eq!(err.code(), "validation");
}

// ── create_repo_body: provider-specific payloads ─────────────────────────

fn req(namespace: NamespaceSummary, name: &str, private: bool) -> CreateRepoRequest {
    CreateRepoRequest {
        namespace,
        name: name.to_string(),
        private,
        auto_init: true,
    }
}

#[test]
fn create_repo_body_github_has_name_private_auto_init() {
    let body =
        create_repo_body("github", &req(ns("octocat", "personal"), "my-repo", true)).unwrap();
    assert_eq!(body["name"], "my-repo");
    assert_eq!(body["private"], true);
    assert_eq!(body["auto_init"], true);
    // GitHub payload must not carry GitLab-only fields.
    assert!(body.get("namespace_id").is_none());
    assert!(body.get("initialize_with_readme").is_none());
}

#[test]
fn create_repo_body_github_org_still_omits_namespace_id() {
    // Org routing is via the URL, not the body.
    let body = create_repo_body("github", &req(ns("acme", "org"), "my-repo", false)).unwrap();
    assert_eq!(body["private"], false);
    assert!(body.get("namespace_id").is_none());
}

#[test]
fn create_repo_body_gitlab_group_sends_numeric_namespace_id() {
    let body = create_repo_body("gitlab", &req(ns("42", "group"), "my-repo", true)).unwrap();
    assert_eq!(body["name"], "my-repo");
    assert_eq!(body["path"], "my-repo");
    assert_eq!(body["visibility"], "private");
    assert_eq!(body["initialize_with_readme"], true);
    // Numeric, not a string.
    assert_eq!(body["namespace_id"], json!(42));
}

#[test]
fn create_repo_body_gitlab_personal_omits_namespace_id() {
    let body = create_repo_body("gitlab", &req(ns("7", "personal"), "my-repo", false)).unwrap();
    assert_eq!(body["visibility"], "public");
    assert!(body.get("namespace_id").is_none());
}

// ── parse_namespaces: merge personal + orgs/groups ───────────────────────

#[test]
fn parse_namespaces_github_merges_personal_and_orgs() {
    let user = json!({ "login": "octocat" });
    let orgs = vec![json!({ "login": "acme" }), json!({ "login": "globex" })];
    let out = parse_namespaces("github", &user, &orgs);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].id, "octocat");
    assert_eq!(out[0].kind, "personal");
    assert_eq!(out[1].id, "acme");
    assert_eq!(out[1].kind, "org");
    assert_eq!(out[2].id, "globex");
}

#[test]
fn parse_namespaces_gitlab_uses_numeric_group_id_as_string() {
    let user = json!({ "username": "octo", "namespace_id": 7 });
    let groups = vec![json!({ "id": 42, "full_path": "acme/team" })];
    let out = parse_namespaces("gitlab", &user, &groups);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].id, "7");
    assert_eq!(out[0].kind, "personal");
    // Numeric GitLab id surfaces as its string form.
    assert_eq!(out[1].id, "42");
    assert_eq!(out[1].name, "acme/team");
    assert_eq!(out[1].kind, "group");
}

#[test]
fn parse_namespaces_gitlab_no_groups_returns_only_personal() {
    let user = json!({ "username": "octo", "id": 9 });
    let out = parse_namespaces("gitlab", &user, &[]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, "9");
    assert_eq!(out[0].kind, "personal");
}

// ── parse_created_repo: response shape per provider ──────────────────────

#[test]
fn parse_created_repo_github_reads_full_name() {
    let data = json!({
        "full_name": "acme/my-repo",
        "default_branch": "main",
        "clone_url": "https://github.com/acme/my-repo.git",
    });
    let repo = parse_created_repo("github", &data);
    assert_eq!(repo.full_name, "acme/my-repo");
    assert_eq!(repo.default_branch, "main");
    assert_eq!(repo.clone_url, "https://github.com/acme/my-repo.git");
}

#[test]
fn parse_created_repo_gitlab_reads_path_with_namespace() {
    let data = json!({
        "path_with_namespace": "acme/team/my-repo",
        "default_branch": "main",
        "http_url_to_repo": "https://gitlab.com/acme/team/my-repo.git",
    });
    let repo = parse_created_repo("gitlab", &data);
    assert_eq!(repo.full_name, "acme/team/my-repo");
    assert_eq!(repo.clone_url, "https://gitlab.com/acme/team/my-repo.git");
}

// ── error mapping: status → AppError kind ────────────────────────────────

#[test]
fn provider_http_error_401_maps_to_provider() {
    let err = provider_http_error(401, "Bad credentials");
    assert_eq!(err.code(), "provider");
    match err {
        AppError::Provider { message } => assert!(message.contains("401")),
        _ => panic!("expected Provider"),
    }
}

#[test]
fn provider_http_error_422_maps_to_provider_with_body() {
    // Duplicate/invalid repo name — surfaced verbatim so the wizard can show it.
    let err = provider_http_error(422, "name already exists on this account");
    assert_eq!(err.code(), "provider");
    match err {
        AppError::Provider { message } => {
            assert!(message.contains("422"));
            assert!(message.contains("already exists"));
        }
        _ => panic!("expected Provider"),
    }
}

#[tokio::test]
async fn create_repo_network_failure_maps_to_transport() {
    let adapter = ReqwestProviderHttpAdapter::new();
    // 127.0.0.1:9 (discard) refuses connections → a transport-level failure,
    // not a provider response. No live GitHub/GitLab is contacted.
    let err = adapter
        .create_repo(
            "127.0.0.1:9",
            "github",
            "pat",
            &req(ns("octocat", "personal"), "my-repo", true),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), "transport");
}

#[test]
fn test_sanitize_host() {
    assert_eq!(
        sanitize_host("https://gitlab.stvcloud.dev/prototype/spectacular.git"),
        "gitlab.stvcloud.dev"
    );
    assert_eq!(
        sanitize_host("http://gitlab.company.com:8080/path"),
        "gitlab.company.com:8080"
    );
    assert_eq!(sanitize_host("gitlab.company.com"), "gitlab.company.com");
    assert_eq!(
        sanitize_host("   https://api.github.com   "),
        "api.github.com"
    );
}

// ── vision-capability fallback table ────────────────────────────────────
// Mirrors the soft-warning contract used by the Start-Feature modal:
// `supports_images` is `true` only for model entries that are *known*
// to accept image input. Anything not in the bundled list falls
// through to `model_supports_images_by_name` (pessimistic).

fn find(models: &[crate::domain::models::ConfigOptionValue], value: &str) -> bool {
    models.iter().any(|m| m.value == value)
}

#[test]
fn fallback_claude_code_flags_vision_aliases() {
    let models = fallback_models("claude-code");
    assert!(!models.is_empty());
    for alias in ["opus", "sonnet", "haiku"] {
        let m = models
            .iter()
            .find(|m| m.value == alias)
            .unwrap_or_else(|| panic!("missing claude-code alias: {}", alias));
        assert!(
            m.supports_images,
            "{} should be flagged as vision-capable",
            alias
        );
    }
}

#[test]
fn fallback_claude_code_flags_fable_as_vision() {
    let models = fallback_models("claude-code");
    let fable = models
        .iter()
        .find(|m| m.value == "fable")
        .expect("fable alias should exist");
    assert!(
        fable.supports_images,
        "fable now maps to Claude Fable 5 (GA, vision-capable) — must be flagged"
    );
}

#[test]
fn fallback_opencode_flags_known_vision_models() {
    let models = fallback_models("opencode");
    // Three vision-capable models.
    for value in [
        "anthropic/claude-3-5-sonnet-20241022",
        "openai/gpt-4o",
        "google/gemini-2.5-flash",
    ] {
        assert!(find(&models, value), "missing opencode entry: {}", value);
        let m = models.iter().find(|m| m.value == value).unwrap();
        assert!(
            m.supports_images,
            "{} should be flagged as vision-capable",
            value
        );
    }
}

#[test]
fn fallback_opencode_flags_deepseek_coder_as_not_vision() {
    let models = fallback_models("opencode");
    let coder = models
        .iter()
        .find(|m| m.value == "deepseek/deepseek-coder-v2")
        .expect("deepseek-coder entry should exist");
    assert!(
        !coder.supports_images,
        "deepseek-coder is text-only — must NOT be flagged as vision"
    );
}

#[test]
fn fallback_hermes_uses_same_table_as_opencode() {
    let opencode = fallback_models("opencode");
    let hermes = fallback_models("hermes");
    assert_eq!(opencode.len(), hermes.len());
    for (a, b) in opencode.iter().zip(hermes.iter()) {
        assert_eq!(a.value, b.value);
        assert_eq!(a.supports_images, b.supports_images);
    }
}

#[test]
fn fallback_unknown_agent_kind_returns_empty() {
    assert!(fallback_models("not-a-real-agent").is_empty());
    // The removed `antigravity` kind is now just another unregistered kind.
    assert!(fallback_models("antigravity").is_empty());
}

// ── substring heuristic for free-form model strings ─────────────────────
// Used for dynamically probed models that aren't in the bundled
// fallback table. Negative matches MUST override positive ones.

#[test]
fn heuristic_positive_substrings() {
    let positives = [
        "gpt-4o",
        "gpt-4-turbo",
        "gemini-1.5-pro",
        "gemini-2.5-flash",
        "claude-3-5-sonnet",
        "claude-opus-4",
        "vision-experimental",
        "opus-2025-01-01",
        "sonnet-4-5",
        "haiku-3",
        // fable now resolves to Claude Fable 5 (GA, vision-capable).
        "fable-2025",
        // minimax vendor models — image-aware per the bundled
        // capabilities table. The substring is present in both the
        // bare model id ("MiniMax-M3") and the routing prefix
        // ("minimax-coding-plan/MiniMax-M3"), so the optimistic
        // branch flags both.
        "minimax-coding-plan/MiniMax-M3",
        "MiniMax-M3",
        "minimax-m2",
    ];
    for m in positives {
        assert!(
            model_supports_images_by_name("opencode", m),
            "{} should be flagged true via positive substring",
            m
        );
    }
}

#[test]
fn heuristic_is_case_insensitive() {
    assert!(model_supports_images_by_name(
        "opencode",
        "CLAUDE-OPUS-4-LATEST"
    ));
    assert!(model_supports_images_by_name("opencode", "Gemini-2.5-Pro"));
}

#[test]
fn heuristic_negative_substrings_override_positive() {
    let negatives = [
        "text-embedding-3-small",
        "text-embedding-ada-002",
        "whisper-1",
        "whisper-large-v3",
    ];
    for m in negatives {
        assert!(
            !model_supports_images_by_name("opencode", m),
            "{} must be flagged false via negative substring",
            m
        );
    }
}

#[test]
fn heuristic_unknown_model_returns_false() {
    let unknowns = [
        "deepseek-coder-v2",
        "llama-3-70b",
        "mistral-7b",
        "command-r-plus",
    ];
    for m in unknowns {
        assert!(
            !model_supports_images_by_name("opencode", m),
            "{} is unknown — pessimistic answer must be false",
            m
        );
    }
}

#[test]
fn heuristic_empty_or_whitespace_returns_false() {
    assert!(!model_supports_images_by_name("opencode", ""));
    assert!(!model_supports_images_by_name("opencode", "   "));
}

#[test]
fn heuristic_trims_whitespace_before_matching() {
    assert!(model_supports_images_by_name("opencode", "  gpt-4o-mini  "));
}
