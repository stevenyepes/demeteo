// Tests extracted from `src/adapters/step_executor/preflight.rs`
// (mirrored-tests convention). `super` resolves to that module.

use super::*;
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};
use std::collections::HashMap;
use std::sync::Mutex;

// ── probeable_binaries ───────────────────────────────────────────────────────

#[test]
fn a_plain_command_yields_its_binary() {
    assert_eq!(probeable_binaries("cargo test"), vec!["cargo"]);
}

#[test]
fn every_stage_of_a_chain_is_probed() {
    // The real shape from the dev DB. Each `&&` stage runs a different tool,
    // and any one of them missing breaks the whole harness — so probing only
    // the first would miss exactly the polyglot case that motivated this.
    assert_eq!(
        probeable_binaries(
            "npx vitest run && npm run build && cargo build --manifest-path src-tauri/Cargo.toml"
        ),
        vec!["npx", "npm", "cargo"]
    );
}

#[test]
fn a_repeated_binary_is_probed_once() {
    assert_eq!(
        probeable_binaries("cargo fmt && cargo clippy && cargo test"),
        vec!["cargo"]
    );
}

#[test]
fn leading_env_assignments_are_stepped_over() {
    // `RUST_LOG=debug cargo test` runs `cargo`. Probing `RUST_LOG=debug` would
    // never resolve and would block a perfectly good launch.
    assert_eq!(
        probeable_binaries("RUST_LOG=debug CI=1 cargo test"),
        vec!["cargo"]
    );
}

#[test]
fn shell_builtins_are_not_probed() {
    // `cd src-tauri && cargo test` — the exact command this project's own
    // settings carried. `cd` is a builtin; whether `command -v cd` answers is
    // shell-dependent and irrelevant.
    assert_eq!(
        probeable_binaries("cd src-tauri && cargo test"),
        vec!["cargo"]
    );
}

#[test]
fn the_generated_polyglot_accumulator_probes_only_real_tools() {
    // What `detect_worktree_strategy`'s `run_all` emits for a multi-ecosystem
    // repo. Everything here except `npm` and `cargo` is a builtin, an
    // assignment, or arithmetic substitution.
    let cmd = "set +e; rc=0; npm test; rc=$((rc||$?)); cargo test; rc=$((rc||$?)); exit $rc";
    assert_eq!(probeable_binaries(cmd), vec!["npm", "cargo"]);
}

#[test]
fn unresolvable_words_are_skipped_rather_than_guessed_at() {
    // A false positive blocks a legitimate launch; a false negative just
    // lands the user in today's behaviour. Anything needing a shell to
    // resolve is therefore dropped.
    assert!(probeable_binaries("$(which pytest) -q").is_empty());
    assert!(probeable_binaries("`echo cargo` test").is_empty());
    assert!(probeable_binaries("./scripts/*.sh").is_empty());
}

#[test]
fn an_empty_or_whitespace_command_probes_nothing() {
    assert!(probeable_binaries("").is_empty());
    assert!(probeable_binaries("   \n  ").is_empty());
}

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

#[tokio::test]
async fn no_test_command_is_not_configured_and_never_touches_the_port() {
    let exec = ScriptedExec::new(&[]);
    let v = probe_configured_commands(&exec, "local", "/repo", None, T).await;

    assert_eq!(v, PreflightVerdict::NotConfigured);
    assert!(v.permits_launch(), "an unconfigured harness must not block");
    assert_eq!(v.phase_status(), "skipped");
    assert!(
        exec.seen.lock().unwrap().is_empty(),
        "nothing to probe means nothing should be run"
    );
}

#[tokio::test]
async fn a_blank_test_command_is_treated_as_unconfigured() {
    let exec = ScriptedExec::new(&[]);
    let v = probe_configured_commands(&exec, "local", "/repo", Some("   "), T).await;
    assert_eq!(v, PreflightVerdict::NotConfigured);
}

#[tokio::test]
async fn all_binaries_resolving_permits_the_launch() {
    let exec = ScriptedExec::new(&[
        ("command -v npm", Ok("/usr/bin/npm")),
        ("command -v cargo", Ok("/home/u/.cargo/bin/cargo")),
    ]);
    let v =
        probe_configured_commands(&exec, "local", "/repo", Some("npm test && cargo test"), T).await;

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
    let v =
        probe_configured_commands(&exec, "local", "/repo", Some("npm test && cargo test"), T).await;

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
    let v = probe_configured_commands(&exec, "local", "/repo", Some("cargo test"), T).await;
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
    let v = probe_configured_commands(&exec, "local", "/repo", Some("cargo test"), T).await;

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
    let v = probe_configured_commands(&exec, "local", "/repo", Some("cargo test"), T).await;
    assert!(
        v.permits_launch(),
        "a slow probe is not a missing binary; got {v:?}"
    );
}

#[tokio::test]
async fn a_command_of_pure_builtins_asserts_nothing_and_proceeds() {
    let exec = ScriptedExec::new(&[]);
    let v = probe_configured_commands(&exec, "local", "/repo", Some("true"), T).await;

    assert_eq!(v, PreflightVerdict::Resolved { probed: vec![] });
    assert!(v.permits_launch());
    assert!(
        v.detail().is_none(),
        "having verified nothing, it should claim nothing"
    );
    assert!(exec.seen.lock().unwrap().is_empty());
}
