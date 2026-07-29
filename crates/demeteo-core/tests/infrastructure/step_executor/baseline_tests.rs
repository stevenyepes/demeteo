// Tests extracted from `src/adapters/step_executor/baseline.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// [`measure_gates`] is a free function over one port precisely so it is
// reachable here without an `ExecutionDriver` and its twenty-odd unread ports
// (AGENTS.md §3). What the tests below protect is a single asymmetry: an
// *absent* baseline degrades to today's behaviour, while a *fabricated* one
// inverts HB2c's decision table and excuses a real regression. So every
// ambiguous input must record nothing rather than something plausible.
//
// The end-to-end wiring — which producer writes, at which sha, and that the
// worktree is torn down — is in `tests/conformance/harness_baseline.rs`, which
// runs a real shell.

use super::*;
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// An `ExecutionPort` double that **errors on anything it was not explicitly
/// told to answer**. AGENTS.md §7 names the opposite shape — the e2e
/// `FakeExec` answering `Ok("")` for everything — as what makes a suite unable
/// to fail: a gate asserted against a default is asserted against nothing.
struct ScriptedExec {
    answers: HashMap<String, Result<String, String>>,
    /// `(command, timeout)` in call order — the deadline is per harness, so
    /// which ceiling each command was given is part of what is under test.
    seen: Mutex<Vec<(String, Option<Duration>)>>,
}

impl ScriptedExec {
    fn new(answers: &[(&str, Result<&str, &str>)]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(k, v)| {
                    (
                        wrapped(k),
                        match v {
                            Ok(s) => Ok(s.to_string()),
                            Err(e) => Err(e.to_string()),
                        },
                    )
                })
                .collect(),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn commands(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(c, _)| c.clone())
            .collect()
    }
}

/// The `( … ) 2>&1` shape every baseline command is wrapped in, so a test can
/// script an answer against the command it authored rather than against the
/// command the adapter is handed.
fn wrapped(cmd: &str) -> String {
    crate::adapters::step_executor::driver::verifier::merge_stderr_into_stdout(cmd)
}

