// `super` = `adapters::step_executor::sync_worktree`.
//
// The guard is the point: a recursive delete of the main repo checkout is what
// it prevents, and it used to be spelled by each caller rather than by the
// function. Everything here goes through a double that errors on any call it
// was not told to answer, so "it issued something else" is a failure.

use super::*;
use crate::domain::models::Platform;
use crate::ports::execution::{ExecutionPort, SftpEntry, ShellOptions};
use std::collections::HashMap;
use std::sync::Mutex;

const REPO: &str = "/repos/demeteo";
const WT: &str = "/repos/demeteo-sync";

const REMOVE: &str = "git -C /repos/demeteo worktree remove --force /repos/demeteo-sync";
const DELETE: &str = "remove_dir_all /repos/demeteo-sync";
const PRUNE: &str = "git -C /repos/demeteo worktree prune";

/// Records shell commands and filesystem verbs in **one** ordered log, because
/// the ordering across the two is the property under test: unregister, then
/// delete, then prune. It errors on anything unscripted (AGENTS.md §7).
struct TeardownExec {
    answers: HashMap<String, Result<String, String>>,
    seen: Mutex<Vec<String>>,
}

impl TeardownExec {
    fn new(answers: &[(&str, Result<&str, &str>)]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        match v {
                            Ok(s) => Ok((*s).to_string()),
                            Err(e) => Err((*e).to_string()),
                        },
                    )
                })
                .collect(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn answer(&self, key: &str) -> Result<String, String> {
        self.seen.lock().unwrap().push(key.to_string());
        self.answers
            .get(key)
            .cloned()
            .unwrap_or_else(|| Err(format!("TeardownExec: unscripted `{key}`")))
    }

    fn calls(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ExecutionPort for TeardownExec {
    async fn test_connection(&self, _m: &str) -> Result<(), String> {
        Err("unscripted test_connection".into())
    }
    async fn run_command_with(
        &self,
        _m: &str,
        cmd: &str,
        _o: ShellOptions,
    ) -> Result<String, String> {
        self.answer(cmd)
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
    async fn remove_dir_all(&self, _m: &str, path: &str) -> Result<(), String> {
        self.answer(&format!("remove_dir_all {path}")).map(|_| ())
    }
    async fn get_metadata(&self, _m: &str, _p: &str) -> Result<SftpEntry, String> {
        Err("unscripted get_metadata".into())
    }
    async fn list_dir(&self, _m: &str, _p: &str) -> Result<Vec<SftpEntry>, String> {
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
    async fn resolve_platform(&self, _m: &str) -> Result<Platform, String> {
        Err("unscripted resolve_platform".into())
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
        _env: &HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("unscripted spawn_interactive".into())
    }
}

#[tokio::test]
async fn the_three_steps_issue_in_order_against_the_right_paths() {
    let exec = TeardownExec::new(&[(REMOVE, Ok("")), (DELETE, Ok("")), (PRUNE, Ok(""))]);
    discard_sync_worktree(&exec, "local", REPO, WT).await;
    assert_eq!(
        exec.calls(),
        vec![REMOVE.to_string(), DELETE.to_string(), PRUNE.to_string()]
    );
}

#[tokio::test]
async fn a_worktree_that_is_the_repo_itself_is_never_touched() {
    // The guard. Without it this deletes the user's checkout.
    let exec = TeardownExec::new(&[]);
    discard_sync_worktree(&exec, "local", REPO, REPO).await;
    assert!(exec.calls().is_empty(), "issued {:?}", exec.calls());
}

#[tokio::test]
async fn a_failing_remove_still_lets_the_delete_and_the_prune_run() {
    // All three are `let _ =` today: a worktree git refuses to unregister
    // must still be deleted, and the stale entry must still be pruned.
    let exec = TeardownExec::new(&[
        (REMOVE, Err("fatal: is not a working tree")),
        (DELETE, Ok("")),
        (PRUNE, Ok("")),
    ]);
    discard_sync_worktree(&exec, "local", REPO, WT).await;
    assert_eq!(
        exec.calls(),
        vec![REMOVE.to_string(), DELETE.to_string(), PRUNE.to_string()]
    );
}

/// `rm -rf` was silent about a directory `git worktree remove` had already
/// taken; `remove_dir_all` is not. The prune must still run — the stale entry
/// is exactly what is left when the directory went first.
#[tokio::test]
async fn an_already_deleted_directory_still_lets_the_prune_run() {
    let exec = TeardownExec::new(&[
        (REMOVE, Ok("")),
        (DELETE, Err("No such file or directory")),
        (PRUNE, Ok("")),
    ]);
    discard_sync_worktree(&exec, "local", REPO, WT).await;
    assert_eq!(
        exec.calls(),
        vec![REMOVE.to_string(), DELETE.to_string(), PRUNE.to_string()]
    );
}

/// The path reaches the port verbatim. `shell_escape_posix` wrapped it because
/// the shell would otherwise split on the space; the port takes one argument,
/// so quotes carried through here would name a directory that does not exist
/// and leave the real one on disk.
#[tokio::test]
async fn the_worktree_path_reaches_the_port_unescaped() {
    let wt = "/repos/my projects/demeteo-sync";
    let exec = TeardownExec::new(&[(&format!("remove_dir_all {wt}"), Ok(""))]);
    discard_sync_worktree(&exec, "local", REPO, wt).await;
    assert!(
        exec.calls().contains(&format!("remove_dir_all {wt}")),
        "{:?}",
        exec.calls()
    );
}
