//! The desktop's half of the Hub socket — the I/O half (docs/HUB.md §5).
//! What crosses it, and what follows a connection's end, is
//! [`crate::domain::hub_socket`]'s to decide; this module only performs it.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::runtime::Handle;
use tokio::sync::{watch, Notify};
use tokio::task::JoinHandle;

use crate::domain::hub_pairing::{socket_dial_url, socket_url, SecretString};
use crate::domain::hub_socket::{
    after_close, after_refresh, backoff, resets_backoff, Reconnect, RefreshEnd, SocketEnd,
    StopReason,
};
use crate::ports::hub::{DialFailure, HubSnapshotSource, HubSocketDialer, SocketUpgrade};
use crate::ports::run_events::RunEventsPort;
use crate::shared::time::now_secs;

use self::session::{run_session, SessionContext, SessionEnd};
use super::device_key::DeviceKey;
use super::{random_token, HubPairing, Refresher};

pub mod session;
mod snapshot_send;
pub mod snapshot_wake;
pub mod wake;
#[cfg(feature = "hub-socket")]
pub mod ws_transport;

/// Where the supervisor gets its access token. [`HubPairing`] is one; the
/// trait exists so the socket can hold the pairing's refresh state without
/// holding the pairing that owns the socket.
#[async_trait]
pub trait AccessTokens: Send + Sync {
    /// See [`HubPairing::access_token`].
    fn access_token(&self) -> Option<SecretString>;
    /// See [`HubPairing::access_expires_at`].
    fn access_expires_at(&self) -> Option<u64>;
    /// See [`HubPairing::refresh_access`].
    async fn refresh_access(&self) -> Result<SecretString, RefreshEnd>;
}

#[async_trait]
impl AccessTokens for HubPairing {
    fn access_token(&self) -> Option<SecretString> {
        HubPairing::access_token(self)
    }

    fn access_expires_at(&self) -> Option<u64> {
        HubPairing::access_expires_at(self)
    }

    async fn refresh_access(&self) -> Result<SecretString, RefreshEnd> {
        HubPairing::refresh_access(self).await
    }
}

#[async_trait]
impl AccessTokens for Refresher {
    fn access_token(&self) -> Option<SecretString> {
        Refresher::access_token(self)
    }

    fn access_expires_at(&self) -> Option<u64> {
        Refresher::access_expires_at(self)
    }

    async fn refresh_access(&self) -> Result<SecretString, RefreshEnd> {
        Refresher::refresh_access(self).await
    }
}

/// What [`HubPairing::start_socket`] needs that the pairing does not hold.
pub struct SocketConfig {
    pub dialer: Arc<dyn HubSocketDialer>,
    /// See [`SessionContext::run_events`] and [`SessionContext::wake`]: the
    /// two have to come from the same [`wake::WakingRunEvents`].
    pub run_events: Arc<dyn RunEventsPort>,
    pub wake: Arc<Notify>,
    /// See [`SessionContext::snapshots`] and [`SessionContext::snapshot_wake`].
    pub snapshots: Arc<dyn HubSnapshotSource>,
    pub snapshot_wake: Arc<Notify>,
    /// See [`SessionContext::snapshot_debounce`].
    pub snapshot_debounce: Duration,
    /// Where the socket task is spawned; `start_socket` is synchronous, so
    /// it cannot take the caller's runtime for granted.
    pub runtime: Handle,
    /// [`TokioSleeper`] and [`os_jitter`] in the app. A test passes its own
    /// so a backoff never waits real time, and a redial never depends on
    /// how fast the test reaches its next step.
    pub sleeper: Arc<dyn Sleeper>,
    pub jitter: fn() -> f64,
}

/// A running [`supervise`], and the stop signal it obeys.
pub(super) struct SocketTask {
    handle: JoinHandle<StopReason>,
    stop: watch::Sender<bool>,
}

impl SocketTask {
    pub(super) fn spawn(runtime: &Handle, deps: SupervisorDeps) -> Self {
        let (stop, stopped) = watch::channel(false);
        Self {
            handle: runtime.spawn(supervise(deps, stopped)),
            stop,
        }
    }

    pub(super) fn is_running(&self) -> bool {
        !self.handle.is_finished()
    }

    pub(super) fn stop(&self) {
        self.stop.send_replace(true);
    }

    /// Resolves once the task has ended and dropped what it held.
    pub(super) async fn ended(self) {
        if let Err(e) = self.handle.await {
            tracing::warn!(error = %e, "the Hub socket task did not end cleanly");
        }
    }
}

/// The supervisor's only way to wait, so a test never waits real seconds.
#[async_trait]
pub trait Sleeper: Send + Sync {
    async fn sleep(&self, duration: Duration);
}

pub struct TokioSleeper;

#[async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// A jitter for [`backoff`] from the OS, or `1.0` without one: a desktop
/// that cannot spread its redials still waits the full time.
pub fn os_jitter() -> f64 {
    let mut bytes = [0u8; 4];
    match getrandom::fill(&mut bytes) {
        Ok(()) => 0.5 + 0.5 * f64::from(u32::from_le_bytes(bytes)) / f64::from(u32::MAX),
        Err(_) => 1.0,
    }
}

