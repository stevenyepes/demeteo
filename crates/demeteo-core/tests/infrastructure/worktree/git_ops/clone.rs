// Tests extracted from `crates/demeteo-core/src/adapters/worktree/git_ops/clone.rs` (mirrored-tests convention). `super` = that module.

use super::super::common::make_repo;
use super::super::git_request_vec;
use super::{clone_args, clone_config_args, configure_clone};
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::ports::execution::ExecutionPort;

const URL: &str = "https://x-access-token:tok@example.com/acme/widgets";
const TARGET: &str = "/workspace/repos/widgets";

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
        strs(&clone_args(URL, TARGET, true)),
        ["-c", "core.longpaths=true", "clone", URL, TARGET]
    );
}

#[test]
fn a_posix_clone_carries_no_overrides_at_all() {
    assert_eq!(
        strs(&clone_args(URL, TARGET, false)),
        ["clone", URL, TARGET]
    );
}

#[test]
fn no_demeteo_git_command_line_overrides_the_worktree_comparison() {
    for windows_target in [false, true] {
        let mut command_lines = vec![clone_args(URL, TARGET, windows_target)];
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
