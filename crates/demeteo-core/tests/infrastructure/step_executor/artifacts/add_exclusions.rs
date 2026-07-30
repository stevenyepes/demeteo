//! The `git add` pathspec, against a real repo and against a strict double.
//!
//! The three `.gitignore` cases genuinely need git's own ignore semantics, so
//! they drive `LocalSubprocessAdapter` end to end through
//! `commit_worktree_changes`. The shape of the probe command itself is asserted
//! separately, against a double that answers the probe and **errors on any
//! other command**.

use super::*;
use crate::adapters::step_executor::artifacts::commit_worktree_changes;
use crate::ports::execution::ShellOptions;
use std::sync::Mutex;

/// Set up `temp` as a worktree that has a dependency-cache symlink
/// standing in for `node_modules` (what `provision_subtask_worktree`
/// leaves behind), with `ignore_line` as the repo's whole `.gitignore`,
/// one committed file, and one uncommitted agent write.
async fn repo_with_node_modules_symlink(
    exec: &crate::adapters::local::execution::LocalSubprocessAdapter,
    temp: &str,
    ignore_line: &str,
) {
    let machine = "local";
    exec.write_file(machine, &format!("{temp}/.gitignore"), ignore_line)
        .await
        .unwrap();
    exec.write_file(machine, &format!("{temp}/src.rs"), "fn a() {}\n")
        .await
        .unwrap();
    exec.run_command(
        machine,
        &format!(
            "git -C {t} add -A && git -c user.email=t@t.com -c user.name=t -C {t} commit -m base",
            t = shell_esc(temp),
        ),
    )
    .await
    .unwrap();

    // The cache lives outside the worktree; the worktree only gets a
    // symlink to it, exactly like `link_dependency_caches_cmd` does.
    let cache = format!("{temp}_cache");
    exec.write_file(
        machine,
        &format!("{cache}/node_modules/dep/index.js"),
        "//x\n",
    )
    .await
    .unwrap();
    exec.run_command(
        machine,
        &format!(
            "ln -sfn {}/node_modules {}/node_modules",
            shell_esc(&cache),
            shell_esc(temp),
        ),
    )
    .await
    .unwrap();

    // The agent's actual deliverable for the step.
    exec.write_file(machine, &format!("{temp}/src.rs"), "fn b() {}\n")
        .await
        .unwrap();
}

/// Regression: `git add` must not fail when a `.gitignore` entry has no
/// trailing slash (`node_modules`, the common form) and therefore
/// already covers our symlink.
///
/// The symlink exclusion used to be added unconditionally, and naming a
/// path in a pathspec makes git treat it as explicitly requested even
/// when the pathspec is negative — so `git add -A -- ':!node_modules'`
/// exited 1 with "The following paths are ignored by one of your
/// .gitignore files", the sequence step could commit nothing, and the
/// whole task failed with "produced nothing to merge".
#[tokio::test]
async fn test_commit_worktree_changes_when_symlinked_cache_dir_is_gitignored() {
    let temp = temp_git_repo("commit_worktree_ignored_symlink");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    // No trailing slash: this pattern matches the symlink too, so git
    // ignores it and the exclusion must be dropped.
    repo_with_node_modules_symlink(&exec, &temp, "node_modules\n").await;

    let sha = commit_worktree_changes(
        &exec,
        machine,
        &temp,
        "worker: task-1",
        "artifacts/",
        false,
        &[],
    )
    .await
    .expect("an already-ignored cache symlink must not break `git add`");
    assert!(!sha.is_empty());

    let committed = exec
        .run_command(
            machine,
            &format!(
                "git -C {} show --stat --name-only --format= HEAD",
                shell_esc(&temp)
            ),
        )
        .await
        .unwrap();
    assert!(
        committed.contains("src.rs"),
        "the agent's deliverable is committed, got: {committed}"
    );
    assert!(
        !committed.contains("node_modules"),
        "the cache symlink stays out of the commit, got: {committed}"
    );

    let _ = std::fs::remove_dir_all(&temp);
    let _ = std::fs::remove_dir_all(format!("{temp}_cache"));
}

