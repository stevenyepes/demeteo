// Tests extracted from `src/adapters/step_executor/preflight.rs`
// (mirrored-tests convention). `super` resolves to that module.

use super::*;
use crate::domain::models::WorktreeStrategy;
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};
use std::collections::HashMap;
use std::sync::Mutex;

// ── probe_configured_commands ────────────────────────────────────────────────

/// An `ExecutionPort` double that **errors on anything it was not explicitly
/// told to answer**. AGENTS.md §7 calls out the opposite shape — the e2e
/// `FakeExec` returning `Ok("")` for every command — as the thing that makes a
/// suite unable to fail: a probe asserted against a default is asserted against
/// nothing.
struct ScriptedExec {
    answers: HashMap<String, Result<String, String>>,
    seen: Mutex<Vec<String>>,
}

impl ScriptedExec {
    fn new(answers: &[(&str, Result<&str, &str>)]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
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
        _o: ShellOptions,
    ) -> Result<String, String> {
        self.seen.lock().unwrap().push(cmd.to_string());
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

const T: Duration = Duration::from_secs(5);

/// A `WorktreeStrategy` carrying only what the preflight reads. Spelled out
/// rather than mutated from a default so each test states its whole input:
/// which of the three sources a binary comes from is the entire subject of the
/// HB4 tests below.
fn strategy(
    prepare: Option<&str>,
    test: Option<&str>,
    harnesses: &[(&str, &str)],
) -> WorktreeStrategy {
    WorktreeStrategy {
        default_branch: "main".to_string(),
        branch_prefix: "demeteo/features/".to_string(),
        test_command: test.map(str::to_string),
        build_command: None,
        coverage_command: None,
        conventions_file: None,
        pr_template: None,
        harnesses: (!harnesses.is_empty()).then(|| {
            harnesses
                .iter()
                .map(|(name, cmd)| (name.to_string(), cmd.to_string()))
                .collect()
        }),
        validation_gates: None,
        prepare_command: prepare.map(str::to_string),
        extra_writable_paths: Vec::new(),
    }
}

/// The pre-HB4 shape: only `test_command` configured.
fn test_only(cmd: &str) -> WorktreeStrategy {
    strategy(None, Some(cmd), &[])
}

#[tokio::test]
async fn nothing_configured_at_all_is_not_configured_and_never_touches_the_port() {
    let exec = ScriptedExec::new(&[]);
    let v = probe_configured_commands(&exec, "local", "/repo", &strategy(None, None, &[]), T).await;

    assert_eq!(v, PreflightVerdict::NotConfigured);
    assert!(v.permits_launch(), "an unconfigured harness must not block");
    assert_eq!(v.phase_status(), "skipped");
    assert!(
        exec.seen.lock().unwrap().is_empty(),
        "nothing to probe means nothing should be run"
    );
}

#[tokio::test]
async fn blank_commands_everywhere_are_treated_as_unconfigured() {
    let exec = ScriptedExec::new(&[]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &strategy(Some(" "), Some("   "), &[("unit", "\t\n")]),
        T,
    )
    .await;
    assert_eq!(v, PreflightVerdict::NotConfigured);
    assert!(exec.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn all_binaries_resolving_permits_the_launch() {
    let exec = ScriptedExec::new(&[
        ("command -v npm", Ok("/usr/bin/npm")),
        ("command -v cargo", Ok("/home/u/.cargo/bin/cargo")),
    ]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &test_only("npm test && cargo test"),
        T,
    )
    .await;

    assert_eq!(
        v,
        PreflightVerdict::Resolved {
            probed: vec!["npm".into(), "cargo".into()]
        }
    );
    assert!(v.permits_launch());
    assert_eq!(v.phase_status(), "completed");
}

#[tokio::test]
async fn a_missing_binary_blocks_the_launch_and_names_it() {
    // The whole point of the phase: `cargo` is absent, and today that surfaces
    // as a validate failure after the entire implement budget is spent.
    let exec = ScriptedExec::new(&[
        ("command -v npm", Ok("/usr/bin/npm")),
        ("command -v cargo", Err("Command failed (exit code: 1): ")),
    ]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &test_only("npm test && cargo test"),
        T,
    )
    .await;

    assert_eq!(
        v,
        PreflightVerdict::MissingBinaries {
            missing: vec!["cargo".into()]
        }
    );
    assert!(!v.permits_launch());
    assert_eq!(v.phase_status(), "failed");

    let detail = v.detail().expect("a blocking verdict must explain itself");
    assert!(detail.contains("cargo"));
    assert!(
        detail.contains("bash -l -i -c"),
        "must give the reproduce line in the shell that actually matters; got:\n{detail}"
    );
}

#[tokio::test]
async fn an_empty_command_v_answer_counts_as_missing() {
    // Some shells exit 0 from `command -v` while printing nothing. Trusting the
    // exit code alone would report a missing binary as present.
    let exec = ScriptedExec::new(&[("command -v cargo", Ok("  \n "))]);
    let v = probe_configured_commands(&exec, "local", "/repo", &test_only("cargo test"), T).await;
    assert_eq!(
        v,
        PreflightVerdict::MissingBinaries {
            missing: vec!["cargo".into()]
        }
    );
}

#[tokio::test]
async fn a_transport_failure_never_blocks_the_launch() {
    // The false positive that matters most. A dropped connection must not be
    // read as "your toolchain is missing" — that would refuse to start work
    // over a network blip, which is strictly worse than today's behaviour.
    let exec = ScriptedExec::new(&[(
        "command -v cargo",
        Err(&format!("{TRANSPORT_ERROR_PREFIX}connection reset")),
    )]);
    let v = probe_configured_commands(&exec, "local", "/repo", &test_only("cargo test"), T).await;

    assert!(
        v.permits_launch(),
        "a transport failure is not evidence about the binary; got {v:?}"
    );
}

#[tokio::test]
async fn a_probe_timeout_never_blocks_the_launch() {
    let exec = ScriptedExec::new(&[(
        "command -v cargo",
        Err(&format!(
            "{TIMEOUT_ERROR_PREFIX}command exceeded its 5s ceiling"
        )),
    )]);
    let v = probe_configured_commands(&exec, "local", "/repo", &test_only("cargo test"), T).await;
    assert!(
        v.permits_launch(),
        "a slow probe is not a missing binary; got {v:?}"
    );
}

#[tokio::test]
async fn a_command_of_pure_builtins_asserts_nothing_and_proceeds() {
    let exec = ScriptedExec::new(&[]);
    let v = probe_configured_commands(&exec, "local", "/repo", &test_only("true"), T).await;

    assert_eq!(v, PreflightVerdict::Resolved { probed: vec![] });
    assert!(v.permits_launch());
    assert!(
        v.detail().is_none(),
        "having verified nothing, it should claim nothing"
    );
    assert!(exec.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_binary_named_only_by_prepare_command_is_probed() {
    // Today's gap: `npm ci` naming a binary that isn't there launches happily
    // and dies in `run_harness_first` after the whole implement budget.
    let exec = ScriptedExec::new(&[
        ("command -v pnpm", Err("Command failed (exit code: 1): ")),
        ("command -v cargo", Ok("/home/u/.cargo/bin/cargo")),
    ]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &strategy(Some("pnpm install"), Some("cargo test"), &[]),
        T,
    )
    .await;

    assert_eq!(
        v,
        PreflightVerdict::MissingBinaries {
            missing: vec!["pnpm".into()]
        }
    );
    assert!(!v.permits_launch());
}

#[tokio::test]
async fn a_binary_named_only_by_a_harness_is_probed() {
    // `verifier.harness_name` selects one of these, so probing only
    // `test_command` checks a string the step will never run.
    let exec = ScriptedExec::new(&[
        ("command -v cargo", Ok("/home/u/.cargo/bin/cargo")),
        (
            "command -v pytest",
            Err("Command failed (exit code: 127): "),
        ),
    ]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &strategy(None, Some("cargo test"), &[("integration", "pytest -q")]),
        T,
    )
    .await;

    assert_eq!(
        v,
        PreflightVerdict::MissingBinaries {
            missing: vec!["pytest".into()]
        }
    );
}

#[tokio::test]
async fn a_binary_named_by_two_sources_is_probed_exactly_once() {
    let exec = ScriptedExec::new(&[("command -v npm", Ok("/usr/bin/npm"))]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &strategy(
            Some("npm ci"),
            Some("npm test"),
            &[("lint", "npm run lint")],
        ),
        T,
    )
    .await;

    assert_eq!(
        v,
        PreflightVerdict::Resolved {
            probed: vec!["npm".into()]
        }
    );
    assert_eq!(
        *exec.seen.lock().unwrap(),
        vec!["command -v npm".to_string()],
        "one distinct tool must cost one probe however many commands name it"
    );
}

#[tokio::test]
async fn a_project_configuring_only_harnesses_is_not_reported_as_unconfigured() {
    // `NotConfigured` means *no* harness. A project whose only harnesses are
    // named ones has several, and telling the user otherwise is simply false.
    let exec = ScriptedExec::new(&[("command -v pytest", Ok("/usr/bin/pytest"))]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &strategy(None, None, &[("unit", "pytest -q")]),
        T,
    )
    .await;

    assert_eq!(
        v,
        PreflightVerdict::Resolved {
            probed: vec!["pytest".into()]
        }
    );
    assert_ne!(v, PreflightVerdict::NotConfigured);
    assert_eq!(v.phase_status(), "completed");
}

#[tokio::test]
async fn only_a_prepare_command_is_still_configured() {
    let exec = ScriptedExec::new(&[("command -v npm", Ok("/usr/bin/npm"))]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &strategy(Some("npm ci"), None, &[]),
        T,
    )
    .await;

    assert_eq!(
        v,
        PreflightVerdict::Resolved {
            probed: vec!["npm".into()]
        }
    );
}
#[tokio::test]
async fn the_settings_probe_answers_per_command_over_the_same_port() {
    let exec = ScriptedExec::new(&[
        ("command -v npm", Ok("/usr/bin/npm")),
        ("command -v cargo", Err("Command failed (exit code: 1): ")),
    ]);
    let report = probe_project_commands(
        &exec,
        "local",
        &strategy(Some("npm ci"), Some("cargo test"), &[]),
        T,
    )
    .await;

    assert_eq!(report.machine, "local");
    assert!(report.blocks_launch);
    assert_eq!(
        report
            .commands
            .iter()
            .map(|c| c.binaries.iter().all(|b| b.resolved))
            .collect::<Vec<_>>(),
        vec![true, false]
    );
}

#[tokio::test]
async fn the_settings_probe_reads_no_repository_directory() {
    // It runs before — and often instead of — a checkout on that machine: a
    // project may be configured for a runner it has never been provisioned on.
    // Naming a directory that does not exist there would fail every probe at
    // spawn time and read as a missing toolchain.
    struct CwdSpy(Mutex<Vec<Option<String>>>);
    #[async_trait::async_trait]
    impl ExecutionPort for CwdSpy {
        async fn test_connection(&self, _m: &str) -> Result<(), String> {
            Ok(())
        }
        async fn run_command_with(
            &self,
            _m: &str,
            _cmd: &str,
            o: ShellOptions,
        ) -> Result<String, String> {
            self.0.lock().unwrap().push(o.cwd.clone());
            Ok("/usr/bin/npm".to_string())
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

    let spy = CwdSpy(Mutex::new(Vec::new()));
    let _ = probe_project_commands(&spy, "local", &test_only("npm test"), T).await;
    assert_eq!(
        *spy.0.lock().unwrap(),
        vec![None],
        "the settings probe must leave the working directory to the adapter"
    );
}

#[tokio::test]
async fn a_transport_failure_at_configuration_time_accuses_nothing() {
    // The bias is unchanged by the surface: a network blip must not paint a
    // working toolchain red in the panel any more than it may block a launch.
    let exec = ScriptedExec::new(&[(
        "command -v cargo",
        Err(&format!("{TRANSPORT_ERROR_PREFIX}connection reset")),
    )]);
    let report = probe_project_commands(&exec, "local", &test_only("cargo test"), T).await;

    assert!(!report.blocks_launch);
    assert!(report.commands[0].binaries.iter().all(|b| b.resolved));
}

#[tokio::test]
async fn a_transport_failure_on_a_harness_probe_still_never_blocks() {
    // The bias is unchanged by the wider input set: more probes means more
    // chances for a network blip to be mistaken for a missing toolchain, and
    // it must be mistaken for one no more often than before.
    let exec = ScriptedExec::new(&[
        ("command -v cargo", Ok("/home/u/.cargo/bin/cargo")),
        (
            "command -v pytest",
            Err(&format!("{TRANSPORT_ERROR_PREFIX}connection reset")),
        ),
    ]);
    let v = probe_configured_commands(
        &exec,
        "local",
        "/repo",
        &strategy(None, Some("cargo test"), &[("integration", "pytest -q")]),
        T,
    )
    .await;

    assert!(
        v.permits_launch(),
        "a transport failure is not evidence about the binary; got {v:?}"
    );
}
