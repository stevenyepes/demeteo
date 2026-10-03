// Tests extracted from `crates/demeteo-core/src/adapters/worktree/git_ops/clone.rs` (mirrored-tests convention). `super` = that module.

use super::super::common::make_repo;
use super::super::{git_request_vec, GitOpsHelper};
use super::{clone_args, clone_config_args, clones_to_windows, configure_clone};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::git_push::{
    clone_request, credential_helper, GitCredential, HOST_ENV_VAR, PAT_ENV_VAR, USER_ENV_VAR,
};
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::ids::ProviderId;
use crate::domain::models::ProviderInstance;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::{ExecutionPort, ProgramRequest};
use rusqlite::Connection;
use std::sync::Arc;

const HOST: &str = "example.com";
const REPO: &str = "acme/widgets";
const URL: &str = "https://x-access-token@example.com/acme/widgets";
const TARGET: &str = "/workspace/repos/widgets";
const PARENT: &str = "/workspace/repos";
const PAT: &str = "glpat-not-a-real-token";

/// The three keys that decide index-versus-worktree equality, and therefore
/// what `git status` reports the step to have written.
const FORBIDDEN_OVERRIDES: [&str; 3] = ["core.autocrlf", "core.eol", "core.symlinks"];

fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

async fn config_value(
    exec: &LocalSubprocessAdapter,
    repo: &str,
    key: &str,
) -> Result<String, String> {
    exec.run_program(
        "local",
        git_request_vec(
            repo,
            vec![
                "config".to_string(),
                "--local".to_string(),
                "--get".to_string(),
                key.to_string(),
            ],
        ),
    )
    .await
    .map(|value| value.trim().to_string())
}

#[test]
fn a_windows_clone_carries_long_paths_ahead_of_the_subcommand() {
    // Behind `clone` it would be git-clone's own `--config`, which lands in the
    // new repository's config but not in the process doing the cloning.
    assert_eq!(
        strs(&clone_args("x-access-token", HOST, REPO, TARGET, true)),
        ["-c", "core.longpaths=true", "clone", URL, TARGET]
    );
}

#[test]
fn a_posix_clone_carries_no_overrides_at_all() {
    assert_eq!(
        strs(&clone_args("x-access-token", HOST, REPO, TARGET, false)),
        ["clone", URL, TARGET]
    );
}

