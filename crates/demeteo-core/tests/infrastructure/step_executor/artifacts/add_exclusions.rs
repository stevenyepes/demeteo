//! The `git add` pathspec, against a real repo and against a strict double.
//!
//! The `.gitignore` cases genuinely need git's own ignore semantics, so they
//! drive `LocalSubprocessAdapter` end to end through `commit_worktree_changes`.
//! The shape of the probe command itself is asserted separately, against a
//! double that answers the probe and **errors on any other command**.
//!
//! The dependency-cache half of this used to live here as a pathspec and now
//! lives in the clone's `.git/info/exclude`, written at provisioning time. Two
//! cases below still cover it from this side, because this is where the damage
//! would show: they assert that the entry keeps the symlink out of the commit,
//! and that without the entry it lands in it.

use super::*;
use crate::adapters::step_executor::artifacts::commit_worktree_changes;
use crate::ports::execution::ShellOptions;
use std::sync::Mutex;

/// Set up `temp` as a worktree that has a dependency-cache symlink
/// standing in for `node_modules` (what `provision_subtask_worktree`
/// leaves behind), with `ignore_line` as the repo's whole `.gitignore`,
/// one committed file, and one uncommitted agent write.
#[cfg(unix)]
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
// The linked cache is a symlink, which Demeteo only creates on Unix — see
// `git_ops::worktree::share_dependency_caches` for the stated Windows gap.
#[cfg(unix)]
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

/// The case the `.git/info/exclude` entry exists for: a trailing-slash
/// `.gitignore` pattern (`node_modules/`) does NOT match a symlink of that
/// name, so git sees the linked cache as untracked and would stage an absolute
/// host path onto the feature branch.
///
/// `git_ops::worktree` writes the slashless name into the clone's own exclude
/// file before it links anything; from here, `git add -A` simply never sees the
/// symlink and no pathspec is involved.
#[cfg(unix)]
#[tokio::test]
async fn an_excluded_cache_symlink_needs_no_pathspec_to_stay_out_of_the_commit() {
    let temp = temp_git_repo("commit_worktree_excluded_symlink");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    repo_with_node_modules_symlink(&exec, &temp, "node_modules/\n").await;
    write_cache_exclusion(&exec, &temp, "node_modules").await;

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

    let committed = committed_files(&exec, &temp).await;
    assert!(
        committed.contains("src.rs"),
        "the agent's deliverable is committed, got: {committed}"
    );
    assert!(
        !committed.contains("node_modules"),
        "the cache symlink is excluded by .git/info/exclude, got: {committed}"
    );

    let _ = std::fs::remove_dir_all(&temp);
    let _ = std::fs::remove_dir_all(format!("{temp}_cache"));
}

/// What the exclusion is worth, stated as the failure it prevents: the same
/// repository without the entry commits the symlink — an absolute host path —
/// onto the feature branch.
///
/// This is why `share_dependency_caches` writes the exclusion *before* it links
/// and abandons the link when the write fails. Nothing downstream compensates.
#[cfg(unix)]
#[tokio::test]
async fn an_unexcluded_cache_symlink_is_committed_as_an_absolute_host_path() {
    let temp = temp_git_repo("commit_worktree_unexcluded_symlink");
    let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
    let machine = "local";

    repo_with_node_modules_symlink(&exec, &temp, "node_modules/\n").await;

    commit_worktree_changes(
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

    assert!(
        committed_files(&exec, &temp).await.contains("node_modules"),
        "without the exclusion the symlink is staged — the link must not be made without it"
    );

    let _ = std::fs::remove_dir_all(&temp);
    let _ = std::fs::remove_dir_all(format!("{temp}_cache"));
}

/// Write `name` into the repository's `.git/info/exclude` the way
/// `git_ops::worktree::record_cache_exclusions` does.
#[cfg(unix)]
async fn write_cache_exclusion(
    exec: &crate::adapters::local::execution::LocalSubprocessAdapter,
    repo: &str,
    name: &str,
) {
    exec.write_file("local", &format!("{repo}/.git/info/exclude"), name)
        .await
        .unwrap();
}

#[cfg(unix)]
async fn committed_files(
    exec: &crate::adapters::local::execution::LocalSubprocessAdapter,
    repo: &str,
) -> String {
    exec.run_command(
        "local",
        &format!(
            "git -C {} show --stat --name-only --format= HEAD",
            shell_esc(repo)
        ),
    )
    .await
    .unwrap()
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

/// One round trip, and `; true` closes it so the trailing `[ $? -eq 1 ]`
/// cannot turn the whole answer into an `Err` and silently drop the exclusion.
#[tokio::test]
async fn the_probe_is_one_command_that_cannot_exit_non_zero_on_its_last_test() {
    let exec = ProbeOnlyExec {
        answer: "artifacts\n".to_string(),
        seen: Mutex::new(Vec::new()),
    };
    let paths = resolve_add_exclusions(&exec, "local", "/wt", "artifacts/", false).await;

    assert_eq!(paths, " -- ':!artifacts'");
    let seen = exec.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "one round trip");
    assert!(
        seen[0].ends_with("; true"),
        "the last test's exit status must not decide the probe's: {}",
        seen[0]
    );
    assert!(
        seen[0].starts_with("cd /wt || exit 1;"),
        "`|| exit 1` and not `&&`: the `;`-separated probe would otherwise run \
         in the wrong directory: {}",
        seen[0]
    );
    assert!(
        !seen[0].contains("node_modules"),
        "the dependency caches are excluded by .git/info/exclude, not by a pathspec: {}",
        seen[0]
    );
}

/// A caller that commits its artifacts has nothing to exclude, so it must not
/// spend a round trip finding that out — the `ProbeOnlyExec` errors on
/// everything, so any command at all fails this.
#[tokio::test]
async fn nothing_to_exclude_costs_no_round_trip() {
    let exec = ProbeOnlyExec {
        answer: String::new(),
        seen: Mutex::new(Vec::new()),
    };
    assert_eq!(
        resolve_add_exclusions(&exec, "local", "/wt", "artifacts/", true).await,
        ""
    );
    assert_eq!(
        resolve_add_exclusions(&exec, "local", "/wt", "", false).await,
        ""
    );
    assert!(exec.seen.lock().unwrap().is_empty());
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
