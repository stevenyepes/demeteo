//! Bundles the engine `AppContext` with runner-only, non-engine state
//! (the in-memory credential store and the askpass helper path) so
//! `run.rs` / `rpc/` / `reconcile.rs` only have to thread one handle.

use crate::away_notify::AwayNotifier;
use crate::credentials::CredentialStore;
use crate::driver::RunDrivers;
use crate::hub::HubControl;
use demeteo_core::state::AppContext;
use std::path::PathBuf;
use std::sync::Arc;

pub struct RunnerServices {
    pub ctx: Arc<AppContext>,
    pub creds: Arc<CredentialStore>,
    pub askpass_path: PathBuf,
    /// "Runner-push while away" channel (M6.3) — best-effort, never
    /// fails the run it's reporting on.
    pub away_notifier: Arc<dyn AwayNotifier>,
    pub drivers: Arc<RunDrivers>,
    pub hub: HubControl,
}
