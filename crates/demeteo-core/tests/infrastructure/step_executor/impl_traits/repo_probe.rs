// Tests for `src/adapters/step_executor/impl_traits/execution_context/repo_probe.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// The probe reads exactly one port, which is the whole reason these exist: the
// remediation sentence a user meets when a workspace was never bootstrapped was
// unreachable while this sat inside a 305-line `async fn` on `DagStepExecutor`.

use super::*;
use crate::ports::execution::ShellOptions;
use std::collections::HashMap;
use std::sync::Mutex;

/// An `ExecutionPort` that answers **one** command, for **one** machine, and
/// errors on anything else — including a second call. The probe is a
/// single-shot check, so "it ran twice" and "it ran against the wrong machine"
/// are both failures a permissive double would hide.
struct OneShotExec {
    machine: String,
    answer: Mutex<Option<Result<String, String>>>,
    seen: Mutex<Vec<String>>,
}

impl OneShotExec {
    fn answering(machine: &str, answer: Result<&str, &str>) -> Self {
        Self {
            machine: machine.to_string(),
            answer: Mutex::new(Some(match answer {
                Ok(s) => Ok(s.to_string()),
                Err(e) => Err(e.to_string()),
            })),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.seen.lock().expect("not poisoned").clone()
    }

    fn only_call(&self) -> String {
        let calls = self.calls();
        assert_eq!(calls.len(), 1, "the probe runs exactly one command");
        calls[0].clone()
    }
}

#[async_trait::async_trait]
impl ExecutionPort for OneShotExec {
    async fn test_connection(&self, _m: &str) -> Result<(), String> {
        Err("unscripted test_connection".into())
    }
    async fn run_command_with(
        &self,
        m: &str,
        cmd: &str,
        _o: ShellOptions,
    ) -> Result<String, String> {
        self.seen
            .lock()
            .expect("not poisoned")
            .push(cmd.to_string());
        assert_eq!(m, self.machine, "the probe must run on the target machine");
        self.answer
            .lock()
            .expect("not poisoned")
            .take()
            .unwrap_or_else(|| Err("OneShotExec: a second command was run".into()))
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
        _env: &HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("unscripted spawn_interactive".into())
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a current-thread runtime")
        .block_on(f)
}

// ── the verdict ─────────────────────────────────────────────────────────────

/// The marker is the verdict, and it is found in the middle of whatever the
/// login shell printed around it — which is why the probe echoes a marker at
/// all rather than trusting an exit status.
#[test]
fn output_carrying_the_marker_is_a_present_repository() {
    let exec = OneShotExec::answering(
        "builder-01",
        Ok("__DEMETEO_DIAG__ home=\"/home/x\" pwd=\"/\"\n\
            total 4\ndrwxr-xr-x 3 x x 4096 Jan 1 00:00 repo\n\
            __DEMETEO_DIAG__ exists\n"),
    );

    assert_eq!(
        block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo")),
        Ok(())
    );
}

/// The failure a user actually meets. Every part is load-bearing: the machine
/// they were working against, the path that was missing, the probe's own output
/// verbatim (so the empty parent listing is visible), and the one action that
/// fixes it.
#[test]
fn a_missing_repository_reports_the_probe_verbatim_and_what_to_do() {
    let exec = OneShotExec::answering(
        "builder-01",
        Ok("__DEMETEO_DIAG__ home=\"/home/x\" pwd=\"/\"\n\
            total 0\n__DEMETEO_DIAG__ missing\n"),
    );

    assert_eq!(
        block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo")),
        Err(
            "Repository target dir does not exist on 'builder-01': /w/p/repo\n\
             Remote diagnostic probe output:\n\
             __DEMETEO_DIAG__ home=\"/home/x\" pwd=\"/\"\n\
             total 0\n__DEMETEO_DIAG__ missing\n\n\n\
             If the parent dir listing is empty, the bootstrap clone \
             did not actually run for this project — re-save the \
             workspace settings to trigger a fresh bootstrap."
                .to_string()
        )
    );
}

/// A transport failure is not a silent pass. The port's error becomes the probe
/// output, so the message that reaches the user names the connection problem
/// rather than claiming the directory is missing with nothing to show for it.
#[test]
fn a_failed_port_call_still_reaches_the_user_with_its_reason() {
    let exec = OneShotExec::answering("builder-01", Err("ssh: connection refused"));

    let err = block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo"))
        .expect_err("an unreachable machine cannot prove the repo is there");

    assert!(
        err.contains("probe failed: ssh: connection refused"),
        "the port's own error must survive into the message: {err}"
    );
    assert!(err.contains("re-save the workspace settings"));
}

// ── the command that gets sent ──────────────────────────────────────────────

/// One `run_command`, on the machine that will do the work. The probe is the
/// same on every transport — a branch here would be testing something other
/// than what the run is about to do (AGENTS.md §2).
#[test]
fn the_probe_asks_about_the_target_and_lists_its_parent() {
    let exec = OneShotExec::answering("local", Ok("__DEMETEO_DIAG__ exists"));
    let _ = block_on(verify_repo_present(&exec, "local", "/w/p/repo"));

    let cmd = exec.only_call();
    assert!(
        cmd.contains("ls -la /w/p "),
        "parent listing missing: {cmd}"
    );
    assert!(cmd.contains("test -d /w/p/repo "), "target check: {cmd}");
    assert!(cmd.contains("__DEMETEO_DIAG__ exists"));
    assert!(cmd.contains("__DEMETEO_DIAG__ missing"));
}

/// Both paths go through `shell_escape_posix`. A workspace under a directory
/// with a space is ordinary on macOS, and an unescaped one would make the probe
/// list the wrong directory and then report the repo missing.
#[test]
fn both_paths_are_shell_escaped() {
    let exec = OneShotExec::answering("local", Ok("__DEMETEO_DIAG__ exists"));
    let _ = block_on(verify_repo_present(&exec, "local", "/w/my projects/re'po"));

    let cmd = exec.only_call();
    assert!(
        cmd.contains(&crate::paths::shell_escape_posix("/w/my projects")),
        "parent not escaped: {cmd}"
    );
    assert!(
        cmd.contains(&crate::paths::shell_escape_posix("/w/my projects/re'po")),
        "target not escaped: {cmd}"
    );
}

/// A target at the filesystem root has no parent. The probe must still run —
/// degrading to an empty listing, not to a panic on `Path::parent`.
#[test]
fn a_target_with_no_parent_still_probes() {
    let exec = OneShotExec::answering("local", Ok("__DEMETEO_DIAG__ exists"));

    assert_eq!(block_on(verify_repo_present(&exec, "local", "/")), Ok(()));
    assert!(exec.only_call().contains("test -d / &&"));
}
