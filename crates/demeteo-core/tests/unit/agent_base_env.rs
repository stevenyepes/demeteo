//! Pure-logic unit tests for `agent_base_env` / `resolve_agent_home`.
//!
//! Covers the regression that bit a remote-machine run: a parent
//! process's `$HOME` was forwarded as `HOME` into the SSH channel
//! when the agent was spawned against a remote machine, so opencode
//! and claude-code (which read their config out of `$HOME`) tried
//! to read `/home/<gui-user>` from the remote box and exited with
//! code 1. See the doc-comment on `agent_base_env` for the full
//! rationale.
//!
//! These tests are integration-level (lives in `tests/unit/`) so
//! they exercise the public `pub` signature without reaching into
//! the crate's private modules.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::action::AgentAction;
use crate::domain::intercept::ExecutionResult;
use crate::domain::models::Platform;
use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
use crate::ports::agent_runtime::{agent_base_env, resolve_agent_home, resolve_agent_platform};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};

/// `ExecutionPort` stub whose `resolve_home`, `resolve_user` and
/// `resolve_platform` return configurable per-machine values. Those
/// three are the only methods `agent_base_env` consults on the exec
/// port, so a single fake is enough to exercise every behaviour the
/// function cares about — every other `ExecutionPort` method returns
/// a benign no-op so the trait stays satisfied.
struct FakeExec {
    homes: HashMap<String, String>,
    users: HashMap<String, String>,
    platforms: HashMap<String, Platform>,
    fail_for: Vec<String>,
}

impl FakeExec {
    fn new() -> Self {
        Self {
            homes: HashMap::new(),
            users: HashMap::new(),
            platforms: HashMap::new(),
            fail_for: Vec::new(),
        }
    }

    fn with_home(mut self, machine_id: &str, home: &str) -> Self {
        self.homes.insert(machine_id.to_string(), home.to_string());
        self
    }

    fn with_user(mut self, machine_id: &str, user: &str) -> Self {
        self.users.insert(machine_id.to_string(), user.to_string());
        self
    }

    fn with_platform(mut self, machine_id: &str, platform: Platform) -> Self {
        self.platforms.insert(machine_id.to_string(), platform);
        self
    }

    fn failing_for(mut self, machine_id: &str) -> Self {
        self.fail_for.push(machine_id.to_string());
        self
    }
}

