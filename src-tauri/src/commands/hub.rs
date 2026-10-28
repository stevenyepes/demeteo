//! Desktop pairing with the Demeteo Hub, and the Settings side of the Hub
//! local policy and activity log. Every decision is
//! `adapters::hub::HubPairing`'s, `application::hub_settings`' or the
//! repositories'; these only unwrap state and map errors to the IPC string.

use std::sync::Arc;

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::adapters::hub::HubPairing;
use crate::application::hub_settings::save_hub_policy;
use crate::domain::hub_pairing::{HubMachine, HubStatus, PAIR_TIMEOUT};
use crate::domain::hub_request::log::MAX_LOG_PAGE;
use crate::domain::hub_request::passkeys::HubTrustedPasskey;
use crate::domain::hub_request::{HubLocalPolicy, HubRequestLogRow};
use crate::domain::hub_socket::connection::HubConnection;
use crate::ports::hub::{
    BrowserLauncher, HubPasskeyReader, HubPolicyReader, HubPolicyWriter, HubRequestRepository,
};
use crate::state::AppContext;

/// Tauri state of its own, never a field of [`AppContext`]: the Hub request
/// handler is built over the context, and nothing it can reach may write the
/// policy it is judged by (docs/HUB.md §7). [`hub_policy_set`] is the only
/// production user of this.
pub struct HubPolicySettings(pub Arc<dyn HubPolicyWriter>);

/// Opens `/authorize` from Rust, which the opener plugin allows without a
/// capability: only its JS `open_url` command is gated by
/// `src-tauri/capabilities/`.
pub struct OpenerLauncher(AppHandle);

impl BrowserLauncher for OpenerLauncher {
    fn open(&self, url: &str) -> Result<(), String> {
        self.0
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())
    }
}

fn pairing(ctx: &AppContext) -> Result<&HubPairing, String> {
    ctx.hub
        .as_deref()
        .ok_or_else(|| "Hub pairing is not available in this mode".to_owned())
}

pub async fn pair_core(
    hub: &HubPairing,
    hub_url: &str,
    launcher: &dyn BrowserLauncher,
) -> Result<HubStatus, String> {
    hub.pair(hub_url, launcher, PAIR_TIMEOUT)
        .await
        .map_err(|e| e.to_string())
}

pub fn status_core(hub: &HubPairing) -> Result<HubStatus, String> {
    hub.status().map_err(|e| e.to_string())
}

pub fn connection_core(hub: &HubPairing) -> Result<HubConnection, String> {
    Ok(hub.connection())
}

pub fn machine_core(hub: &HubPairing) -> Result<HubMachine, String> {
    Ok(hub.machine())
}

pub async fn disconnect_core(hub: &HubPairing) -> Result<HubStatus, String> {
    hub.disconnect().await.map_err(|e| e.to_string())
}

pub fn policy_get_core(reader: &dyn HubPolicyReader) -> Result<HubLocalPolicy, String> {
    reader.load().map_err(|e| e.to_string())
}

pub async fn policy_set_core(
    reader: &dyn HubPolicyReader,
    writer: &dyn HubPolicyWriter,
    hub: Option<&HubPairing>,
    policy: &HubLocalPolicy,
) -> Result<(), String> {
    save_hub_policy(reader, writer, hub, policy)
        .await
        .map_err(|e| e.to_string())
}

pub fn request_log_core(
    requests: &dyn HubRequestRepository,
    limit: usize,
) -> Result<Vec<HubRequestLogRow>, String> {
    requests
        .list_log(limit.min(MAX_LOG_PAGE))
        .map_err(|e| e.to_string())
}

pub fn trusted_passkeys_core(
    reader: &dyn HubPasskeyReader,
) -> Result<Vec<HubTrustedPasskey>, String> {
    reader.listed().map_err(|e| e.to_string())
}

/// Waits up to [`PAIR_TIMEOUT`] for the browser's answer; `async`, so on
/// Tauri's runtime rather than the main thread.
#[tauri::command]
pub async fn hub_pair(
    app: AppHandle,
    ctx: State<'_, AppContext>,
    hub_url: String,
) -> Result<HubStatus, String> {
    pair_core(pairing(&ctx)?, &hub_url, &OpenerLauncher(app)).await
}

#[tauri::command]
pub fn hub_status(ctx: State<'_, AppContext>) -> Result<HubStatus, String> {
    status_core(pairing(&ctx)?)
}

#[tauri::command]
pub fn hub_connection(ctx: State<'_, AppContext>) -> Result<HubConnection, String> {
    connection_core(pairing(&ctx)?)
}

#[tauri::command]
pub fn hub_machine(ctx: State<'_, AppContext>) -> Result<HubMachine, String> {
    machine_core(pairing(&ctx)?)
}

#[tauri::command]
pub async fn hub_disconnect(ctx: State<'_, AppContext>) -> Result<HubStatus, String> {
    disconnect_core(pairing(&ctx)?).await
}

#[tauri::command]
pub fn hub_policy_get(ctx: State<'_, AppContext>) -> Result<HubLocalPolicy, String> {
    policy_get_core(ctx.hub_policy.as_ref())
}

#[tauri::command]
pub async fn hub_policy_set(
    ctx: State<'_, AppContext>,
    settings: State<'_, HubPolicySettings>,
    policy: HubLocalPolicy,
) -> Result<(), String> {
    policy_set_core(
        ctx.hub_policy.as_ref(),
        settings.0.as_ref(),
        ctx.hub.as_deref(),
        &policy,
    )
    .await
}

#[tauri::command]
pub fn hub_trusted_passkeys(ctx: State<'_, AppContext>) -> Result<Vec<HubTrustedPasskey>, String> {
    trusted_passkeys_core(ctx.hub_passkeys.as_ref())
}

/// Newest first.
#[tauri::command]
pub fn hub_request_log(
    ctx: State<'_, AppContext>,
    limit: usize,
) -> Result<Vec<HubRequestLogRow>, String> {
    request_log_core(ctx.hub_requests.as_ref(), limit)
}

#[cfg(test)]
#[path = "../../tests/infrastructure/hub.rs"]
mod tests;