/// The other half of the gate: a trailing-slash `.gitignore` pattern
/// (`node_modules/`) does NOT match a symlink of that name, so git sees
/// it as untracked and the exclusion is the only thing keeping an
/// absolute host path out of the feature branch. Dropping the exclusion
/// wholesale to fix the test above would reintroduce that bug.
#[tokio::test]
async fn test_commit_worktree_changes_excludes_unignored_cache_symlink() {
    let temp = temp_git_repo("commit_worktree_unignored_symlink");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    repo_with_node_modules_symlink(&exec, &temp, "node_modules/\n").await;

    let sha = commit_worktree_changes(
        &exec,
        machine,
        &temp,
        "worker: task-1",
        "artifacts/",
        false,
        &[],
    )
    .await
    .unwrap();
    assert!(!sha.is_empty());

    let committed = exec
        .run_command(
            machine,
            &format!(
                "git -C {} show --stat --name-only --format= HEAD",
                shell_esc(&temp)
            ),
        )
        .await
        .unwrap();
    assert!(
        committed.contains("src.rs"),
        "the agent's deliverable is committed, got: {committed}"
    );
    assert!(
        !committed.contains("node_modules"),
        "the unignored cache symlink is pathspec-excluded, got: {committed}"
    );

    let _ = std::fs::remove_dir_all(&temp);
    let _ = std::fs::remove_dir_all(format!("{temp}_cache"));
}

