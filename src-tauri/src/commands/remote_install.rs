//! One-click "Enable remote runs" (docs/REMOTE_EXECUTION_PLAN.md M7.1):
//! detect → provision → supervise `demeteo-runner` on a machine,
//! entirely as the SSH user, no `sudo`.
//!
//! **Provisioning stays laptop-driven, never `curl`-on-remote:** CI
//! publishes a `demeteo-runner` release asset under the exact same
//! version tag as the desktop app, but remote boxes are not assumed
//! to have internet access, so the *laptop* is the one that fetches
//! it (`remote_runner_download`) — never the remote machine. The
//! binary still reaches the machine exactly as before: SFTP push +
//! user-space systemd install (`remote_enable_runs`).
//!
//! This file is the thin Tauri command layer; the heavy lifting lives
//! in focused deep modules:
//! - `crates/demeteo-core/src/infrastructure/runner/binary.rs` — locate
//!   + magic-byte arch detection + version probe + cache paths
//! - `crates/demeteo-core/src/infrastructure/runner/install.rs` —
//!   systemd `--user` unit rendering
//! - `crates/demeteo-core/src/infrastructure/runner/status.rs` —
//!   remote state probe (binary + service + linger)
//! - `src-tauri/src/adapters/tauri_ui/runner_download.rs` — release-
//!   asset fetch with Tauri event emission + cancellation

use crate::adapters::tauri_ui::runner_download as download_adapter;
use crate::domain::ids::MachineId;
use crate::error::AppError;
use crate::infrastructure::runner::binary::{
    self as binary, locate_local, release_cache_path, RunnerArch, RunnerBinary,
};
use crate::infrastructure::runner::{install as install_module, status as status_module};
use crate::paths::shell_escape_posix;
use crate::state::AppContext;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

pub use download_adapter::DOWNLOAD_PROGRESS_EVENT;

#[derive(Debug, Serialize)]
pub struct RunnerInstallStatus {
    pub installed: bool,
    /// Raw `demeteo-runner --version` output. `None` when not installed
    /// or the probe failed.
    pub version: Option<String>,
    /// `Some(true)` when `systemctl --user is-active demeteo-runner`
    /// reports `active`; `Some(false)` for any other state; `None` on
    /// probe failure.
    pub service_active: Option<bool>,
    /// `Some(true)` when `loginctl show-user "$USER" -p Linger`
    /// reports `Linger=yes`; `Some(false)` for `Linger=no`; `None` on
    /// probe failure.
    pub lingering: Option<bool>,
}

