//! Asks a runner how it is paired with the Hub, and keeps the answer in
//! `machine_hub_state`, from which the desktop decides whether it still
//! reports that runner's runs ([`crate::domain::hub_socket::reported_feature_id`]).

use crate::domain::hub_socket::{observed_runner_hub_state, RunnerHubState};
use crate::ports::db::{AppSettingsRepository, MachineHubStateRepository};
use crate::ports::execution::ExecutionPort;
use crate::shared::time::now_secs;
use crate::state::AppContext;

use super::rpc::remote_rpc_via;

/// The state stored for `machine_id`, or `None` when the runner could not
/// be asked and its last known state stands.
pub async fn refresh_runner_hub_state(
    ctx: &AppContext,
    machine_id: &str,
) -> Result<Option<RunnerHubState>, String> {
    refresh_runner_hub_state_via(
        &*ctx.exec,
        &*ctx.app_settings,
        &*ctx.machine_hub_state,
        machine_id,
    )
    .await
}

/// [`refresh_runner_hub_state`] over the three ports it reads.
pub(super) async fn refresh_runner_hub_state_via(
    exec: &dyn ExecutionPort,
    settings: &dyn AppSettingsRepository,
    states: &dyn MachineHubStateRepository,
    machine_id: &str,
) -> Result<Option<RunnerHubState>, String> {
    let answer = remote_rpc_via(
        exec,
        settings,
        machine_id,
        "hub_status",
        serde_json::json!({}),
    )
    .await;
    let checked_at = i64::try_from(now_secs()).unwrap_or(i64::MAX);
    let Some(state) =
        observed_runner_hub_state(answer.as_ref().map_err(String::as_str), checked_at)
    else {
        return Ok(None);
    };
    states.put(machine_id, &state)?;
    Ok(Some(state))
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/runner_hub_state.rs"]
mod tests;
