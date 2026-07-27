//! Render the systemd `--user` unit that supervises `demeteo-runner` on
//! a remote machine. The unit template lives next to the runner source
//! (single source of truth across both the in-app install path and the
//! manual `crates/demeteo-runner/packaging/install.sh`), so this module
//! is essentially a template loader + per-machine webhook injector.

use crate::domain::ids::MachineId;
use crate::error::AppError;
use crate::ports::db::MachineRepository;

/// The systemd `--user` unit shipped in the same source tree the app
/// itself is built from (M2.1) — single source of truth so this install
/// path never drifts from `crates/demeteo-runner/packaging/install.sh`'s
/// manual equivalent.
pub const RUNNER_SERVICE_UNIT: &str =
    include_str!("../../../../demeteo-runner/packaging/systemd/demeteo-runner.service");

/// Build the unit file text for `machine_id`, optionally injecting its
/// "away" notification webhook (docs/REMOTE_EXECUTION.md M6.3
/// follow-up) as an `Environment=` line. Without a webhook, returns the
/// template verbatim.
pub fn unit_for(
    machines: &dyn MachineRepository,
    machine_id: &MachineId,
) -> Result<String, AppError> {
    let url = machines
        .get_machine(machine_id)
        .map_err(AppError::from)?
        .and_then(|m| m.notify_webhook_url)
        .filter(|s| !s.trim().is_empty());
    let Some(url) = url else {
        return Ok(RUNNER_SERVICE_UNIT.to_string());
    };
    let escaped = url.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(RUNNER_SERVICE_UNIT.replacen(
        "StandardError=journal\n",
        &format!("StandardError=journal\nEnvironment=\"DEMETEO_NOTIFY_WEBHOOK_URL={escaped}\"\n"),
        1,
    ))
}
