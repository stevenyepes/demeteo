// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/sequence/resume.rs` (mirrored-tests convention). `super` = that module.
//
// What `probe_anchor` reports for each thing git can say, driven through
// the real async path against a scripted `ExecutionPort`.
//
// This is the half of the crash-resume logic that a pure test cannot
// reach: `classify` decides what a verdict *means*, but something has to
// turn two git invocations into that verdict, and getting *that* wrong
// resets a fresh worktree backwards onto a stale commit just as
// effectively.
//
// The e2e suite cannot stand in for it. Its `FakeExec` answers every
// command `Ok("")`, so `merge-base` reads as "some other commit" and every
// checkpoint classifies as `Stranded` — one arm of four, with the step
// running green either way.

use std::sync::Mutex;

use super::*;
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry, ShellOptions};

const ANCHOR: &str = "1111111111111111111111111111111111111111";
const BASE: &str = "2222222222222222222222222222222222222222";

/// Answers commands by substring match, in insertion order, and records
/// what it was asked — so a test can assert the shape of what went over the
/// wire, quoting included.
///
/// Unscripted commands are an `Err` naming the command rather than a silent
/// `Ok("")`. A default that looks like a successful answer is the specific
/// hazard this double exists to remove.
#[derive(Default)]
struct ScriptedExec {
    rules: Mutex<Vec<(String, Result<String, String>)>>,
    issued: Mutex<Vec<String>>,
}

impl ScriptedExec {
    fn new() -> Self {
        Self::default()
    }

    fn ok(self, pattern: &str, stdout: &str) -> Self {
        self.push(pattern, Ok(stdout.to_string()))
    }

    /// Git's non-zero exit, as `ExecutionPort` flattens it: `Err(stderr)`.
    fn err(self, pattern: &str, stderr: &str) -> Self {
        self.push(pattern, Err(stderr.to_string()))
    }

    fn push(self, pattern: &str, answer: Result<String, String>) -> Self {
        self.rules
            .lock()
            .expect("rules")
            .push((pattern.to_string(), answer));
        self
    }

    fn issued(&self) -> Vec<String> {
        self.issued.lock().expect("issued").clone()
    }
}

#[async_trait::async_trait]
impl ExecutionPort for ScriptedExec {
    async fn run_command(&self, _machine_id: &str, cmd: &str) -> Result<String, String> {
        self.issued.lock().expect("issued").push(cmd.to_string());
        let rules = self.rules.lock().expect("rules");
        match rules.iter().find(|(pattern, _)| cmd.contains(pattern)) {
            Some((_, answer)) => answer.clone(),
            None => Err(format!("ScriptedExec: no rule matches: {cmd}")),
        }
    }

    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn get_metadata(&self, _: &str, _: &str) -> Result<SftpEntry, String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        unreachable!("probe_anchor only runs commands")
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        unreachable!("probe_anchor only runs commands")
    }
    async fn run_command_with(
        &self,
        machine_id: &str,
        cmd: &str,
        _: ShellOptions,
    ) -> Result<String, String> {
        self.run_command(machine_id, cmd).await
    }
}

async fn probe(exec: &ScriptedExec) -> checkpoint::AnchorProbe {
    probe_anchor(
        exec,
        "local",
        "/repo",
        ANCHOR,
        BASE,
        ProbeLog {
            feature_id: "f-1",
            step_id: "s-impl",
        },
    )
    .await
}

/// `merge-base` printing the anchor itself is the only evidence that the
/// prefix already reached the feature branch.
#[tokio::test]
async fn the_merge_base_being_the_anchor_reads_as_merged() {
    let exec = ScriptedExec::new()
        .ok("cat-file", "")
        .ok("merge-base", &format!("{ANCHOR}\n"));
    assert_eq!(probe(&exec).await, checkpoint::AnchorProbe::Merged);
}

/// An *earlier* common ancestor means the anchor is off on the step branch:
/// the crash shape, and the only verdict that may reset a worktree.
#[tokio::test]
async fn an_earlier_merge_base_reads_as_stranded() {
    let exec = ScriptedExec::new()
        .ok("cat-file", "")
        .ok("merge-base", "3333333333333333333333333333333333333333\n");
    assert_eq!(probe(&exec).await, checkpoint::AnchorProbe::Stranded);
}

/// The anchor commit is gone — the ref was deleted, or the repo was
/// replaced. Nothing to resume onto, and `merge-base` is never asked.
#[tokio::test]
async fn a_missing_anchor_short_circuits_before_the_merge_base() {
    let exec = ScriptedExec::new().err("cat-file", "fatal: Not a valid object name");
    assert_eq!(probe(&exec).await, checkpoint::AnchorProbe::Missing);
    assert_eq!(
        exec.issued().len(),
        1,
        "a vanished anchor must not go on to ask where it merges: {:?}",
        exec.issued()
    );
}

/// The failure the whole probe shape exists to prevent. Git could not
/// answer, and that is **not** "not merged": a wrong `Stranded` resets a
/// worktree backwards past work that was merged, where a wrong `Unknown`
/// only re-runs tasks that were already paid for.
#[tokio::test]
async fn an_unanswerable_merge_base_reads_as_unknown_not_stranded() {
    let exec = ScriptedExec::new().ok("cat-file", "").err(
        "merge-base",
        "fatal: refusing to work with unrelated histories",
    );
    assert_eq!(probe(&exec).await, checkpoint::AnchorProbe::Unknown);
}

/// Empty stdout is the shape `FakeExec` hands every command, and it must
/// not read as a verdict. Under the old always-`Ok("")` double this cell
/// silently answered `Stranded` — a fresh worktree reset onto a stale
/// commit — while the step still ran green.
#[tokio::test]
async fn empty_merge_base_output_is_not_a_verdict() {
    let exec = ScriptedExec::new().ok("cat-file", "").ok("merge-base", "");
    assert_eq!(
        probe(&exec).await,
        checkpoint::AnchorProbe::Stranded,
        "empty output is a *different* commit, not a match — the caller's \
         safety comes from `classify`, not from pretending this is unknown"
    );
}

/// Git prints lowercase hex; a checkpoint written from a differently-cased
/// source must still match rather than resolving to `Stranded`.
#[tokio::test]
async fn the_comparison_ignores_case_and_surrounding_whitespace() {
    let exec = ScriptedExec::new()
        .ok("cat-file", "")
        .ok("merge-base", &format!("  {}  \n", ANCHOR.to_uppercase()));
    assert_eq!(probe(&exec).await, checkpoint::AnchorProbe::Merged);
}

/// Both probes go through `shell_escape_posix`. The SHAs and the repo path
/// here need no quoting, so the assertion is that they arrive *unmangled* —
/// the regression this guards is a path or ref that does need quoting being
/// interpolated raw.
#[tokio::test]
async fn both_probes_name_the_repo_and_the_anchor() {
    let exec = ScriptedExec::new()
        .ok("cat-file", "")
        .ok("merge-base", &format!("{ANCHOR}\n"));
    probe(&exec).await;

    let issued = exec.issued();
    assert_eq!(issued.len(), 2, "{issued:?}");
    assert_eq!(
        issued[0],
        format!("git -C /repo cat-file -e {ANCHOR}^{{commit}}")
    );
    assert_eq!(
        issued[1],
        format!("git -C /repo merge-base {ANCHOR} {BASE}")
    );
}