/// Everything one socket's lifetime reads. `origin`, `key` and the
/// session's pinned scopes are the pairing's, read once: a change to any of
/// them is a new pairing, and [`HubPairing::pair`] replaces the running
/// socket with one built from it.
pub struct SupervisorDeps {
    pub tokens: Arc<dyn AccessTokens>,
    pub origin: String,
    pub key: DeviceKey,
    pub dialer: Arc<dyn HubSocketDialer>,
    pub session: SessionContext,
    pub sleeper: Arc<dyn Sleeper>,
    pub jitter: Box<dyn FnMut() -> f64 + Send>,
}

/// Dials, runs a session, and obeys [`after_close`] on how it ended, until a
/// verdict stops it or `stop` is raised (or its sender dropped), which ends
/// it as [`StopReason::Stopped`] from whatever it was awaiting — a refresh
/// included, so `disconnect` can take the refresh lock after it.
pub async fn supervise(deps: SupervisorDeps, mut stop: watch::Receiver<bool>) -> StopReason {
    let mut supervisor = Supervisor { deps, attempt: 0 };
    let reason = match supervisor.run(&mut stop).await {
        Err(reason) => reason,
        Ok(never) => match never {},
    };
    if reason != StopReason::Stopped {
        tracing::warn!(?reason, "the Hub socket stopped");
    }
    reason
}

struct Supervisor {
    deps: SupervisorDeps,
    /// Backoffs since the last session [`resets_backoff`] counted.
    attempt: u32,
}

impl Supervisor {
    async fn run(
        &mut self,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<std::convert::Infallible, StopReason> {
        // An origin that does not dial, or a proof that cannot be signed,
        // is local state the next attempt would meet unchanged.
        let dial_url = socket_dial_url(&self.deps.origin).map_err(|_| StopReason::Refused)?;
        let htu = socket_url(&self.deps.origin);
        let mut refreshed: Option<SecretString> = None;
        loop {
            let (access_token, fresh_token) = match refreshed.take() {
                Some(token) => (token, true),
                None => match self.deps.tokens.access_token() {
                    Some(token) => (token, false),
                    None => {
                        refreshed = self.obey(Reconnect::RefreshNow, stop).await?;
                        continue;
                    }
                },
            };
            let expires_at = self.deps.tokens.access_expires_at();
            let jti = random_token().map_err(|_| StopReason::Refused)?;
            let dpop = self
                .deps
                .key
                .dpop_proof_for("GET", &htu, now_secs(), &jti, Some(access_token.expose()))
                .map_err(|_| StopReason::Refused)?;
            let upgrade = SocketUpgrade {
                url: dial_url.clone(),
                access_token,
                dpop,
            };
            let (end, acked) = match unless_stopped(stop, self.deps.dialer.dial(upgrade)).await? {
                Err(DialFailure::Refused { status, .. }) => (
                    SocketEnd::Refused {
                        status,
                        fresh_token,
                    },
                    false,
                ),
                Err(DialFailure::Unreachable(_)) => (SocketEnd::Unreachable, false),
                Ok(conn) => match run_session(conn, &self.deps.session, stop).await {
                    (SessionEnd::Stopped, _) => return Err(StopReason::Stopped),
                    (SessionEnd::Ended(end), acked) => (end, acked),
                },
            };
            let secs_left = expires_at.map(|at| at.saturating_sub(now_secs()));
            let verdict = after_close(end, fresh_token, secs_left);
            if resets_backoff(acked, verdict) {
                self.attempt = 0;
            }
            refreshed = self.obey(verdict, stop).await?;
        }
    }

    /// Performs `verdict`: the access token a refresh it called for
    /// returned, `None` after a backoff, or the reason to stop.
    async fn obey(
        &mut self,
        mut verdict: Reconnect,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Option<SecretString>, StopReason> {
        loop {
            match verdict {
                Reconnect::Stop(reason) => return Err(reason),
                Reconnect::AfterBackoff => {
                    self.back_off(stop).await?;
                    return Ok(None);
                }
                Reconnect::RefreshAfterBackoff => {
                    self.back_off(stop).await?;
                    verdict = Reconnect::RefreshNow;
                }
                Reconnect::RefreshNow => {
                    match unless_stopped(stop, self.deps.tokens.refresh_access()).await? {
                        Ok(token) => return Ok(Some(token)),
                        Err(end) => verdict = after_refresh(end),
                    }
                }
            }
        }
    }

    async fn back_off(&mut self, stop: &mut watch::Receiver<bool>) -> Result<(), StopReason> {
        let wait = backoff(self.attempt, (self.deps.jitter)());
        self.attempt = self.attempt.saturating_add(1);
        unless_stopped(stop, self.deps.sleeper.sleep(wait)).await
    }
}

/// `work`, unless `stop` is raised first; a raised or dropped `stop` drops
/// `work` where it stands.
async fn unless_stopped<T>(
    stop: &mut watch::Receiver<bool>,
    work: impl Future<Output = T>,
) -> Result<T, StopReason> {
    tokio::select! {
        biased;
        _ = stop.wait_for(|stop| *stop) => Err(StopReason::Stopped),
        done = work => Ok(done),
    }
}
