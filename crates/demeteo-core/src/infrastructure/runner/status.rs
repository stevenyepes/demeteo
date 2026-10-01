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
#[derive(Debug, Clone)]
pub struct RemoteRunnerProbe {
    /// The `--version` probe as `probe_version` returned it. Kept raw
    /// because the compatibility verdict reuses it, and there a command that
    /// did not run is `unknown` while one that printed nothing is
    /// `not_installed`.
    pub binary_version: Result<Option<String>, String>,
    /// `true` when `systemctl --user is-active demeteo-runner` reports
    /// `active`; `false` for any other state; `None` on probe failure.
    pub service_active: Option<bool>,
    /// `true` when `loginctl show-user "$USER" -p Linger` reports
    /// `Linger=yes`; `false` for `Linger=no`; `None` on probe failure.
    pub lingering: Option<bool>,
}

impl RemoteRunnerProbe {
    /// Raw `demeteo-runner --version` output. `None` when the binary
    /// isn't on the box or the probe failed.
    pub fn version(&self) -> Option<&str> {
        self.binary_version.as_ref().ok().and_then(Option::as_deref)
    }

    pub fn is_installed(&self) -> bool {
        self.version().is_some()
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
    let binary_version = probe_version(exec, machine_id, &installed_bin_path(&home)).await;

    if !matches!(binary_version, Ok(Some(_))) {
        return Ok(RemoteRunnerProbe {
            binary_version,
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
        binary_version,
        service_active,
        lingering,
    })
}

/// Where the runner is installed under the SSH user's `home`.
pub(crate) fn installed_bin_path(home: &str) -> String {
    format!("{home}/.local/bin/demeteo-runner")
}

/// The trimmed `--version` line of the binary at `bin_path`. `Ok(None)` when
/// it printed nothing — the `|| true` folds a missing binary into that — and
/// `Err` only when the machine did not run the command at all.
pub(crate) async fn probe_version(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    bin_path: &str,
) -> Result<Option<String>, String> {
    use crate::paths::shell_escape_posix;
    let cmd = format!(
        "{} --version 2>/dev/null || true",
        shell_escape_posix(bin_path)
    );
    let out = exec.run_command(machine_id, &cmd).await?;
    let trimmed = out.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
}