#[async_trait]
impl AgentExecutionPort for FakeExec {
    async fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> {
        Ok(CommandOutcome::Executed {
            output: ExecutionResult::Bash {
                output: String::new(),
            },
        })
    }
    async fn submit_agent(
        &self,
        _: &str,
        _: &str,
        _: AgentAction,
        _: Option<String>,
    ) -> Result<CommandOutcome, ActionError> {
        Err(ActionError::internal("fake exec: no agent submission"))
    }
    async fn approve(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn reject(&self, _: &str, _: String) -> Result<(), String> {
        Ok(())
    }
    async fn register_result_responder(
        &self,
        _: &str,
        _: tokio::sync::oneshot::Sender<Result<ExecutionResult, String>>,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
impl ExecutionPort for FakeExec {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
    async fn get_metadata(&self, _: &str, path: &str) -> Result<SftpEntry, String> {
        Ok(SftpEntry {
            name: path.into(),
            path: path.into(),
            is_dir: false,
            size: 0,
            modified: 0,
        })
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        Ok(vec![])
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
        if self.fail_for.iter().any(|m| m == machine_id) {
            return Err(format!("simulated resolve_home failure for {}", machine_id));
        }
        self.homes
            .get(machine_id)
            .cloned()
            .ok_or_else(|| format!("no fake home configured for {}", machine_id))
    }
    async fn resolve_user(&self, machine_id: &str) -> Result<String, String> {
        if self.fail_for.iter().any(|m| m == machine_id) {
            return Err(format!("simulated resolve_user failure for {}", machine_id));
        }
        self.users
            .get(machine_id)
            .cloned()
            .ok_or_else(|| format!("no fake user configured for {}", machine_id))
    }
    async fn resolve_platform(&self, machine_id: &str) -> Result<Platform, String> {
        if self.fail_for.iter().any(|m| m == machine_id) {
            return Err(format!(
                "simulated resolve_platform failure for {}",
                machine_id
            ));
        }
        self.platforms
            .get(machine_id)
            .copied()
            .ok_or_else(|| format!("no fake platform configured for {}", machine_id))
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("control_rpc not supported by FakeExec".to_string())
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        Err("spawn_interactive not supported by FakeExec".to_string())
    }
}

/// The identity of the local machine, spelled as a value the port has to be
/// asked for. A test that compared against `std::env::var("HOME")` would pass
/// whether the function consulted the port or read the environment behind its
/// back, which is the distinction these tests exist to hold.
const GUI_HOME: &str = "/home/gui-user";

#[tokio::test]
async fn agent_base_env_uses_remote_home_for_remote_machine() {
    let exec = Arc::new(FakeExec::new().with_home("m-dev", "/home/developer"));
    let env = agent_base_env(exec.as_ref(), "m-dev").await;
    assert_eq!(
        env.get("HOME").map(String::as_str),
        Some("/home/developer"),
        "remote machine must NOT receive the parent process's HOME"
    );
}

#[tokio::test]
async fn agent_base_env_asks_the_port_for_an_empty_machine_id() {
    let exec = Arc::new(FakeExec::new().with_home("", GUI_HOME));
    let env = agent_base_env(exec.as_ref(), "").await;
    assert_eq!(env.get("HOME").map(String::as_str), Some(GUI_HOME));
}

#[tokio::test]
async fn agent_base_env_asks_the_port_for_the_literal_local_machine() {
    let exec = Arc::new(FakeExec::new().with_home("local", GUI_HOME));
    let env = agent_base_env(exec.as_ref(), "local").await;
    assert_eq!(env.get("HOME").map(String::as_str), Some(GUI_HOME));
}

#[tokio::test]
async fn agent_base_env_invents_no_home_when_the_local_machine_cannot_name_one() {
    // Windows reaches its home through `USERPROFILE`, so an unset `HOME` is
    // not proof of a broken machine and must not be forged into one.
    let exec = Arc::new(FakeExec::new());
    let env = agent_base_env(exec.as_ref(), "local").await;
    assert!(!env.contains_key("HOME"));
}

#[tokio::test]
async fn agent_base_env_falls_back_to_local_home_on_remote_resolution_failure() {
    let exec = Arc::new(
        FakeExec::new()
            .with_home("local", GUI_HOME)
            .failing_for("m-flaky"),
    );
    let env = agent_base_env(exec.as_ref(), "m-flaky").await;
    // Graceful degradation: the agent at least sees *some* HOME
    // rather than crashing on a missing `~`. The real fix is the
    // SSH adapter's `home_cache`, but the port may legitimately be
    // down at agent-spawn time and the agent shouldn't fail the
    // whole run for it.
    assert_eq!(env.get("HOME").map(String::as_str), Some(GUI_HOME));
}

#[tokio::test]
async fn agent_base_env_uses_remote_user_for_remote_machine() {
    // Regression: the GUI's local `$USER` (e.g. `jsteven`) used to
    // leak into every SSH spawn, so the agent ran with a split
    // identity (HOME=/home/developer, USER=jsteven) that confused
    // some provider auth flows. The port-side fix routes USER
    // through `ExecutionPort::resolve_user`, so a remote machine
    // now gets the SSH-authenticated user.
    let exec = Arc::new(
        FakeExec::new()
            .with_home("m-dev", "/home/developer")
            .with_user("m-dev", "developer"),
    );
    let env = agent_base_env(exec.as_ref(), "m-dev").await;
    assert_eq!(
        env.get("USER").map(String::as_str),
        Some("developer"),
        "remote USER must come from the execution port, not the GUI"
    );
    assert_eq!(
        env.get("LOGNAME").map(String::as_str),
        Some("developer"),
        "LOGNAME mirrors USER so the agent's $USER and $LOGNAME agree"
    );
}

#[tokio::test]
async fn agent_base_env_falls_back_to_parent_user_when_remote_resolution_fails() {
    // Same graceful-degradation contract as HOME: if the port can't
    // resolve the remote user, the agent at least sees the parent
    // process's USER (or nothing if the parent has no USER set)
    // rather than the wrong one.
    let exec = Arc::new(
        FakeExec::new()
            .with_home("m-flaky", "/home/developer")
            .failing_for("m-flaky"),
    );
    let env = agent_base_env(exec.as_ref(), "m-flaky").await;
    let expected = std::env::var("USER").ok();
    assert_eq!(
        env.get("USER").map(String::as_str),
        expected.as_deref(),
        "USER must fall back to the parent process value when resolve_user fails"
    );
}

#[tokio::test]
async fn agent_base_env_forwards_the_desktop_shell_to_a_posix_agent() {
    // The pre-existing behaviour, now conditional: a POSIX target still sees
    // the desktop's own SHELL/TMPDIR verbatim. USER/LOGNAME are covered by
    // their own tests above (remote uses the port, local inherits the parent).
    let exec = Arc::new(
        FakeExec::new()
            .with_home("m-dev", "/home/developer")
            .with_platform("m-dev", Platform::Linux),
    );
    let env = agent_base_env(exec.as_ref(), "m-dev").await;
    for k in ["SHELL", "TMPDIR"] {
        assert_eq!(
            env.get(k).cloned(),
            std::env::var(k).ok(),
            "{k} must reach a POSIX agent exactly as the desktop has it"
        );
    }
}

#[tokio::test]
async fn agent_base_env_hands_a_windows_agent_no_posix_identity() {
    // A desktop started from Git Bash carries SHELL=/usr/bin/bash, and
    // forwarding it told the agent a POSIX shell was waiting for it. The
    // resolved HOME is unaffected — it was asked of the machine, so it is
    // already whatever that machine calls a home.
    let exec = Arc::new(
        FakeExec::new()
            .with_home("m-win", "C:/Users/dev")
            .with_platform("m-win", Platform::Windows),
    );
    let env = agent_base_env(exec.as_ref(), "m-win").await;
    for k in ["SHELL", "TMPDIR"] {
        assert!(!env.contains_key(k), "{k} must not reach a Windows agent");
    }
    assert_eq!(env.get("HOME").map(String::as_str), Some("C:/Users/dev"));
}

#[tokio::test]
async fn resolve_agent_home_remote_uses_exec() {
    let exec = Arc::new(FakeExec::new().with_home("m-dev", "/home/developer"));
    let home = resolve_agent_home(exec.as_ref(), "m-dev").await;
    assert_eq!(home, "/home/developer");
}

#[tokio::test]
async fn resolve_agent_home_local_uses_exec() {
    let exec = Arc::new(FakeExec::new().with_home("local", GUI_HOME));
    let home = resolve_agent_home(exec.as_ref(), "local").await;
    assert_eq!(home, GUI_HOME);
}

#[tokio::test]
async fn resolve_agent_home_empty_uses_exec() {
    let exec = Arc::new(FakeExec::new().with_home("", GUI_HOME));
    let home = resolve_agent_home(exec.as_ref(), "").await;
    assert_eq!(home, GUI_HOME);
}

/// The whole point of routing this through the port: the machine the agent
/// lands on decides, so a remote answer must survive to the context even when
/// it disagrees with everything about the desktop that asked.
#[tokio::test]
async fn resolve_agent_platform_reports_what_the_machine_said() {
    let exec = Arc::new(FakeExec::new().with_platform("m-dev", Platform::Linux));
    assert_eq!(
        resolve_agent_platform(exec.as_ref(), "m-dev").await,
        Some(Platform::Linux),
    );
}

/// Unlike HOME there is no local fallback to reach for — the desktop's OS is
/// the answer to a different question — so an unreachable machine leaves the
/// platform unknown rather than POSIX.
#[tokio::test]
async fn resolve_agent_platform_degrades_to_unknown_rather_than_to_the_desktop() {
    let exec = Arc::new(
        FakeExec::new()
            .with_platform("local", Platform::MacOS)
            .failing_for("m-flaky"),
    );
    assert_eq!(resolve_agent_platform(exec.as_ref(), "m-flaky").await, None);
}