#[async_trait::async_trait]
impl ExecutionPort for ScriptedExec {
    async fn test_connection(&self, _m: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_command_with(
        &self,
        _m: &str,
        cmd: &str,
        o: ShellOptions,
    ) -> Result<String, String> {
        self.seen.lock().unwrap().push((cmd.to_string(), o.timeout));
        self.answers
            .get(cmd)
            .cloned()
            .unwrap_or_else(|| Err(format!("ScriptedExec: unscripted command `{cmd}`")))
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

/// The worktree every test measures in. Its path is what the fingerprint is
/// normalized against, so it has to be the same one the assertions use.
fn site(producer: BaselineProducer) -> BaselineSite<'static> {
    BaselineSite {
        machine: "local",
        wt_path: "/repo_wt_baseline",
        step_id: "s-validate",
        base_sha: "abc123",
        producer,
    }
}

fn gate(name: &str, command: &str, deadline_s: u64) -> ResolvedHarness {
    ResolvedHarness {
        name: name.to_string(),
        command: command.to_string(),
        deadline_s,
    }
}

/// Run `measure_gates` against a scripted port with the defaults every test
/// shares.
async fn measure(
    exec: &ScriptedExec,
    prepare: Option<&str>,
    harnesses: &[ResolvedHarness],
) -> Vec<MeasuredGate> {
    measure_gates(
        exec,
        &site(BaselineProducer::Node),
        prepare,
        harnesses,
        ShellOptions::login_interactive(),
        1_700_000_000,
    )
    .await
}

// ── What a gate said ─────────────────────────────────────────────────────────

#[tokio::test]
async fn a_green_gate_is_recorded_green_with_no_fingerprint() {
    let exec = ScriptedExec::new(&[("npm test", Ok("42 passing"))]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    assert_eq!(measured.len(), 1);
    assert!(measured[0].run.exit_ok);
    assert!(
        measured[0].run.fingerprint.is_empty(),
        "there is no failure to fingerprint on a green gate"
    );
    assert_eq!(measured[0].output, "42 passing");
    assert_eq!(measured[0].run.name, "unit");
    // Recorded as the user authored it, not as the wrapper the port was
    // handed: HB2c compares this string against validate's, and the two could
    // never match if one carried the redirection and the other did not.
    assert_eq!(measured[0].run.command, "npm test");
}

#[tokio::test]
async fn a_red_gate_is_recorded_red_with_a_fingerprint() {
    let exec = ScriptedExec::new(&[("npm test", Err("1 failing\n  auth spec"))]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    assert_eq!(measured.len(), 1);
    assert!(!measured[0].run.exit_ok);
    assert!(
        measured[0].run.fingerprint.contains("auth spec"),
        "the fingerprint must carry the failure: {}",
        measured[0].run.fingerprint
    );
}

#[tokio::test]
async fn the_fingerprint_is_built_the_same_way_the_live_failure_path_builds_it() {
    // HB2c compares a baseline fingerprint against a live one. If the two are
    // computed over differently-shaped strings the comparison can only ever be
    // false, which silently disables the subtraction rather than breaking it.
    let output = "1 failing";
    let exec = ScriptedExec::new(&[("npm test", Err(output))]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    let live = crate::adapters::step_executor::driver::verifier::normalize_failure_fingerprint(
        &crate::adapters::step_executor::driver::verifier::harness_block(
            "unit", "npm test", output,
        ),
        "/repo_wt_baseline",
    );
    assert_eq!(measured[0].run.fingerprint, live);
}

#[tokio::test]
async fn every_gate_runs_even_after_one_of_them_is_red() {
    // The same rule the live path follows (HB5): a baseline that stopped at
    // the first red gate would leave the later ones unmeasured, and an
    // unmeasured gate gets no subtraction at all.
    let exec = ScriptedExec::new(&[
        ("npm run lint", Err("lint blew up")),
        ("npm test", Ok("green")),
    ]);
    let measured = measure(
        &exec,
        None,
        &[
            gate("lint", "npm run lint", 600),
            gate("unit", "npm test", 600),
        ],
    )
    .await;

    let names: Vec<&str> = measured.iter().map(|m| m.run.name.as_str()).collect();
    assert_eq!(names, vec!["lint", "unit"]);
}

// ── What must never be recorded ──────────────────────────────────────────────

#[tokio::test]
async fn a_failed_prepare_records_nothing_at_all() {
    // The most dangerous thing this module could do. A suite run without its
    // install step fails for reasons that have nothing to do with the base
    // commit — and every such gate would land in the record as red-at-base,
    // which is precisely the shape that excuses a real regression later.
    let exec = ScriptedExec::new(&[
        ("npm ci", Err("ENOENT")),
        ("npm test", Err("cannot find module")),
    ]);
    let measured = measure(&exec, Some("npm ci"), &[gate("unit", "npm test", 600)]).await;

    assert!(measured.is_empty(), "no gate may be recorded: {measured:?}");
    assert_eq!(
        exec.commands(),
        vec![wrapped("npm ci")],
        "the harness must not even be attempted once prepare has failed"
    );
}

#[tokio::test]
async fn a_transport_failure_omits_the_gate_rather_than_recording_it_red() {
    // The machine went away, so the command never reached a verdict. Recording
    // it red at the base would excuse whatever it does at the tip — the same
    // reason `run_harness_first` refuses to call this a verdict.
    let exec = ScriptedExec::new(&[(
        "npm test",
        Err(&format!("{TRANSPORT_ERROR_PREFIX} channel closed")),
    )]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    assert!(
        measured.is_empty(),
        "no evidence means no record: {measured:?}"
    );
}

#[tokio::test]
async fn a_timeout_omits_the_gate_rather_than_recording_it_red() {
    let exec = ScriptedExec::new(&[(
        "npm test",
        Err(&format!("{TIMEOUT_ERROR_PREFIX} exceeded 600s")),
    )]);
    let measured = measure(&exec, None, &[gate("unit", "npm test", 600)]).await;

    assert!(
        measured.is_empty(),
        "an abandoned command is not a red one: {measured:?}"
    );
}

#[tokio::test]
async fn one_unreachable_gate_does_not_discard_the_others() {
    let exec = ScriptedExec::new(&[
        (
            "npm run lint",
            Err(&format!("{TRANSPORT_ERROR_PREFIX} gone")),
        ),
        ("npm test", Err("1 failing")),
    ]);
    let measured = measure(
        &exec,
        None,
        &[
            gate("lint", "npm run lint", 600),
            gate("unit", "npm test", 600),
        ],
    )
    .await;

    let names: Vec<&str> = measured.iter().map(|m| m.run.name.as_str()).collect();
    assert_eq!(names, vec!["unit"]);
}

// ── How the commands are run ─────────────────────────────────────────────────

#[tokio::test]
async fn prepare_runs_first_and_is_not_recorded_as_a_gate() {
    // `prepare` makes the worktree runnable; it is not one of the gates HB2c
    // subtracts, and recording it as one would put a name in the record that
    // no live `HarnessRun` can ever join against.
    let exec = ScriptedExec::new(&[("npm ci", Ok("added 900 packages")), ("npm test", Ok("ok"))]);
    let measured = measure(&exec, Some("npm ci"), &[gate("unit", "npm test", 600)]).await;

    assert_eq!(
        exec.commands(),
        vec![wrapped("npm ci"), wrapped("npm test")]
    );
    let names: Vec<&str> = measured.iter().map(|m| m.run.name.as_str()).collect();
    assert_eq!(names, vec!["unit"]);
}

#[tokio::test]
async fn a_blank_prepare_command_is_skipped_rather_than_run() {
    let exec = ScriptedExec::new(&[("npm test", Ok("ok"))]);
    let measured = measure(&exec, Some("   "), &[gate("unit", "npm test", 600)]).await;

    assert_eq!(exec.commands(), vec![wrapped("npm test")]);
    assert_eq!(measured.len(), 1);
}

#[tokio::test]
async fn each_gate_is_given_its_own_deadline() {
    // Per harness, not per step (HB5/S10). A gate's ceiling must not depend on
    // how many other gates a workflow happens to declare.
    let exec = ScriptedExec::new(&[("npm run lint", Ok("ok")), ("npm test", Ok("ok"))]);
    measure(
        &exec,
        None,
        &[
            gate("lint", "npm run lint", 60),
            gate("unit", "npm test", 900),
        ],
    )
    .await;

    let deadlines: Vec<Option<Duration>> =
        exec.seen.lock().unwrap().iter().map(|(_, t)| *t).collect();
    assert_eq!(
        deadlines,
        vec![
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(900))
        ]
    );
}

#[tokio::test]
async fn every_command_merges_stderr_into_stdout() {
    // D3: the port yields stdout only on success, and the suites this codebase
    // runs report heavily on stderr — so a *green* gate would otherwise be
    // recorded, and later fingerprinted, against an empty string.
    let exec = ScriptedExec::new(&[("npm ci", Ok("")), ("cargo test", Ok(""))]);
    measure(&exec, Some("npm ci"), &[gate("unit", "cargo test", 600)]).await;

    for cmd in exec.commands() {
        assert!(
            cmd.ends_with(") 2>&1"),
            "every baseline command owes the stderr wrap: {cmd}"
        );
    }
}

#[tokio::test]
async fn each_gate_carries_the_producer_and_the_measurement_time() {
    // Provenance is per gate, not per record (HB2a): one record can legitimately
    // hold gates measured by both producers at different times.
    let exec = ScriptedExec::new(&[("npm test", Ok("ok"))]);
    let measured = measure_gates(
        &exec,
        &site(BaselineProducer::Fallback),
        None,
        &[gate("unit", "npm test", 600)],
        ShellOptions::login_interactive(),
        1_700_000_042,
    )
    .await;

    assert_eq!(measured[0].run.producer, BaselineProducer::Fallback);
    assert_eq!(measured[0].run.measured_at, 1_700_000_042);
    assert!(
        measured[0].run.output_ref.is_none(),
        "the output reference is the caller's to fill in once it has stored it"
    );
}

#[tokio::test]
async fn no_harnesses_and_no_prepare_touches_the_port_at_all() {
    let exec = ScriptedExec::new(&[]);
    let measured = measure(&exec, None, &[]).await;

    assert!(measured.is_empty());
    assert!(exec.commands().is_empty());
}
