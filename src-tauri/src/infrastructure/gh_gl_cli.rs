//! Pure-function helpers for the `gh` / `glab` provider CLIs.
//!
//! The CreateFromZero wizard shells out to whichever CLI the user
//! picked on the Provider step:
//!
//! - GitHub: `gh repo create <org-or-user>/<name> --<visibility> --add-readme --confirm --json name,defaultBranchRef,url`
//! - GitLab: `glab project create <name> --<visibility> --initialize-with-readme --default-branch main -F json`
//!
//! Every command the wizard issues is composed by the pure functions
//! in this module — `gh_create_repo_args`, `glab_create_project_args`
//! — so the unit tests can assert the exact argv the orchestrator
//! will spawn without ever touching the host filesystem. The actual
//! `tokio::process::Command` invocation lives in
//! `CreateProjectAdapter::create_remote_repo`, which uses these
//! builders and then parses the JSON output through
//! `parse_gh_create_repo_output` / `parse_glab_create_project_output`.

use crate::error::AppError;
use crate::ports::provider_http::{CreatedRepo, NamespaceSummary};

/// Provider kind strings accepted by the port layer. Lower-case by
/// construction; the adapter normalises before dispatching.
pub const KIND_GITHUB: &str = "github";
pub const KIND_GITLAB: &str = "gitlab";

/// Visibility values accepted by the wizard's description step.
/// `private` → `--private`; `public` → `--public` (or `--visibility`
/// flag on GitLab depending on the flag mapping).
pub const VIS_PRIVATE: &str = "private";
pub const VIS_PUBLIC: &str = "public";

/// Build the argv list for `gh repo create`. The wizard passes the
/// fully-qualified `org/name` or `user/name` so the CLI doesn't
/// need a `--source` flag or an interactive org picker.
///
/// Reference: <https://cli.github.com/manual/gh_repo_create>
pub fn gh_create_repo_args(
    namespace: &NamespaceSummary,
    name: &str,
    visibility: &str,
) -> Vec<String> {
    // gh refuses to create a repo with the same name as an existing
    // directory the user is currently in; `--confirm` skips the
    // interactive confirmation prompt the wizard can't satisfy.
    let mut args: Vec<String> = vec![
        "repo".to_string(),
        "create".to_string(),
        format!("{}/{}", namespace.id, name),
        "--confirm".to_string(),
        "--add-readme".to_string(),
    ];
    push_visibility_flag_gh(&mut args, visibility);
    // The wizard doesn't need a human-friendly stdout, so always ask
    // for JSON. The adapter parses this — see `parse_gh_create_repo_output`.
    args.push("--json".to_string());
    args.push("name,defaultBranchRef,url".to_string());
    args
}

fn push_visibility_flag_gh(args: &mut Vec<String>, visibility: &str) {
    // Frontend-bug tolerance: an unrecognised value (empty string,
    // a typo, …) must default to `--private`, matching the policy
    // in `normalise_visibility`. Visibility defaults to private for
    // new repos, period — public is the explicit opt-in.
    match visibility {
        VIS_PUBLIC => args.push("--public".to_string()),
        _ => args.push("--private".to_string()),
    }
}

/// Build the argv list for `glab project create`. GitLab's CLI takes
/// the bare name (no namespace prefix); the namespace is conveyed
/// through a separate flag. Personal namespace → omit the flag so
/// glab defaults to the authenticated user; org/group → pass
/// `--namespace <id>` with the numeric id.
///
/// Reference: <https://gitlab.com/gitlab-org/cli/-/blob/main/docs/project_create.md>
pub fn glab_create_project_args(
    namespace: &NamespaceSummary,
    name: &str,
    visibility: &str,
) -> Vec<String> {
    // --initialize-with-readme seeds a default branch + first commit
    // so the wizard's clone/strategy detection has something to bite
    // into. See spec §6 constraint 10.
    let mut args: Vec<String> = vec![
        "project".to_string(),
        "create".to_string(),
        name.to_string(),
        "--initialize-with-readme".to_string(),
        "--default-branch".to_string(),
        "main".to_string(),
        "-F".to_string(),
        "json".to_string(),
    ];
    if namespace.kind != "personal" {
        args.push("--namespace".to_string());
        args.push(namespace.id.clone());
    }
    push_visibility_flag_glab(&mut args, visibility);
    args
}

fn push_visibility_flag_glab(args: &mut Vec<String>, visibility: &str) {
    // See `push_visibility_flag_gh` — unknown → private.
    match visibility {
        VIS_PUBLIC => args.push("--public".to_string()),
        _ => args.push("--private".to_string()),
    }
}

/// Parse `gh repo create --json` stdout. The schema is documented
/// at <https://cli.github.com/manual/gh_repo_create>:
/// `{ "name": "<full_name>", "defaultBranchRef": { "name": "<branch>" }, "url": "<url>" }`.
pub fn parse_gh_create_repo_output(stdout: &str) -> Result<CreatedRepo, AppError> {
    let data: serde_json::Value = serde_json::from_str(stdout).map_err(|e| {
        AppError::provider(format!(
            "gh repo create returned invalid JSON: {} (stdout: {})",
            e, stdout
        ))
    })?;
    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::provider("gh output missing `name` field"))?;
    let default_branch = data
        .pointer("/defaultBranchRef/name")
        .and_then(|v| v.as_str())
        .unwrap_or("main")
        .to_string();
    let clone_url = data
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(CreatedRepo {
        full_name: name.to_string(),
        default_branch,
        clone_url,
    })
}

