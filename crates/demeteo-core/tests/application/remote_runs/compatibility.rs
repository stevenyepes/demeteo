use super::*;

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;

use crate::domain::models::Platform;
use crate::domain::runner_version::ReleaseChannel;
use crate::ports::execution::{InteractiveHandle, SftpEntry};

const MACHINE: &str = "m-1";
const HOME: &str = "/home/run ner";
/// The fallback probe, pinned literally: a home with a space proves the path
/// is quoted, and a double keyed by command answers nothing else.
const VERSION_CMD: &str = "'/home/run ner/.local/bin/demeteo-runner' --version 2>/dev/null || true";

/// Answers only what it was scripted with — every other call is `Err` — and
/// records every call it received.
#[derive(Default)]
struct StrictExec {
    rpc: HashMap<String, Result<Value, String>>,
    commands: HashMap<String, Result<String, String>>,
    home: Option<Result<String, String>>,
    calls: Mutex<Vec<String>>,
}

impl StrictExec {
    fn health(mut self, answer: Result<Value, &str>) -> Self {
        self.rpc
            .insert("health".to_string(), answer.map_err(str::to_string));
        self
    }

    fn home(mut self, answer: Result<&str, &str>) -> Self {
        self.home = Some(answer.map(str::to_string).map_err(str::to_string));
        self
    }

    fn version_cmd(mut self, answer: Result<&str, &str>) -> Self {
        self.commands.insert(
            VERSION_CMD.to_string(),
            answer.map(str::to_string).map_err(str::to_string),
        );
        self
    }

