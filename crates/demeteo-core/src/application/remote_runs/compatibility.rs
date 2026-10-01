//! Reads a detached machine's runner version and holds it against the app's.
//! What counts as compatible is [`crate::domain::runner_version`]; these
//! functions only take the reading.
//!
//! The live process is asked first: `health.build_version` is the version of
//! the binary actually serving the socket. A runner from before that field
//! existed, or one that is not running, is read from `--version` on the
//! installed binary instead. `health.version` is never read — it is the crate
//! version, which on a nightly build is the base release without its `-N`, so
//! a nightly runner would read as the stable build it came after.

use serde_json::Value;

use crate::domain::runner_version::{assess, RunnerCompatibility, RunnerVersionReading};
use crate::error::AppError;
use crate::infrastructure::runner::status::{installed_bin_path, probe_version};
use crate::ports::execution::ExecutionPort;

/// Where the `--version` reading comes from when health gives no
/// `build_version`.
pub enum BinaryFallback {
    /// Resolve home and run `--version` here.
    Probe,
    /// A `--version` the caller already ran on this machine: `Err` when the
    /// command did not run, `Ok(None)` when it printed nothing.
    Taken(Result<Option<String>, String>),
}

pub async fn read_runner_version(
    exec: &dyn ExecutionPort,
    machine_id: &str,
) -> RunnerVersionReading {
    read_runner_version_with(exec, machine_id, BinaryFallback::Probe).await
}

pub async fn read_runner_version_with(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    fallback: BinaryFallback,
) -> RunnerVersionReading {
    let health = match exec
        .control_rpc(machine_id, "health", serde_json::json!({}))
        .await
    {
        Ok(health) => match build_version(&health) {
            Some(raw) => return RunnerVersionReading::Reported(raw),
            None => "health reported no build_version".to_string(),
        },
        Err(e) => format!("health: {e}"),
    };
    let binary = match fallback {
        BinaryFallback::Taken(binary) => binary,
        BinaryFallback::Probe => match exec.resolve_home(machine_id).await {
            Ok(home) => probe_version(exec, machine_id, &installed_bin_path(&home)).await,
            Err(e) => return RunnerVersionReading::Unreachable(format!("{health}; home: {e}")),
        },
    };
    match binary {
        Ok(Some(raw)) => RunnerVersionReading::Reported(raw),
        Ok(None) => RunnerVersionReading::NotInstalled,
        Err(e) => RunnerVersionReading::Unreachable(format!("{health}; --version: {e}")),
    }
}

/// A present-but-malformed `build_version` is still the live runner's answer,
/// so it is reported as-is for the verdict to reject; falling through to
/// `--version` would let the installed binary vouch for a process it is not.
/// The residue: a live runner from before `build_version` existed answers
/// without it, so `--version` vouches for a process that may be an older,
/// not-yet-restarted build — until the next push restarts it.
fn build_version(health: &Value) -> Option<String> {
    let value = health.get("build_version").filter(|v| !v.is_null())?;
    Some(
        value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_string),
    )
}

pub async fn runner_compatibility(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    app_version: &str,
) -> RunnerCompatibility {
    runner_compatibility_with(exec, machine_id, app_version, BinaryFallback::Probe).await
}

pub async fn runner_compatibility_with(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    app_version: &str,
    fallback: BinaryFallback,
) -> RunnerCompatibility {
    assess(
        app_version,
        read_runner_version_with(exec, machine_id, fallback).await,
    )
}

/// `Err` for every verdict but compatible, including one that could not be
/// reached: an unverified runner is refused, not assumed.
pub async fn ensure_runner_compatible(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    app_version: &str,
) -> Result<(), AppError> {
    let verdict = runner_compatibility(exec, machine_id, app_version).await;
    if verdict.is_compatible() {
        Ok(())
    } else {
        Err(AppError::runner_incompatible(machine_id, verdict))
    }
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/compatibility.rs"]
mod tests;