#[tauri::command]
pub async fn remote_runner_status(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<RunnerInstallStatus, AppError> {
    let probe = status_module::probe(&*ctx.exec, &machine_id).await?;
    Ok(RunnerInstallStatus {
        installed: probe.is_installed(),
        version: probe.version,
        service_active: probe.service_active,
        lingering: probe.lingering,
    })
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LocalRunnerCheck {
    Ready {
        path: String,
        version: Option<String>,
        expected: String,
        stale_warning: Option<String>,
    },
    Missing {
        expected: String,
    },
}

/// Step 1 of provisioning: does this laptop already have a
/// `demeteo-runner` it can push, with no network call? Checked in
/// order: a dev build cached by `npm run build:runner`, the explicit
/// `$DEMETEO_RUNNER_BIN` override, a sibling of the running app, then
/// a previously downloaded release.
#[tauri::command]
pub async fn remote_runner_local_check(app: tauri::AppHandle) -> LocalRunnerCheck {
    let expected = app.package_info().version.to_string();
    if let Some(binary) = locate_local().await {
        return readiness_for(binary, &expected);
    }
    let cached = release_cache_path(&expected);
    if cached.is_file() {
        return LocalRunnerCheck::Ready {
            path: cached.display().to_string(),
            version: Some(expected.clone()),
            expected,
            stale_warning: None,
        };
    }
    LocalRunnerCheck::Missing { expected }
}

fn readiness_for(binary: RunnerBinary, expected: &str) -> LocalRunnerCheck {
    let stale = binary::stale_version_warning(&binary, expected);
    LocalRunnerCheck::Ready {
        path: binary.path.display().to_string(),
        version: binary.version,
        expected: expected.to_string(),
        stale_warning: stale,
    }
}

// `remote_runner_download` and `remote_runner_download_cancel` are
// registered from `adapters::tauri_ui::runner_download` (the Tauri
// events they emit live there). They're invoked through the same
// `invoke('remote_runner_download', ...)` names as before.

#[derive(Debug, Serialize)]
pub struct EnableRemoteRunsOutcome {
    /// `demeteo-runner --version` output read back after install, so
    /// the UI shows the version that's actually running.
    pub version: Option<String>,
    pub linger_enabled: bool,
    /// Set when lingering couldn't be enabled (needs admin/polkit on
    /// some distros, §10.8).
    pub warning: Option<String>,
}

/// Install (or upgrade — same idempotent sequence either way) as a
/// systemd `--user` service: mkdir the three user-space directories,
/// SFTP the binary + `chmod +x`, write the unit file, `daemon-reload` +
/// `enable --now`, then try `loginctl enable-linger`. `local_bin_path`
/// is whatever `remote_runner_local_check`/`remote_runner_download`
/// resolved on this laptop — this command only ever pushes bytes.
#[tauri::command]
pub async fn remote_enable_runs(
    ctx: State<'_, AppContext>,
    machine_id: String,
    local_bin_path: String,
) -> Result<EnableRemoteRunsOutcome, AppError> {
    let bin_path = PathBuf::from(&local_bin_path);
    reject_non_linux_x86_64(&bin_path)?;

    let bytes = tokio::fs::read(&bin_path).await.map_err(|e| {
        AppError::from(format!(
            "failed to read local demeteo-runner binary at {}: {}",
            bin_path.display(),
            e
        ))
    })?;

    let home = ctx
        .exec
        .resolve_home(&machine_id)
        .await
        .map_err(AppError::from)?;
    let bin_dst = format!("{home}/.local/bin/demeteo-runner");
    let unit_dst = format!("{home}/.config/systemd/user/demeteo-runner.service");
    let data_dir = format!("{home}/.local/share/demeteo-runner");

    ctx.exec
        .run_command(
            &machine_id,
            &format!(
                "mkdir -p {} {} {}",
                shell_escape_posix(&format!("{home}/.local/bin")),
                shell_escape_posix(&format!("{home}/.config/systemd/user")),
                shell_escape_posix(&data_dir),
            ),
        )
        .await
        .map_err(AppError::from)?;

    ctx.exec
        .write_file_bytes(&machine_id, &bin_dst, &bytes)
        .await
        .map_err(AppError::from)?;
    ctx.exec
        .run_command(
            &machine_id,
            &format!("chmod +x {}", shell_escape_posix(&bin_dst)),
        )
        .await
        .map_err(AppError::from)?;
    let unit = install_module::unit_for(&*ctx.machines, &MachineId::from(machine_id.clone()))?;
    ctx.exec
        .write_file(&machine_id, &unit_dst, &unit)
        .await
        .map_err(AppError::from)?;

    ctx.exec
        .run_command(
            &machine_id,
            "systemctl --user daemon-reload && systemctl --user enable --now demeteo-runner \
             && systemctl --user restart demeteo-runner",
        )
        .await
        .map_err(AppError::from)?;

    // Best-effort linger enable (R2). Some distros gate this behind
    // polkit/sudo, so failure here doesn't fail the install.
    let (linger_enabled, warning) = match ctx
        .exec
        .run_command(&machine_id, "loginctl enable-linger \"$(whoami)\"")
        .await
    {
        Ok(_) => (true, None),
        Err(_) => (
            false,
            Some(
                "Runner won't survive SSH logout or a reboot — ask an administrator \
                 to run `loginctl enable-linger <user>` on this machine (some distros \
                 gate this behind polkit/sudo)."
                    .to_string(),
            ),
        ),
    };

    let version_probe = format!(
        "{} --version 2>/dev/null || true",
        shell_escape_posix(&bin_dst)
    );
    let version = ctx
        .exec
        .run_command(&machine_id, &version_probe)
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok(EnableRemoteRunsOutcome {
        version,
        linger_enabled,
        warning,
    })
}

/// Defense-in-depth arch check: refuse to push any non-ELF binary to a
/// remote Linux host, with a message that points the dev at
/// `npm run build:runner`. Reuses `RunnerBinary::is_linux_x86_64` so
/// the magic-byte logic lives in exactly one place.
fn reject_non_linux_x86_64(path: &Path) -> Result<(), AppError> {
    let probe = RunnerBinary {
        path: path.to_path_buf(),
        version: None,
    };
    let arch = probe.arch()?;
    if arch == RunnerArch::LinuxX86_64 {
        return Ok(());
    }
    let hint = match arch {
        RunnerArch::LinuxOther => "the binary is a Linux ELF for a non-x86_64 architecture",
        RunnerArch::MacOs => "the binary is a macOS Mach-O (arm64 or x86_64) — Demeteo can't run that on a Linux remote",
        RunnerArch::Windows => "the binary is a Windows PE — Demeteo can't run that on a Linux remote",
        RunnerArch::Unknown => "the binary's format isn't recognised as Linux x86_64 ELF",
        RunnerArch::LinuxX86_64 => unreachable!(),
    };
    Err(AppError::from(format!(
        "refusing to push {} — {hint}. Run `npm run build:runner` to produce a Linux x86_64 build, \
         or set DEMETEO_RUNNER_BIN to one.",
        path.display()
    )))
}