    fn record(&self, machine_id: &str, call: String) -> Result<(), String> {
        self.calls.lock().unwrap().push(call.clone());
        if machine_id == MACHINE {
            Ok(())
        } else {
            Err(format!("unscripted machine {machine_id} for {call}"))
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn unscripted<T>(&self, call: &str) -> Result<T, String> {
        self.calls.lock().unwrap().push(call.to_string());
        Err(format!("unscripted {call}"))
    }
}

#[async_trait]
impl ExecutionPort for StrictExec {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        self.unscripted("test_connection")
    }
    async fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String> {
        self.record(machine_id, format!("run_command {cmd}"))?;
        self.commands
            .get(cmd)
            .cloned()
            .unwrap_or_else(|| Err(format!("unscripted command {cmd}")))
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
    async fn get_metadata(&self, _machine_id: &str, _path: &str) -> Result<SftpEntry, String> {
        self.unscripted("get_metadata")
    }
    async fn list_dir(&self, _machine_id: &str, _path: &str) -> Result<Vec<SftpEntry>, String> {
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
        self.record(machine_id, "resolve_home".to_string())?;
        self.home
            .clone()
            .unwrap_or_else(|| Err("unscripted resolve_home".to_string()))
    }
    async fn resolve_platform(&self, _machine_id: &str) -> Result<Platform, String> {
        self.unscripted("resolve_platform")
    }
    async fn resolve_user(&self, _machine_id: &str) -> Result<String, String> {
        self.unscripted("resolve_user")
    }
    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        _params: Value,
    ) -> Result<Value, String> {
        self.record(machine_id, format!("rpc {method}"))?;
        self.rpc
            .get(method)
            .cloned()
            .unwrap_or_else(|| Err(format!("unscripted rpc {method}")))
    }
    fn spawn_interactive(
        &self,
        _machine_id: &str,
        _binary: &str,
        _args: &[String],
        _cwd: &str,
        _env: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        self.unscripted("spawn_interactive")
    }
}

fn refused(result: Result<(), AppError>) -> RunnerCompatibility {
    match result {
        Err(AppError::RunnerIncompatible { compatibility, .. }) => compatibility,
        other => panic!("expected RunnerIncompatible, got {other:?}"),
    }
}

fn fallback_calls() -> Vec<String> {
    vec![
        "rpc health".to_string(),
        "resolve_home".to_string(),
        format!("run_command {VERSION_CMD}"),
    ]
}

#[tokio::test]
async fn a_live_runner_on_the_app_build_passes_without_touching_the_shell() {
    let exec = StrictExec::default().health(Ok(json!({
        "version": "1.2.0",
        "build_version": "1.2.0-31",
        "pid": 7,
    })));

    ensure_runner_compatible(&exec, MACHINE, "1.2.0-31")
        .await
        .expect("the live runner is this build");

    assert_eq!(exec.calls(), ["rpc health"]);
}

#[tokio::test]
async fn an_older_live_runner_is_refused_as_behind() {
    let exec = StrictExec::default().health(Ok(json!({"build_version": "1.2.0-30"})));

    let verdict = refused(ensure_runner_compatible(&exec, MACHINE, "1.2.0-31").await);

    assert_eq!(
        verdict,
        RunnerCompatibility::RunnerBehind {
            runner: "1.2.0-30".to_string(),
            runner_channel: ReleaseChannel::Nightly,
            app: "1.2.0-31".to_string(),
            app_channel: ReleaseChannel::Nightly,
        }
    );
    assert_eq!(exec.calls(), ["rpc health"]);
}

#[tokio::test]
async fn a_runner_not_answering_health_is_read_from_its_binary() {
    let exec = StrictExec::default()
        .health(Err("connect: no such socket"))
        .home(Ok(HOME))
        .version_cmd(Ok("demeteo-runner 1.2.0-32\n"));

    let verdict = refused(ensure_runner_compatible(&exec, MACHINE, "1.2.0-31").await);

    assert_eq!(
        verdict,
        RunnerCompatibility::RunnerAhead {
            runner: "1.2.0-32".to_string(),
            runner_channel: ReleaseChannel::Nightly,
            app: "1.2.0-31".to_string(),
            app_channel: ReleaseChannel::Nightly,
        }
    );
    assert_eq!(exec.calls(), fallback_calls());
}

#[tokio::test]
async fn the_legacy_health_version_is_ignored_for_the_binary_reading() {
    let exec = StrictExec::default()
        .health(Ok(json!({"version": "1.2.0", "pid": 7})))
        .home(Ok(HOME))
        .version_cmd(Ok("demeteo-runner 1.2.0-31\n"));

    ensure_runner_compatible(&exec, MACHINE, "1.2.0-31")
        .await
        .expect("the binary is this build; the crate version is not the build");

    assert_eq!(exec.calls(), fallback_calls());
}

#[tokio::test]
async fn a_runner_nothing_can_reach_is_unknown() {
    let exec = StrictExec::default()
        .health(Err("connect: no such socket"))
        .home(Ok(HOME))
        .version_cmd(Err("transport: connection reset"));

    let verdict = refused(ensure_runner_compatible(&exec, MACHINE, "1.2.0-31").await);

    let RunnerCompatibility::Unknown { detail, .. } = verdict else {
        panic!("expected Unknown, got {verdict:?}");
    };
    assert!(detail.contains("no such socket"), "{detail}");
    assert!(detail.contains("connection reset"), "{detail}");
}

#[tokio::test]
async fn a_machine_whose_home_cannot_be_resolved_is_unknown() {
    let exec = StrictExec::default()
        .health(Err("connect: no such socket"))
        .home(Err("transport: auth failed"));

    let verdict = runner_compatibility(&exec, MACHINE, "1.2.0-31").await;

    assert!(
        matches!(verdict, RunnerCompatibility::Unknown { ref detail, .. } if detail.contains("auth failed")),
        "{verdict:?}"
    );
    assert_eq!(exec.calls(), ["rpc health", "resolve_home"]);
}

#[tokio::test]
async fn a_binary_that_prints_nothing_is_not_installed() {
    let exec = StrictExec::default()
        .health(Err("connect: no such socket"))
        .home(Ok(HOME))
        .version_cmd(Ok(""));

    let verdict = refused(ensure_runner_compatible(&exec, MACHINE, "1.2.0-31").await);

    assert_eq!(
        verdict,
        RunnerCompatibility::NotInstalled {
            app: "1.2.0-31".to_string(),
            app_channel: ReleaseChannel::Nightly,
        }
    );
}

#[tokio::test]
async fn a_malformed_live_build_version_is_unknown_not_a_fallback() {
    let exec = StrictExec::default()
        .health(Ok(json!({"build_version": "latest"})))
        .home(Ok(HOME))
        .version_cmd(Ok("demeteo-runner 1.2.0-31\n"));

    let verdict = runner_compatibility(&exec, MACHINE, "1.2.0-31").await;

    assert!(
        matches!(verdict, RunnerCompatibility::Unknown { .. }),
        "{verdict:?}"
    );
    assert_eq!(exec.calls(), ["rpc health"]);
}

#[tokio::test]
async fn a_non_string_live_build_version_is_reported_raw() {
    let exec = StrictExec::default().health(Ok(json!({"build_version": 31})));

    assert_eq!(
        read_runner_version(&exec, MACHINE).await,
        RunnerVersionReading::Reported("31".to_string())
    );
}

fn taken(answer: Result<&str, &str>) -> BinaryFallback {
    BinaryFallback::Taken(
        answer
            .map(|out| Some(out.to_string()).filter(|o| !o.is_empty()))
            .map_err(str::to_string),
    )
}

#[tokio::test]
async fn a_taken_reading_stands_in_for_a_runner_not_answering_health() {
    let exec = StrictExec::default().health(Err("connect: no such socket"));

    let verdict = runner_compatibility_with(
        &exec,
        MACHINE,
        "1.2.0-31",
        taken(Ok("demeteo-runner 1.2.0-32")),
    )
    .await;

    assert_eq!(
        verdict,
        RunnerCompatibility::RunnerAhead {
            runner: "1.2.0-32".to_string(),
            runner_channel: ReleaseChannel::Nightly,
            app: "1.2.0-31".to_string(),
            app_channel: ReleaseChannel::Nightly,
        }
    );
    assert_eq!(exec.calls(), ["rpc health"]);
}

#[tokio::test]
async fn a_taken_reading_stands_in_for_a_legacy_health_answer() {
    let exec = StrictExec::default().health(Ok(json!({"version": "1.2.0", "pid": 7})));

    let verdict = runner_compatibility_with(
        &exec,
        MACHINE,
        "1.2.0-31",
        taken(Ok("demeteo-runner 1.2.0-31")),
    )
    .await;

    assert!(verdict.is_compatible(), "{verdict:?}");
    assert_eq!(exec.calls(), ["rpc health"]);
}

#[tokio::test]
async fn a_taken_reading_that_did_not_run_is_unknown() {
    let exec = StrictExec::default().health(Err("connect: no such socket"));

    let verdict = runner_compatibility_with(
        &exec,
        MACHINE,
        "1.2.0-31",
        taken(Err("transport: connection reset")),
    )
    .await;

    let RunnerCompatibility::Unknown { detail, .. } = verdict else {
        panic!("expected Unknown, got {verdict:?}");
    };
    assert!(detail.contains("no such socket"), "{detail}");
    assert!(detail.contains("connection reset"), "{detail}");
    assert_eq!(exec.calls(), ["rpc health"]);
}

#[tokio::test]
async fn a_taken_reading_that_printed_nothing_is_not_installed() {
    let exec = StrictExec::default().health(Err("connect: no such socket"));

    let verdict = runner_compatibility_with(&exec, MACHINE, "1.2.0-31", taken(Ok(""))).await;

    assert_eq!(
        verdict,
        RunnerCompatibility::NotInstalled {
            app: "1.2.0-31".to_string(),
            app_channel: ReleaseChannel::Nightly,
        }
    );
    assert_eq!(exec.calls(), ["rpc health"]);
}

#[tokio::test]
async fn a_live_build_version_outranks_a_taken_reading() {
    let exec = StrictExec::default().health(Ok(json!({"build_version": "1.2.0-31"})));

    let verdict = runner_compatibility_with(&exec, MACHINE, "1.2.0-31", taken(Ok(""))).await;

    assert!(verdict.is_compatible(), "{verdict:?}");
    assert_eq!(exec.calls(), ["rpc health"]);
}
