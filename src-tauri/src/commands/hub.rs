//! Desktop pairing with the Demeteo Hub. Every decision is
//! `adapters::hub::HubPairing`'s; these only unwrap `AppContext.hub` and map
//! its error to the IPC string.

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::adapters::hub::HubPairing;
use crate::domain::hub_pairing::{HubStatus, PAIR_TIMEOUT};
use crate::ports::hub::BrowserLauncher;
use crate::state::AppContext;

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

pub fn disconnect_core(hub: &HubPairing) -> Result<HubStatus, String> {
    hub.disconnect().map_err(|e| e.to_string())
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
pub fn hub_disconnect(ctx: State<'_, AppContext>) -> Result<HubStatus, String> {
    disconnect_core(pairing(&ctx)?)
}

#[cfg(test)]
#[path = "../../tests/infrastructure/hub.rs"]
mod tests;
