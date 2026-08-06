// Tests for `src/adapters/step_executor/harness_shell.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// Three claims that were unreachable while these were methods on
// `ExecutionDriver`: the shell is login-interactive unconditionally, Stop
// prevents the command from ever reaching the port, and a *dropped* cancel
// sender is not a cancellation.

use super::*;
use crate::domain::models::{AgentTimeouts, Platform, CONFIG_KEY};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// An `AppSettingsRepository` that answers the one key the timeout resolver
/// reads and **refuses everything else** — the shape AGENTS.md §7 asks for. A
/// double that answers every call is asserted against a default, not an answer.
struct SettingsDouble {
    timeouts: Option<String>,
}

impl SettingsDouble {
    fn wall_cap(seconds: u64) -> Self {
        let t = AgentTimeouts::validated(10, 30, seconds).expect("a valid ladder");
        Self {
            timeouts: Some(serde_json::to_string(&t).expect("serialisable")),
        }
    }

    fn unconfigured() -> Self {
        Self { timeouts: None }
    }
}

impl AppSettingsRepository for SettingsDouble {
    fn add_provider_instance(
        &self,
        _p: crate::domain::models::ProviderInstance,
    ) -> Result<(), String> {
        panic!("unscripted add_provider_instance")
    }
    fn get_provider_instances(
        &self,
    ) -> Result<Vec<crate::domain::models::ProviderInstance>, String> {
        panic!("unscripted get_provider_instances")
    }
    fn delete_provider_instance(&self, _id: &crate::domain::ids::ProviderId) -> Result<(), String> {
        panic!("unscripted delete_provider_instance")
    }
    fn get_app_session(&self, _key: &str) -> Result<Option<String>, String> {
        panic!("unscripted get_app_session")
    }
    fn set_app_session(&self, _key: &str, _value: &str) -> Result<(), String> {
        panic!("unscripted set_app_session")
    }
    fn delete_app_session(&self, _key: &str) -> Result<(), String> {
        panic!("unscripted delete_app_session")
    }
    fn app_setting_get(&self, key: &str) -> Result<Option<String>, String> {
        assert_eq!(key, CONFIG_KEY, "the only key this resolver may read");
        Ok(self.timeouts.clone())
    }
    fn app_setting_set(&self, _key: &str, _value: &str) -> Result<(), String> {
        panic!("resolving a ceiling must not write")
    }
}

/// An `ExecutionPort` double that **errors on anything it was not explicitly
/// told to answer**, and records every command it was asked to run. The call
/// log is half the point: "a pre-fired cancel never reaches the port" is a
/// claim about a call that must not happen, and a double that only returns
/// values cannot witness it.
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

    fn calls(&self) -> Vec<String> {
        self.seen.lock().expect("not poisoned").clone()
    }
}

#[async_trait::async_trait]
impl ExecutionPort for ScriptedExec {
    async fn test_connection(&self, _m: &str) -> Result<(), String> {
        Err("unscripted test_connection".into())
    }
    async fn run_command_with(
        &self,
        _m: &str,
        cmd: &str,
        _o: ShellOptions,
    ) -> Result<String, String> {
        self.seen
            .lock()
            .expect("not poisoned")
            .push(cmd.to_string());
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

// ── the shell is login-interactive, unconditionally ─────────────────────────

/// The detached-run incident in one assertion: a bare `sh -c` cannot see a
/// version manager's shims, so `cargo test` dies with "cargo: not found" while
/// the implement step — whose agent binary was resolved to an absolute path —
/// sails through. There is no machine parameter here on purpose; the flag that
/// used to gate this is hardcoded `None` for a detached run.
#[test]
fn the_harness_always_runs_under_a_login_interactive_shell_with_an_explicit_cwd() {
    let opts = harness_shell_options(&SettingsDouble::wall_cap(900), "/home/u/wt/feat");

    assert!(opts.login_shell, "a profile establishes the user's PATH");
    assert!(
        opts.interactive,
        "only an interactive shell activates shims"
    );
    assert_eq!(
        opts.cwd.as_deref(),
        Some("/home/u/wt/feat"),
        "D2: the worktree is explicit, never ambient"
    );
}

#[test]
fn the_options_carry_the_resolved_wall_cap_as_their_deadline() {
    let opts = harness_shell_options(&SettingsDouble::wall_cap(1234), "/wt");
    assert_eq!(opts.timeout, Some(Duration::from_secs(1234)));
    assert_eq!(harness_ceiling_s(&SettingsDouble::wall_cap(1234)), 1234);
}

/// An unconfigured install still gets a ceiling. Without one the harness was
/// the only unbounded wait in a step, and a watch-mode `npm test` hung it until
/// the app restarted.
#[test]
fn an_unconfigured_install_still_gets_a_ceiling() {
    let ceiling = harness_ceiling_s(&SettingsDouble::unconfigured());
    assert_eq!(ceiling, AgentTimeouts::default().wall_cap_s);
    assert!(ceiling > 0);
    assert_eq!(
        harness_shell_options(&SettingsDouble::unconfigured(), "/wt").timeout,
        Some(Duration::from_secs(ceiling))
    );
}

// ── Stop, and the thing that is not Stop ────────────────────────────────────

/// Dropping the run future is what stops the work, so a cancel that has already
/// fired must not let the command start at all — a `Some` here would mean the
/// user pressed Stop and then waited out a cold `npm install`.
#[tokio::test]
async fn a_pre_fired_cancel_never_reaches_the_port() {
    let (tx, rx) = tokio::sync::watch::channel(false);
    tx.send(true).expect("receiver alive");
    let exec = ScriptedExec::new(&[("npm test", Ok("42 passing"))]);

    let out = run_harness_command(&exec, rx, "local", "npm test", ShellOptions::default()).await;

    assert!(out.is_none(), "a cancelled command has no result");
    assert!(
        exec.calls().is_empty(),
        "the port must never have been asked; got {:?}",
        exec.calls()
    );
}

#[tokio::test]
async fn an_un_cancelled_command_returns_the_ports_own_answer() {
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let exec = ScriptedExec::new(&[
        ("npm test", Ok("42 passing")),
        ("npm run lint", Err("3 problems")),
    ]);

    assert_eq!(
        run_harness_command(
            &exec,
            rx.clone(),
            "local",
            "npm test",
            ShellOptions::default()
        )
        .await,
        Some(Ok("42 passing".to_string()))
    );
    assert_eq!(
        run_harness_command(&exec, rx, "local", "npm run lint", ShellOptions::default()).await,
        Some(Err("3 problems".to_string()))
    );
}

/// `wait_for` also resolves — as `Err` — when the *sender* is dropped. That is
/// "nobody can cancel this any more", not "this was cancelled": treating it as
/// a cancel would kill a healthy step during feature teardown, so the branch
/// parks and lets the command decide the outcome.
#[tokio::test]
async fn a_dropped_cancel_sender_is_not_a_cancellation() {
    let (tx, rx) = tokio::sync::watch::channel(false);
    drop(tx);
    let exec = ScriptedExec::new(&[("npm test", Ok("42 passing"))]);

    let out = run_harness_command(&exec, rx, "local", "npm test", ShellOptions::default()).await;

    assert_eq!(
        out,
        Some(Ok("42 passing".to_string())),
        "the command's own answer must survive the sender going away"
    );
}