#[test]
fn no_demeteo_git_command_line_overrides_the_worktree_comparison() {
    for windows_target in [false, true] {
        let mut command_lines = vec![clone_args(
            "x-access-token",
            HOST,
            REPO,
            TARGET,
            windows_target,
        )];
        command_lines.extend(clone_config_args(windows_target));
        for args in command_lines {
            for (flag, value) in args.iter().zip(args.iter().skip(1)) {
                if flag != "-c" {
                    continue;
                }
                for key in FORBIDDEN_OVERRIDES {
                    assert!(
                        !value.starts_with(&format!("{key}=")),
                        "{key} is overridden on a command line: {args:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn long_paths_is_persisted_only_where_a_max_path_exists() {
    let windows = clone_config_args(true);
    let posix = clone_config_args(false);
    assert!(
        windows
            .iter()
            .any(|args| args.contains(&"core.longpaths".to_string())),
        "{windows:?}"
    );
    assert!(
        !posix
            .iter()
            .any(|args| args.contains(&"core.longpaths".to_string())),
        "{posix:?}"
    );
}

#[tokio::test]
async fn configuring_a_clone_writes_autocrlf_false_into_its_own_config_file() {
    let (dir, _helper) = make_repo("clone_config_posix").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    configure_clone(&exec, "local", &repo, false)
        .await
        .expect("configures the clone Demeteo owns");

    let on_disk = std::fs::read_to_string(dir.join(".git").join("config"))
        .expect("reads the clone's own config file");
    assert!(
        on_disk.contains("autocrlf = false"),
        "the setting has to survive the command that set it: {on_disk}"
    );
    assert_eq!(
        config_value(&exec, &repo, "core.autocrlf").await,
        Ok("false".to_string())
    );
    assert!(
        config_value(&exec, &repo, "core.longpaths").await.is_err(),
        "a POSIX clone has no MAX_PATH to raise"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_windows_clone_persists_both_settings() {
    let (dir, _helper) = make_repo("clone_config_windows").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    configure_clone(&exec, "local", &repo, true)
        .await
        .expect("configures the clone Demeteo owns");

    assert_eq!(
        config_value(&exec, &repo, "core.autocrlf").await,
        Ok("false".to_string())
    );
    assert_eq!(
        config_value(&exec, &repo, "core.longpaths").await,
        Ok("true".to_string())
    );

    let _ = std::fs::remove_dir_all(dir);
}

fn cred_for(kind: &str) -> GitCredential {
    GitCredential {
        user: crate::adapters::git_push::remote_user(kind),
        pat: PAT.to_string(),
        host: "github.com".to_string(),
    }
}

fn rendered(request: &ProgramRequest) -> String {
    format!("git {}", request.args.join(" "))
}

/// Whether the strict double below is scripting a Windows-host clone: the
/// `"local"` machine is the desktop, so on a Windows CI leg it is one.
fn windows_target() -> bool {
    clones_to_windows("local")
}

fn config_keys() -> Vec<String> {
    clone_config_args(windows_target())
        .iter()
        .map(|args| format!("git -C {TARGET} {}", args.join(" ")))
        .collect()
}

/// Runs `clone_repository` for a provider of `kind` against a strict double
/// scripted to accept exactly the invocations a successful clone makes, and
/// returns the result with every program request the double saw.
async fn clone_for(
    kind: &str,
    provider_id: &str,
    clone_answer: Result<&str, &str>,
) -> (Result<(), String>, Vec<ProgramRequest>) {
    clone_from(HOST, kind, provider_id, clone_answer).await
}

async fn clone_from(
    host: &str,
    kind: &str,
    provider_id: &str,
    clone_answer: Result<&str, &str>,
) -> (Result<(), String>, Vec<ProgramRequest>) {
    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    db.add_provider_instance(ProviderInstance {
        id: ProviderId::from(provider_id),
        kind: kind.to_string(),
        host: host.to_string(),
        username: "someone".to_string(),
        avatar_url: String::new(),
        created_at: 0,
    })
    .unwrap();
    crate::credential_cache::set(provider_id, PAT);

    let clone_key = rendered(&clone_request(
        clone_args(cred_for(kind).user, host, REPO, TARGET, windows_target()),
        &cred_for(kind),
    ));
    let config_keys = config_keys();
    let mut programs = vec![(clone_key.as_str(), clone_answer)];
    programs.extend(config_keys.iter().map(|key| (key.as_str(), Ok(""))));
    let exec = Arc::new(
        ScriptedExec::new(&[])
            .with_dirs(&[PARENT])
            .with_programs(&programs),
    );
    let result = GitOpsHelper::new(db, exec.clone())
        .clone_repository(Some("local"), provider_id, REPO, TARGET)
        .await;
    (result, exec.requests())
}

#[tokio::test]
async fn a_clone_names_the_provider_user_and_never_the_token() {
    for (kind, id, url) in [
        (
            "github",
            "prov-clone-gh",
            "https://x-access-token@example.com/acme/widgets",
        ),
        (
            "gitlab",
            "prov-clone-gl",
            "https://oauth2@example.com/acme/widgets",
        ),
    ] {
        let (result, requests) = clone_for(kind, id, Ok("")).await;
        result.expect("the scripted clone succeeds");

        let clone = &requests[0];
        for arg in &clone.args {
            assert!(!arg.contains(PAT), "{kind}: token in argv: {arg}");
        }
        assert_eq!(clone.args[clone.args.len() - 2], url, "{kind}");
        assert_eq!(clone.args.last().map(String::as_str), Some(TARGET));
    }
}

#[tokio::test]
async fn a_clone_resets_then_installs_the_helper_before_the_subcommand() {
    let (result, requests) = clone_for("github", "prov-clone-order", Ok("")).await;
    result.expect("the scripted clone succeeds");

    let helper = format!("credential.helper={}", credential_helper());
    let clone = clone_args("x-access-token", HOST, REPO, TARGET, windows_target());
    let mut expected = vec!["-c", "credential.helper=", "-c", helper.as_str()];
    expected.extend(strs(&clone));
    assert_eq!(strs(&requests[0].args), expected);
}

#[tokio::test]
async fn a_clone_carries_the_credential_and_every_prompt_suppression_in_its_env() {
    let (result, requests) = clone_for("gitlab", "prov-clone-env", Ok("")).await;
    result.expect("the scripted clone succeeds");

    let clone = &requests[0];
    assert_eq!(clone.env.get(PAT_ENV_VAR).map(String::as_str), Some(PAT));
    assert_eq!(
        clone.env.get(USER_ENV_VAR).map(String::as_str),
        Some("oauth2")
    );
    for (key, value) in crate::domain::git_push::unattended_env() {
        assert_eq!(clone.env.get(&key), Some(&value), "{key}");
    }
    assert_eq!(clone.timeout, None);
}

/// `sanitize_host` keeps `:port`, so the URL must too — and the helper, which
/// compares git's port-stripped `host=` against `DEMETEO_GIT_HOST`, must be
/// bound to the bare host or it answers nothing and the clone dies asking for
/// a username.
#[tokio::test]
async fn a_provider_on_a_port_clones_by_a_url_that_keeps_it_and_a_helper_bound_without_it() {
    let (result, requests) =
        clone_from("gitlab.local:8443", "gitlab", "prov-clone-port", Ok("")).await;
    result.expect("the scripted clone succeeds");

    let clone = &requests[0];
    assert_eq!(
        clone.args[clone.args.len() - 2],
        "https://oauth2@gitlab.local:8443/acme/widgets"
    );
    assert_eq!(
        clone.env.get(HOST_ENV_VAR).map(String::as_str),
        Some("gitlab.local")
    );

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "{} get",
                credential_helper().trim_start_matches('!')
            ))
            .envs(&clone.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"protocol=https\nhost=gitlab.local:8443\n\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();

        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            format!("username=oauth2\npassword={PAT}\n")
        );
    }
}

#[tokio::test]
async fn a_failed_clone_returns_an_error_without_the_token() {
    let leaked = format!("Command failed: sh -c export DEMETEO_GIT_PAT={PAT}; git clone");
    let (result, _) = clone_for("github", "prov-clone-err", Err(&leaked)).await;

    let error = result.expect_err("the scripted clone fails");
    assert!(!error.contains(PAT), "token survived: {error}");
    assert!(
        error.contains("git clone"),
        "the diagnosis survives: {error}"
    );
}

#[test]
fn every_dash_c_precedes_the_subcommand_on_a_windows_clone() {
    let request = clone_request(
        clone_args("x-access-token", HOST, REPO, TARGET, true),
        &cred_for("github"),
    );
    let clone_at = request.args.iter().position(|a| a == "clone").unwrap();
    let dash_c: Vec<usize> = request
        .args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "-c")
        .map(|(i, _)| i)
        .collect();

    assert_eq!(dash_c.len(), 3, "{:?}", request.args);
    assert!(dash_c.iter().all(|i| *i < clone_at), "{:?}", request.args);
    assert_eq!(request.args[1], "credential.helper=");
    assert!(request.args[3].starts_with("credential.helper=!"));
    assert_eq!(request.args[5], "core.longpaths=true");
    assert_eq!(request.args[clone_at + 1], URL);
}