/// Same pathspec trap, reached through the artifact subdir instead: a
/// project that gitignores the artifact dir made every
/// `commit_artifacts=false` commit fail.
#[tokio::test]
async fn test_commit_worktree_changes_when_artifact_subdir_is_gitignored() {
    let temp = temp_git_repo("commit_worktree_ignored_artifacts");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    exec.write_file(machine, &format!("{temp}/.gitignore"), "artifacts/\n")
        .await
        .unwrap();
    exec.write_file(machine, &format!("{temp}/src.rs"), "fn a() {}\n")
        .await
        .unwrap();
    exec.run_command(
        machine,
        &format!(
            "git -C {t} add -A && git -c user.email=t@t.com -c user.name=t -C {t} commit -m base",
            t = shell_esc(&temp),
        ),
    )
    .await
    .unwrap();

    exec.write_file(machine, &format!("{temp}/src.rs"), "fn b() {}\n")
        .await
        .unwrap();
    exec.write_file(
        machine,
        &format!("{temp}/artifacts/report.md"),
        "# report\n",
    )
    .await
    .unwrap();

    let sha = commit_worktree_changes(
        &exec,
        machine,
        &temp,
        "worker: task-1",
        "artifacts/",
        false,
        &[],
    )
    .await
    .expect("an already-ignored artifact subdir must not break `git add`");
    assert!(!sha.is_empty());

    let committed = exec
        .run_command(
            machine,
            &format!(
                "git -C {} show --stat --name-only --format= HEAD",
                shell_esc(&temp)
            ),
        )
        .await
        .unwrap();
    assert!(committed.contains("src.rs"), "got: {committed}");
    assert!(
        !committed.contains("report.md"),
        "the report stays out of the PR, got: {committed}"
    );

    let _ = std::fs::remove_dir_all(&temp);
}
fn temp_git_repo(label: &str) -> String {
    let d = std::env::temp_dir().join(format!(
        "demeteo_test_{}_{}",
        label,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let path = d.to_string_lossy().to_string();
    let cmd = format!("git init -b main {}", shell_esc(&path));
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output();
    path
}

fn shell_esc(s: &str) -> String {
    crate::paths::shell_escape_posix(s)
}

/// An `ExecutionPort` double that answers **only** the exclusion probe and
/// errors on anything else, so "it ran a different command" is a failure rather
/// than silence (AGENTS.md §7).
struct ProbeOnlyExec {
    answer: String,
    seen: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl crate::ports::execution::ExecutionPort for ProbeOnlyExec {
    async fn test_connection(&self, _m: &str) -> Result<(), String> {
        Err("unscripted test_connection".into())
    }
    async fn run_command_with(
        &self,
        _m: &str,
        cmd: &str,
        _o: ShellOptions,
    ) -> Result<String, String> {
        self.seen.lock().unwrap().push(cmd.to_string());
        if cmd.starts_with("cd ") && cmd.contains("check-ignore") {
            Ok(self.answer.clone())
        } else {
            Err(format!("ProbeOnlyExec: unscripted command `{cmd}`"))
        }
    }
    async fn read_file(&self, _m: &str, _p: &str) -> Result<String, String> {
        Err("unscripted read_file".into())
    }
    async fn write_file(&self, _m: &str, _p: &str, _c: &str) -> Result<(), String> {
        Err("unscripted write_file".into())
    }
    async fn write_file_bytes(&self, _m: &str, _p: &str, _c: &[u8]) -> Result<(), String> {
        Err("unscripted write_file_bytes".into())
    }
    async fn get_metadata(
        &self,
        _m: &str,
        _p: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        Err("unscripted get_metadata".into())
    }
    async fn list_dir(
        &self,
        _m: &str,
        _p: &str,
    ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
        Err("unscripted list_dir".into())
    }
    async fn setup_worktree(&self, _m: &str, _r: &str, _b: &str, _s: &str) -> Result<(), String> {
        Err("unscripted setup_worktree".into())
    }
    async fn resolve_home(&self, _m: &str) -> Result<String, String> {
        Err("unscripted resolve_home".into())
    }
    async fn resolve_user(&self, _m: &str) -> Result<String, String> {
        Err("unscripted resolve_user".into())
    }
    async fn control_rpc(
        &self,
        _m: &str,
        _method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("unscripted control_rpc".into())
    }
    fn spawn_interactive(
        &self,
        _m: &str,
        _binary: &str,
        _args: &[String],
        _cwd: &str,
        _env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("unscripted spawn_interactive".into())
    }
}

/// One round trip, and `; true` closes it so the loop's last failing `[ -L … ]`
/// cannot turn the whole answer into an `Err` and silently drop every exclusion.
#[tokio::test]
async fn the_probe_is_one_command_that_cannot_exit_non_zero_on_its_last_test() {
    let exec = ProbeOnlyExec {
        answer: "node_modules\nartifacts\n".to_string(),
        seen: Mutex::new(Vec::new()),
    };
    let paths = resolve_add_exclusions(&exec, "local", "/wt", "artifacts/", false).await;

    assert_eq!(paths, " -- ':!node_modules' ':!artifacts'");
    let seen = exec.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "one round trip, not one per candidate");
    assert!(
        seen[0].ends_with("; true"),
        "the loop's exit status must not decide the probe's: {}",
        seen[0]
    );
    assert!(
        seen[0].starts_with("cd /wt || exit 1;"),
        "`|| exit 1` and not `&&`: the `;`-separated probes would otherwise run \
         in the wrong directory: {}",
        seen[0]
    );
}

/// A probe that could not run keeps the artifact exclusion, so a dead transport
/// can never quietly commit the step's reports into the PR.
#[tokio::test]
async fn a_probe_failure_still_keeps_the_artifact_exclusion() {
    struct DeadExec;
    #[async_trait::async_trait]
    impl crate::ports::execution::ExecutionPort for DeadExec {
        async fn test_connection(&self, _m: &str) -> Result<(), String> {
            Err("dead".into())
        }
        async fn run_command_with(
            &self,
            _m: &str,
            _c: &str,
            _o: ShellOptions,
        ) -> Result<String, String> {
            Err("transport error: connection closed".into())
        }
        async fn read_file(&self, _m: &str, _p: &str) -> Result<String, String> {
            Err("unscripted read_file".into())
        }
        async fn write_file(&self, _m: &str, _p: &str, _c: &str) -> Result<(), String> {
            Err("unscripted write_file".into())
        }
        async fn write_file_bytes(&self, _m: &str, _p: &str, _c: &[u8]) -> Result<(), String> {
            Err("unscripted write_file_bytes".into())
        }
        async fn get_metadata(
            &self,
            _m: &str,
            _p: &str,
        ) -> Result<crate::ports::execution::SftpEntry, String> {
            Err("unscripted get_metadata".into())
        }
        async fn list_dir(
            &self,
            _m: &str,
            _p: &str,
        ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
            Err("unscripted list_dir".into())
        }
        async fn setup_worktree(
            &self,
            _m: &str,
            _r: &str,
            _b: &str,
            _s: &str,
        ) -> Result<(), String> {
            Err("unscripted setup_worktree".into())
        }
        async fn resolve_home(&self, _m: &str) -> Result<String, String> {
            Err("unscripted resolve_home".into())
        }
        async fn resolve_user(&self, _m: &str) -> Result<String, String> {
            Err("unscripted resolve_user".into())
        }
        async fn control_rpc(
            &self,
            _m: &str,
            _method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Err("unscripted control_rpc".into())
        }
        fn spawn_interactive(
            &self,
            _m: &str,
            _binary: &str,
            _args: &[String],
            _cwd: &str,
            _env: &std::collections::HashMap<String, String>,
        ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
            Err("unscripted spawn_interactive".into())
        }
    }

    assert_eq!(
        resolve_add_exclusions(&DeadExec, "local", "/wt", "artifacts/", false).await,
        " -- ':!artifacts'"
    );
    assert_eq!(
        resolve_add_exclusions(&DeadExec, "local", "/wt", "artifacts/", true).await,
        "",
        "a caller committing its artifacts excludes nothing"
    );
}