/// Parse `glab project create -F json` stdout. Schema is documented
/// at <https://gitlab.com/gitlab-org/cli/-/blob/main/docs/project_create.md>:
/// `{ "name": "<name>", "path_with_namespace": "<ns>/<name>", "default_branch": "<branch>", "http_url_to_repo": "<url>" }`.
pub fn parse_glab_create_project_output(stdout: &str) -> Result<CreatedRepo, AppError> {
    let data: serde_json::Value = serde_json::from_str(stdout).map_err(|e| {
        AppError::provider(format!(
            "glab project create returned invalid JSON: {} (stdout: {})",
            e, stdout
        ))
    })?;
    let full_name = data
        .get("path_with_namespace")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::provider("glab output missing `path_with_namespace`"))?;
    let default_branch = data
        .get("default_branch")
        .and_then(|v| v.as_str())
        .unwrap_or("main")
        .to_string();
    let clone_url = data
        .get("http_url_to_repo")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(CreatedRepo {
        full_name: full_name.to_string(),
        default_branch,
        clone_url,
    })
}

/// Translate a non-zero exit status + stderr into an `AppError`.
/// The message is intentionally redacted at the adapter layer (we
/// keep stderr in the structured error for the wizard to render
/// inline, but the PAT never appears in the args we pass to the
/// CLI so redaction is mostly a defence-in-depth).
pub fn cli_failure(provider_kind: &str, code: Option<i32>, stderr: &str) -> AppError {
    let trimmed = stderr.trim();
    let summary = if trimmed.is_empty() {
        format!("{} CLI exited with status {:?}", provider_kind, code)
    } else {
        // Keep the first line of stderr; providers tend to dump
        // multi-paragraph help text on misuse that the wizard would
        // just render as a wall of text.
        let first = trimmed.lines().next().unwrap_or("").trim();
        format!("{} CLI error: {}", provider_kind, first)
    };
    AppError::provider(summary)
}

/// Validate the `provider_kind` argument at the infrastructure
/// boundary. Anything other than `github` / `gitlab` is a programming
/// error in the calling code; we surface it as a `Validation` error
/// so the wizard can show a friendly message instead of a 500.
pub fn normalise_provider_kind(provider_kind: &str) -> Result<&'static str, AppError> {
    match provider_kind.to_ascii_lowercase().as_str() {
        KIND_GITHUB => Ok(KIND_GITHUB),
        KIND_GITLAB => Ok(KIND_GITLAB),
        other => Err(AppError::validation(format!(
            "Unsupported provider kind: {} (expected 'github' or 'gitlab')",
            other
        ))),
    }
}

