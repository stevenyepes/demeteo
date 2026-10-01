// `super` = `commands::remote_install`. The commands take `State<'_, AppContext>`,
// which a test cannot construct, so this covers the wire shape they return and
// the free functions they delegate to.

use super::*;
use crate::domain::runner_version::{ReleaseChannel, RunnerCompatibility};

#[test]
fn compatibility_report_flattens_verdict_beside_message() {
    let verdict = RunnerCompatibility::RunnerAhead {
        runner: "1.3.0".into(),
        runner_channel: ReleaseChannel::Stable,
        app: "1.2.0-31".into(),
        app_channel: ReleaseChannel::Nightly,
    };
    let report = RunnerCompatibilityReport::new("devbox", verdict.clone());

    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["verdict"], "runner_ahead");
    assert_eq!(json["runner"], "1.3.0");
    assert_eq!(json["runner_channel"], "stable");
    assert_eq!(json["app"], "1.2.0-31");
    assert_eq!(json["app_channel"], "nightly");
    assert_eq!(json["message"], verdict.message("devbox"));
}

#[test]
fn install_status_serializes_absent_compatibility_as_null() {
    let status = RunnerInstallStatus {
        installed: false,
        version: None,
        service_active: None,
        lingering: None,
        compatibility: None,
    };

    let json = serde_json::to_value(&status).unwrap();
    assert!(json.as_object().unwrap().contains_key("compatibility"));
    assert!(json["compatibility"].is_null());
}

const MACHINE: &str = "devbox";
const VERSION_CMD: &str = "/home/dev/.local/bin/demeteo-runner --version 2>/dev/null || true";
const SERVICE_CMD: &str = "systemctl --user is-active demeteo-runner 2>/dev/null || true";
const LINGER_CMD: &str = "loginctl show-user \"$(whoami)\" -p Linger 2>/dev/null || true";

/// Answers `resolve_home`, the `health` RPC and the commands it was scripted
/// with, errors on everything else, and records every call it received — the
/// record is what the tests assert, so an accommodating default would hide a
/// second probe.
struct ScriptedExec {
    health: Result<serde_json::Value, String>,
    commands: std::collections::HashMap<&'static str, Result<String, String>>,
    calls: std::sync::Mutex<Vec<String>>,
}

impl ScriptedExec {
    fn new(health: Result<serde_json::Value, &str>, version: Result<&str, &str>) -> Self {
        let commands = [
            (
                VERSION_CMD,
                version.map(str::to_string).map_err(str::to_string),
            ),
            (SERVICE_CMD, Ok("active\n".to_string())),
            (LINGER_CMD, Ok("Linger=yes\n".to_string())),
        ]
        .into_iter()
        .collect();
        Self {
            health: health.map_err(str::to_string),
            commands,
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn count(&self, call: &str) -> usize {
        self.calls().iter().filter(|c| *c == call).count()
    }

    fn unscripted<T>(&self, call: &str) -> Result<T, String> {
        self.calls.lock().unwrap().push(call.to_string());
        Err(format!("unscripted {call}"))
    }
}

#[async_trait::async_trait]
impl ExecutionPort for ScriptedExec {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        self.unscripted("test_connection")
    }
    async fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String> {
        let call = format!("run_command {cmd}");
        match self.commands.get(cmd) {
            Some(answer) if machine_id == MACHINE => {
                self.calls.lock().unwrap().push(call);
                answer.clone()
            }
            _ => self.unscripted(&call),
        }
    }
    async fn read_file(&self, _machine_id: &str, _path: &str) -> Result<String, String> {
        self.unscripted("read_file")
    }
    async fn write_file(
        &self,
        _machine_id: &str,
        _path: &str,
        _content: &str,
    ) -> Result<(), String> {
        self.unscripted("write_file")
    }
    async fn write_file_bytes(
        &self,
        _machine_id: &str,
        _path: &str,
        _content: &[u8],
    ) -> Result<(), String> {
        self.unscripted("write_file_bytes")
    }
    async fn get_metadata(
        &self,
        _machine_id: &str,
        _path: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        self.unscripted("get_metadata")
    }
    async fn list_dir(
        &self,
        _machine_id: &str,
        _path: &str,
    ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
        self.unscripted("list_dir")
    }
    async fn setup_worktree(
        &self,
        _machine_id: &str,
        _repo_path: &str,
        _branch: &str,
        _sandbox_path: &str,
    ) -> Result<(), String> {
        self.unscripted("setup_worktree")
    }
    async fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
        if machine_id != MACHINE {
            return self.unscripted("resolve_home");
        }
        self.calls.lock().unwrap().push("resolve_home".to_string());
        Ok("/home/dev".to_string())
    }
    async fn resolve_platform(
        &self,
        _machine_id: &str,
    ) -> Result<crate::domain::models::Platform, String> {
        self.unscripted("resolve_platform")
    }
    async fn resolve_user(&self, _machine_id: &str) -> Result<String, String> {
        self.unscripted("resolve_user")
    }
    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if machine_id != MACHINE || method != "health" {
            return self.unscripted(&format!("rpc {method}"));
        }
        self.calls.lock().unwrap().push("rpc health".to_string());
        self.health.clone()
    }
    fn spawn_interactive(
        &self,
        _machine_id: &str,
        _binary: &str,
        _args: &[String],
        _cwd: &str,
        _env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        self.unscripted("spawn_interactive")
    }
}

#[tokio::test]
async fn status_of_a_legacy_runner_reads_the_binary_once() {
    let exec = ScriptedExec::new(
        Ok(serde_json::json!({"version": "1.2.0", "pid": 7})),
        Ok("demeteo-runner 1.2.0-31\n"),
    );

    let status = runner_status(&exec, MACHINE, "1.2.0-31").await.unwrap();

    assert!(status.installed);
    assert_eq!(status.version.as_deref(), Some("demeteo-runner 1.2.0-31"));
    let verdict = status.compatibility.expect("status carries a verdict");
    assert!(verdict.compatibility.is_compatible(), "{verdict:?}");
    assert_eq!(exec.count("resolve_home"), 1, "{:?}", exec.calls());
    assert_eq!(
        exec.count(&format!("run_command {VERSION_CMD}")),
        1,
        "{:?}",
        exec.calls()
    );
    assert_eq!(exec.count("rpc health"), 1, "{:?}", exec.calls());
}

#[tokio::test]
async fn status_whose_version_command_did_not_run_is_unknown_not_uninstalled() {
    let exec = ScriptedExec::new(
        Err("connect: no such socket"),
        Err("transport: connection reset"),
    );

    let status = runner_status(&exec, MACHINE, "1.2.0-31").await.unwrap();

    assert!(!status.installed);
    assert_eq!(status.version, None);
    let verdict = status.compatibility.expect("status carries a verdict");
    assert!(
        matches!(verdict.compatibility, RunnerCompatibility::Unknown { .. }),
        "{verdict:?}"
    );
    assert_eq!(exec.count("resolve_home"), 1, "{:?}", exec.calls());
}
