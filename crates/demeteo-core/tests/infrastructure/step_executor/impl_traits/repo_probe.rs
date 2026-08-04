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

/// An `ExecutionPort` whose filesystem answers are scripted per path and which
/// **errors on everything else**, including any shell command. A permissive
/// double would hide the two failures that matter here: a probe that asked
/// about the wrong machine, and a probe that reached for a shell at all
/// (AGENTS.md §7).
struct FsExec {
    machine: String,
    metadata: HashMap<String, Result<SftpEntry, String>>,
    listings: HashMap<String, Result<Vec<SftpEntry>, String>>,
    home: Result<String, String>,
    seen: Mutex<Vec<String>>,
}

fn entry(name: &str, is_dir: bool) -> SftpEntry {
    SftpEntry {
        name: name.to_string(),
        path: name.to_string(),
        is_dir,
        size: 0,
        modified: 0,
    }
}

impl FsExec {
    fn new(machine: &str) -> Self {
        Self {
            machine: machine.to_string(),
            metadata: HashMap::new(),
            listings: HashMap::new(),
            home: Ok("/home/x".to_string()),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn with_dir(mut self, path: &str) -> Self {
        self.metadata
            .insert(path.to_string(), Ok(entry("repo", true)));
        self
    }

    fn with_metadata_error(mut self, path: &str, error: &str) -> Self {
        self.metadata
            .insert(path.to_string(), Err(error.to_string()));
        self
    }

    fn with_listing(mut self, path: &str, names: &[(&str, bool)]) -> Self {
        self.listings.insert(
            path.to_string(),
            Ok(names.iter().map(|(n, d)| entry(n, *d)).collect()),
        );
        self
    }

    fn with_listing_error(mut self, path: &str, error: &str) -> Self {
        self.listings
            .insert(path.to_string(), Err(error.to_string()));
        self
    }

    fn calls(&self) -> Vec<String> {
        self.seen.lock().expect("not poisoned").clone()
    }
}

#[async_trait::async_trait]
impl ExecutionPort for FsExec {
    async fn test_connection(&self, _m: &str) -> Result<(), String> {
        Err("unscripted test_connection".into())
    }
    async fn run_command_with(
        &self,
        _m: &str,
        cmd: &str,
        _o: ShellOptions,
    ) -> Result<String, String> {
        panic!("the probe must not run a shell command: `{cmd}`");
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
    async fn get_metadata(&self, m: &str, p: &str) -> Result<SftpEntry, String> {
        self.seen
            .lock()
            .expect("not poisoned")
            .push(format!("get_metadata {p}"));
        assert_eq!(m, self.machine, "the probe must ask the target machine");
        self.metadata
            .get(p)
            .cloned()
            .unwrap_or_else(|| Err(format!("Failed to stat '{p}': No such file or directory")))
    }
    async fn list_dir(&self, m: &str, p: &str) -> Result<Vec<SftpEntry>, String> {
        self.seen
            .lock()
            .expect("not poisoned")
            .push(format!("list_dir {p}"));
        assert_eq!(m, self.machine, "the probe must ask the target machine");
        self.listings
            .get(p)
            .cloned()
            .unwrap_or_else(|| Err(format!("FsExec: unscripted list_dir `{p}`")))
    }
    async fn setup_worktree(&self, _m: &str, _r: &str, _b: &str, _s: &str) -> Result<(), String> {
        Err("unscripted setup_worktree".into())
    }
    async fn resolve_home(&self, _m: &str) -> Result<String, String> {
        self.seen
            .lock()
            .expect("not poisoned")
            .push("resolve_home".to_string());
        self.home.clone()
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

/// A directory at the target is the whole success condition, and it costs one
/// port call: nothing else is read when the repository is there.
#[test]
fn a_directory_at_the_target_is_a_present_repository() {
    let exec = FsExec::new("builder-01").with_dir("/w/p/repo");

    assert_eq!(
        block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo")),
        Ok(())
    );
    assert_eq!(exec.calls(), vec!["get_metadata /w/p/repo".to_string()]);
}

/// A *file* at the target is not a repository. `test -d` said so too; the port
/// answers `Ok` for it, so the `is_dir` test is what preserves the verdict.
#[test]
fn a_file_at_the_target_is_not_a_present_repository() {
    let mut exec = FsExec::new("builder-01").with_listing("/w/p", &[("repo", false)]);
    exec.metadata
        .insert("/w/p/repo".to_string(), Ok(entry("repo", false)));

    let err = block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo"))
        .expect_err("a regular file is not a checkout");
    assert!(
        err.contains("the path exists but is not a directory"),
        "{err}"
    );
}

/// The failure a user actually meets. Every part is load-bearing: the machine
/// they were working against, the path that was missing, what each filesystem
/// call answered (so the empty parent listing is visible), and the one action
/// that fixes it.
#[test]
fn a_missing_repository_reports_what_was_seen_and_what_to_do() {
    let exec = FsExec::new("builder-01").with_listing("/w/p", &[]);

    assert_eq!(
        block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo")),
        Err(
            "Repository target dir does not exist on 'builder-01': /w/p/repo\n\
             Diagnostics:\n  \
             probe failed: Failed to stat '/w/p/repo': No such file or directory\n  \
             home on that machine: /home/x\n  \
             contents of /w/p: (empty)\n\n\
             If the parent dir listing is empty, the bootstrap clone \
             did not actually run for this project — re-save the \
             workspace settings to trigger a fresh bootstrap."
                .to_string()
        )
    );
}

/// A populated parent is rendered entry by entry, dirs marked, so a user can
/// see their *other* projects sitting beside the one that is missing.
#[test]
fn a_populated_parent_is_listed_entry_by_entry() {
    let exec = FsExec::new("builder-01")
        .with_listing("/w/p", &[("other-repo", true), ("notes.md", false)]);

    let err = block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo"))
        .expect_err("the target is still absent");
    assert!(
        err.contains("  contents of /w/p:\n    d other-repo\n    - notes.md\n"),
        "{err}"
    );
}

/// A transport failure is not a silent pass. The port's error becomes the
/// diagnostic, so the message that reaches the user names the connection
/// problem rather than claiming the directory is missing with nothing to show
/// for it.
#[test]
fn a_failed_port_call_still_reaches_the_user_with_its_reason() {
    let exec = FsExec::new("builder-01")
        .with_metadata_error("/w/p/repo", "transport: ssh: connection refused")
        .with_listing_error("/w/p", "transport: ssh: connection refused");

    let err = block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo"))
        .expect_err("an unreachable machine cannot prove the repo is there");

    assert!(
        err.contains("probe failed: transport: ssh: connection refused"),
        "the port's own error must survive into the message: {err}"
    );
    assert!(
        err.contains("could not be read: transport: ssh: connection refused"),
        "the parent's error must survive too: {err}"
    );
    assert!(err.contains("re-save the workspace settings"));
}

/// A machine whose home cannot be resolved still produces the message. The home
/// line explains a workspace root that looks right and points nowhere; it is
/// never a reason to withhold the remediation.
#[test]
fn an_unresolvable_home_degrades_inside_the_message() {
    let mut exec = FsExec::new("builder-01").with_listing("/w/p", &[]);
    exec.home = Err("no passwd entry".to_string());

    let err = block_on(verify_repo_present(&exec, "builder-01", "/w/p/repo"))
        .expect_err("the target is still absent");
    assert!(
        err.contains("home on that machine is unknown: no passwd entry"),
        "{err}"
    );
    assert!(err.contains("re-save the workspace settings"));
}

// ── the calls that get made ─────────────────────────────────────────────────

/// Paths reach the port verbatim. `shell_escape_posix` used to wrap both of
/// them because a shell would otherwise split on the space; structured
/// filesystem calls carry the quotes into the *filename* instead, so escaping
/// here would look for a directory that does not exist.
#[test]
fn paths_reach_the_port_unescaped() {
    let exec = FsExec::new("local").with_listing("/w/my projects", &[]);
    let _ = block_on(verify_repo_present(&exec, "local", "/w/my projects/re'po"));

    assert!(
        exec.calls()
            .contains(&"get_metadata /w/my projects/re'po".to_string()),
        "{:?}",
        exec.calls()
    );
    assert!(
        exec.calls()
            .contains(&"list_dir /w/my projects".to_string()),
        "{:?}",
        exec.calls()
    );
}

/// A target at the filesystem root has no parent. The probe must still answer —
/// degrading to an empty parent string, not to a panic on `Path::parent`.
#[test]
fn a_target_with_no_parent_still_probes() {
    let exec = FsExec::new("local").with_dir("/");

    assert_eq!(block_on(verify_repo_present(&exec, "local", "/")), Ok(()));
    assert_eq!(exec.calls(), vec!["get_metadata /".to_string()]);
}
