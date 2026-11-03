//! This runner's pairing with the Demeteo Hub (docs/HUB.md §4 *Pairing a
//! runner by device code*, decision 64).
//!
//! `serve` owns the pairing, its sealed secret store and its socket, so the
//! store key is loaded by one process only. Every `demeteo-runner hub …`
//! subcommand but `seal` is a client of the control socket (`rpc/hub.rs`),
//! the same methods a desktop reaches over SSH.

pub mod cli;
mod client;
mod runs;
pub mod seal;

pub use runs::RunnerHubRuns;

use demeteo_core::adapters::hub::HubPairing;
use demeteo_core::ports::hub::{
    HubPolicyReader, HubPolicyWriter, HubRequestRepository, HubSecretStore,
};
use std::sync::Arc;

/// What the hub control methods act on. The policy writer is here, not
/// reachable from the context, for the reason
/// `build_core_context_with_hub_store` returns it beside the context. So this
/// is never a field of [`crate::services::RunnerServices`]: the request
/// handler holds those services, and nothing it can reach may write the
/// policy (docs/HUB.md §7).
pub struct HubControl {
    /// `None` only when composition had no HTTP client for the Hub.
    pub pairing: Option<Arc<HubPairing>>,
    /// The store `pairing` holds, kept here for why it is unavailable, which
    /// `HubStatus` does not carry.
    pub secrets: Arc<dyn HubSecretStore>,
    pub policy: Arc<dyn HubPolicyReader>,
    pub policy_writer: Arc<dyn HubPolicyWriter>,
    /// Read for the log of what the Hub asked here.
    pub requests: Arc<dyn HubRequestRepository>,
}