/// Validate the `visibility` argument. Defaults to `private` when the
/// wizard passes an empty / unknown value so a malformed payload
/// from an older frontend doesn't crash the wizard — it just gets
/// the safer default.
pub fn normalise_visibility(visibility: &str) -> &'static str {
    match visibility.to_ascii_lowercase().as_str() {
        VIS_PUBLIC => VIS_PUBLIC,
        _ => VIS_PRIVATE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(id: &str, kind: &str) -> NamespaceSummary {
        NamespaceSummary {
            id: id.to_string(),
            name: id.to_string(),
            kind: kind.to_string(),
        }
    }

    // ── gh_create_repo_args ─────────────────────────────────────────────

    #[test]
    fn gh_create_repo_args_personal_namespace_uses_owner_name_form() {
        let args = gh_create_repo_args(&ns("octocat", "personal"), "my-repo", VIS_PRIVATE);
        assert_eq!(
            args,
            vec![
                "repo",
                "create",
                "octocat/my-repo",
                "--confirm",
                "--add-readme",
                "--private",
                "--json",
                "name,defaultBranchRef,url",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn gh_create_repo_args_org_namespace_routes_to_org() {
        let args = gh_create_repo_args(&ns("acme", "org"), "spectacular", VIS_PUBLIC);
        assert!(args.contains(&"acme/spectacular".to_string()));
        assert!(args.contains(&"--public".to_string()));
        assert!(!args.contains(&"--private".to_string()));
    }

    #[test]
    fn gh_create_repo_args_unknown_visibility_defaults_to_private() {
        // Frontend bug tolerance: an unknown visibility value must
        // not crash the wizard.
        let args = gh_create_repo_args(&ns("octocat", "personal"), "x", "weird-value");
        assert!(args.contains(&"--private".to_string()));
    }

    // ── glab_create_project_args ────────────────────────────────────────

    #[test]
    fn glab_create_project_args_personal_omits_namespace_flag() {
        let args = glab_create_project_args(&ns("7", "personal"), "my-repo", VIS_PRIVATE);
        assert_eq!(args[0..3], vec!["project", "create", "my-repo"]);
        assert!(args.contains(&"--private".to_string()));
        assert!(args.contains(&"--initialize-with-readme".to_string()));
        assert!(args.contains(&"--default-branch".to_string()));
        assert!(!args.iter().any(|a| a == "--namespace"));
    }

    #[test]
    fn glab_create_project_args_group_sends_numeric_namespace() {
        let args = glab_create_project_args(&ns("42", "group"), "team-repo", VIS_PRIVATE);
        // --namespace is followed by the numeric id.
        let pos = args
            .iter()
            .position(|a| a == "--namespace")
            .expect("--namespace present");
        assert_eq!(args[pos + 1], "42");
    }

    #[test]
    fn glab_create_project_args_visibility_public() {
        let args = glab_create_project_args(&ns("octo", "personal"), "demo", VIS_PUBLIC);
        assert!(args.contains(&"--public".to_string()));
        assert!(!args.contains(&"--private".to_string()));
    }

    // ── parse_gh_create_repo_output ─────────────────────────────────────

    #[test]
    fn parse_gh_create_repo_output_reads_full_name_and_branch() {
        let json = r#"{
            "name": "acme/spectacular",
            "defaultBranchRef": { "name": "main" },
            "url": "https://github.com/acme/spectacular.git"
        }"#;
        let repo = parse_gh_create_repo_output(json).unwrap();
        assert_eq!(repo.full_name, "acme/spectacular");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.clone_url, "https://github.com/acme/spectacular.git");
    }

    #[test]
    fn parse_gh_create_repo_output_defaults_branch_to_main() {
        // Old gh versions might not return `defaultBranchRef`; the
        // parser must fall back to the documented default.
        let json = r#"{ "name": "acme/x", "url": "https://github.com/acme/x.git" }"#;
        let repo = parse_gh_create_repo_output(json).unwrap();
        assert_eq!(repo.default_branch, "main");
    }

    #[test]
    fn parse_gh_create_repo_output_rejects_non_json() {
        let err = parse_gh_create_repo_output("not json at all").unwrap_err();
        assert_eq!(err.code(), "provider");
    }

    #[test]
    fn parse_gh_create_repo_output_rejects_missing_name() {
        let json = r#"{ "url": "https://github.com/acme/x.git" }"#;
        let err = parse_gh_create_repo_output(json).unwrap_err();
        assert_eq!(err.code(), "provider");
    }

    // ── parse_glab_create_project_output ────────────────────────────────

    #[test]
    fn parse_glab_create_project_output_reads_path_with_namespace() {
        let json = r#"{
            "name": "team-repo",
            "path_with_namespace": "acme/team/team-repo",
            "default_branch": "main",
            "http_url_to_repo": "https://gitlab.com/acme/team/team-repo.git"
        }"#;
        let repo = parse_glab_create_project_output(json).unwrap();
        assert_eq!(repo.full_name, "acme/team/team-repo");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.clone_url, "https://gitlab.com/acme/team/team-repo.git");
    }

    #[test]
    fn parse_glab_create_project_output_rejects_missing_path() {
        let json = r#"{ "name": "x" }"#;
        let err = parse_glab_create_project_output(json).unwrap_err();
        assert_eq!(err.code(), "provider");
    }

    // ── cli_failure ─────────────────────────────────────────────────────

    #[test]
    fn cli_failure_uses_first_line_of_stderr() {
        let err = cli_failure(
            "gh",
            Some(1),
            "could not create: name already exists\nfull help text",
        );
        match err {
            AppError::Provider { message } => {
                assert!(message.starts_with("gh CLI error"));
                assert!(message.contains("already exists"));
                assert!(!message.contains("full help text"));
            }
            _ => panic!("expected Provider"),
        }
    }

    #[test]
    fn cli_failure_with_empty_stderr_reports_exit_status() {
        let err = cli_failure("glab", Some(2), "   \n");
        match err {
            AppError::Provider { message } => {
                assert!(message.contains("status"));
            }
            _ => panic!("expected Provider"),
        }
    }

    // ── normalise_* ─────────────────────────────────────────────────────

    #[test]
    fn normalise_provider_kind_accepts_lowercase_and_uppercase() {
        assert_eq!(normalise_provider_kind("github").unwrap(), KIND_GITHUB);
        assert_eq!(normalise_provider_kind("GitHub").unwrap(), KIND_GITHUB);
        assert_eq!(normalise_provider_kind("GITLAB").unwrap(), KIND_GITLAB);
    }

    #[test]
    fn normalise_provider_kind_rejects_unknown() {
        let err = normalise_provider_kind("bitbucket").unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn normalise_visibility_falls_back_to_private() {
        assert_eq!(normalise_visibility("public"), VIS_PUBLIC);
        assert_eq!(normalise_visibility("Public"), VIS_PUBLIC);
        assert_eq!(normalise_visibility(""), VIS_PRIVATE);
        assert_eq!(normalise_visibility("weird"), VIS_PRIVATE);
    }
}
