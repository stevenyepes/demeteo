//! Probe the state of a remote `demeteo-runner` installation over SSH:
//! does the binary exist + report a version, is the systemd `--user`
//! unit active, is systemd lingering enabled for the SSH user. All
//! three sub-probes are best-effort — a failure on one is surfaced as
//! `None` for that field rather than failing the whole call, so the
//! UI can still show what it did learn.

use crate::error::AppError;
use crate::ports::execution::ExecutionPort;

/// State of a remote runner. Each field is independently optional —
/// `None` means the corresponding sub-probe couldn't run, not that the
/// value is false.
#[derive(Debug, Clone, Default)]
pub struct RemoteRunnerProbe {
    /// Raw `demeteo-runner --version` output. `None` when the binary
    /// isn't on the box or the probe failed.
    pub version: Option<String>,
    /// `true` when `systemctl --user is-active demeteo-runner` reports
    /// `active`; `false` for any other state; `None` on probe failure.
    pub service_active: Option<bool>,
    /// `true` when `loginctl show-user "$USER" -p Linger` reports
    /// `Linger=yes`; `false` for `Linger=no`; `None` on probe failure.
    pub lingering: Option<bool>,
}

impl RemoteRunnerProbe {
    pub fn is_installed(&self) -> bool {
        self.version.is_some()
    }
}

/// Run the three sub-probes against `machine_id`. None of them short-
/// circuits; partial results are returned.
pub async fn probe(
    exec: &dyn ExecutionPort,
    machine_id: &str,
) -> Result<RemoteRunnerProbe, AppError> {
    let home = exec
        .resolve_home(machine_id)
        .await
        .map_err(AppError::from)?;
    let bin_path = format!("{home}/.local/bin/demeteo-runner");

    let version = probe_version(exec, machine_id, &bin_path).await;

    if version.is_none() {
        return Ok(RemoteRunnerProbe {
            version: None,
            service_active: None,
            lingering: None,
        });
    }

    let service_active = exec
        .run_command(
            machine_id,
            "systemctl --user is-active demeteo-runner 2>/dev/null || true",
        )
        .await
        .ok()
        .map(|s| match s.trim() {
            "active" => true,
            "inactive" | "failed" | "activating" | "deactivating" | "unknown" => false,
            _ => false,
        });

    let lingering = exec
        .run_command(
            machine_id,
            "loginctl show-user \"$(whoami)\" -p Linger 2>/dev/null || true",
        )
        .await
        .ok()
        .and_then(|s| match s.trim() {
            "Linger=yes" => Some(true),
            "Linger=no" => Some(false),
            _ => None,
        });

    Ok(RemoteRunnerProbe {
        version,
        service_active,
        lingering,
    })
}

async fn probe_version(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    bin_path: &str,
) -> Option<String> {
    use crate::paths::shell_escape_posix;
    let cmd = format!(
        "{} --version 2>/dev/null || true",
        shell_escape_posix(bin_path)
    );
    let out = exec.run_command(machine_id, &cmd).await.ok()?;
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
