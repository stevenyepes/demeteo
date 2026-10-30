//! The Hub pairing methods of the control socket (docs/HUB.md §4 *Pairing a
//! runner by device code*): the `demeteo-runner hub` CLI and a desktop over
//! SSH both call these.
//!
//! They are host-level, so they skip the `client_id` ownership check every
//! run-scoped method makes. `client_id` is a tenant label for runs, not a
//! credential: whoever can open this `0600` socket is the runner's own user,
//! who can already stop the service, read its database and re-seal its key.
//! A pairing belongs to the host, and there is no client it could be hidden
//! from.

use crate::hub::HubControl;
use demeteo_core::adapters::hub::HubPairing;
use demeteo_core::application::hub_settings::save_hub_policy;
use demeteo_core::domain::hub_pairing::device::{DevicePairing, DeviceTicket};
use demeteo_core::domain::hub_pairing::{HubMachine, HubPairingError, HubStatus};
use demeteo_core::domain::hub_request::HubLocalPolicy;
use demeteo_core::domain::hub_socket::connection::HubConnection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

const SEAL_HINT: &str = "run `demeteo-runner hub seal`, then restart the runner \
     (`systemctl --user restart demeteo-runner`)";

#[derive(Debug, Deserialize)]
struct PairBeginParams {
    hub_url: String,
}

#[derive(Debug, Deserialize)]
struct PolicySetParams {
    policy: HubLocalPolicy,
}

/// `hub_pair_state` on the wire. A success reads `idle`; `hub_status` is what
/// says paired.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum PairStateView {
    Idle,
    Pending { ticket: DeviceTicket },
    Failed { error: String },
}

#[derive(Debug, Serialize)]
struct HubStatusView {
    status: HubStatus,
    /// Why the sealed store cannot hold the Hub credentials, when it cannot.
    store_unavailable: Option<String>,
    connection: HubConnection,
    machine: HubMachine,
}

/// The `hub_*` methods, or `None` for any other, so `dispatch` falls through
/// to the run-scoped ones.
pub(super) async fn dispatch_hub(
    hub: &HubControl,
    method: &str,
    params: &Value,
) -> Option<Result<Value, String>> {
    fn json<T: Serialize>(result: Result<T, String>) -> Result<Value, String> {
        result.and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
    }
    Some(match method {
        "hub_pair_begin" => json(pair_begin(hub, params).await),
        "hub_pair_state" => json(pairing(hub).map(|p| pair_state(p))),
        "hub_pair_cancel" => json(pair_cancel(hub).await),
        "hub_status" => json(status(hub)),
        "hub_disconnect" => json(disconnect(hub).await),
        "hub_policy_get" => json(hub.policy.load().map_err(|e| e.to_string())),
        "hub_policy_set" => json(policy_set(hub, params).await),
        _ => return None,
    })
}

fn pairing(hub: &HubControl) -> Result<&Arc<HubPairing>, String> {
    hub.pairing.as_ref().ok_or_else(|| {
        "Hub pairing is not available on this runner: it has no HTTP client for the Hub \
         (see its log)"
            .to_owned()
    })
}

/// What a refused `hub_pair_begin` says. A store without its key is the one
/// refusal the runner's own user fixes on this host, so it says how.
fn pairing_refusal(e: &HubPairingError) -> String {
    match e {
        HubPairingError::SecretStoreUnavailable { .. } | HubPairingError::KeyringUnavailable => {
            format!("cannot pair: {e}; {SEAL_HINT}")
        }
        _ => format!("cannot pair: {e}"),
    }
}

fn pair_state(hub: &HubPairing) -> PairStateView {
    match hub.device_pairing() {
        DevicePairing::Idle => PairStateView::Idle,
        DevicePairing::Pending(ticket) => PairStateView::Pending { ticket },
        DevicePairing::Failed(e) => PairStateView::Failed {
            error: e.to_string(),
        },
    }
}

async fn pair_begin(hub: &HubControl, params: &Value) -> Result<DeviceTicket, String> {
    let params =
        PairBeginParams::deserialize(params).map_err(|e| format!("invalid params: {}", e))?;
    pairing(hub)?
        .begin_device_pairing(&params.hub_url)
        .await
        .map_err(|e| pairing_refusal(&e))
}

async fn pair_cancel(hub: &HubControl) -> Result<PairStateView, String> {
    let pairing = pairing(hub)?;
    pairing.cancel_device_pairing().await;
    Ok(pair_state(pairing))
}

fn status(hub: &HubControl) -> Result<HubStatusView, String> {
    let pairing = pairing(hub)?;
    let store = hub.secrets.status();
    Ok(HubStatusView {
        status: pairing.status().map_err(|e| e.to_string())?,
        store_unavailable: (!store.available).then(|| {
            let reason = store.reason.unwrap_or("no reason given");
            format!("{reason}; {SEAL_HINT}")
        }),
        connection: pairing.connection(),
        machine: pairing.machine(),
    })
}

async fn disconnect(hub: &HubControl) -> Result<HubStatus, String> {
    pairing(hub)?.disconnect().await.map_err(|e| e.to_string())
}

/// Answers the policy as stored afterwards, so a caller sees what is in force.
async fn policy_set(hub: &HubControl, params: &Value) -> Result<HubLocalPolicy, String> {
    let params =
        PolicySetParams::deserialize(params).map_err(|e| format!("invalid params: {}", e))?;
    save_hub_policy(
        hub.policy.as_ref(),
        hub.policy_writer.as_ref(),
        hub.pairing.as_deref(),
        &params.policy,
    )
    .await
    .map_err(|e| e.to_string())?;
    hub.policy.load().map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "hub_tests.rs"]
mod tests;
