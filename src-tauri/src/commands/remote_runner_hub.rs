//! A remote runner's Hub pairing, managed from Settings › Machines. Each is
//! one control-socket call through
//! `demeteo_core::application::remote_runs::RunnerHubRpc`; the runner owns the
//! pairing and the policy, so nothing here is stored on this desktop.

use tauri::State;

use crate::domain::hub_pairing::device::DeviceTicket;
use crate::domain::hub_pairing::runner_view::{HubStatusView, PairStateView};
use crate::domain::hub_pairing::HubStatus;
use crate::domain::hub_request::{HubLocalPolicy, HubRequestLogRow};
use crate::state::AppContext;
use demeteo_core::application::remote_runs::RunnerHubRpc;

#[tauri::command]
pub async fn remote_hub_pair_begin(
    ctx: State<'_, AppContext>,
    machine_id: String,
    hub_url: String,
) -> Result<DeviceTicket, String> {
    RunnerHubRpc::of(&ctx)
        .pair_begin(&machine_id, &hub_url)
        .await
}

#[tauri::command]
pub async fn remote_hub_pair_state(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<PairStateView, String> {
    RunnerHubRpc::of(&ctx).pair_state(&machine_id).await
}

#[tauri::command]
pub async fn remote_hub_pair_cancel(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<PairStateView, String> {
    RunnerHubRpc::of(&ctx).pair_cancel(&machine_id).await
}

/// `null` for a runner from before Hub pairing.
#[tauri::command]
pub async fn remote_hub_status(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<Option<HubStatusView>, String> {
    RunnerHubRpc::of(&ctx).status(&machine_id).await
}

#[tauri::command]
pub async fn remote_hub_disconnect(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<HubStatus, String> {
    RunnerHubRpc::of(&ctx).disconnect(&machine_id).await
}

#[tauri::command]
pub async fn remote_hub_policy_get(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<HubLocalPolicy, String> {
    RunnerHubRpc::of(&ctx).policy_get(&machine_id).await
}

#[tauri::command]
pub async fn remote_hub_policy_set(
    ctx: State<'_, AppContext>,
    machine_id: String,
    policy: HubLocalPolicy,
) -> Result<HubLocalPolicy, String> {
    RunnerHubRpc::of(&ctx)
        .policy_set(&machine_id, &policy)
        .await
}

/// Newest first; `null` for a runner from before its activity log.
#[tauri::command]
pub async fn remote_hub_request_log(
    ctx: State<'_, AppContext>,
    machine_id: String,
    limit: usize,
) -> Result<Option<Vec<HubRequestLogRow>>, String> {
    RunnerHubRpc::of(&ctx).request_log(&machine_id, limit).await
}
