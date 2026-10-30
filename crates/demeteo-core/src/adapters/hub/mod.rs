//! Pairing with the Demeteo Hub — the choreography half, for a desktop by
//! loopback redirect ([`HubPairing::pair`]) and for a runner by device code
//! ([`HubPairing::begin_device_pairing`]). Every decision is
//! [`crate::domain::hub_pairing`]'s; this module only orders the I/O, and the
//! order is the contract (spec §4.3), the same for both grants:
//!
//! - The device key reaches the keyring **before** `/authorize` (or
//!   `/device_authorization`), which binds the pairing to its thumbprint. A key lost after that point would leave a
//!   pairing nobody can prove.
//! - The refresh token reaches the keyring before the pins reach SQLite, and
//!   is deleted again if they do not. Pins without a token read as
//!   `incomplete`; a token without pins would be a credential nothing names.
//! - The access token lives in memory only, never in a store, and only
//!   after the refresh token it came with is in the keyring. A rotation
//!   whose new refresh token is lost has unpaired this desktop; one whose
//!   access token is lost costs a refresh.
//! - Once the pins are saved, still under the refresh lock, a pairing stops
//!   the socket already running, awaits its end, and starts one on the new
//!   pins: a re-pairing is how a user narrows scopes, and a socket left on
//!   the old ones would keep announcing them to the Hub. A refresh that
//!   socket has queued on the lock is dropped by its stop signal, so the
//!   wait cannot deadlock. `disconnect` stops it the same way before it
//!   deletes anything (see [`HubPairing::disconnect`]).
//!
//! Nothing here formats a token, the code, the verifier or the PKCS#8 into
//! an error or a `tracing` field.

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use tokio::sync::{watch, OwnedMutexGuard};
use tokio::task::JoinHandle;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use crate::domain::hub_pairing::answer::{refresh_form_body, token_form_body};
use crate::domain::hub_pairing::device::{
    after_device_poll, device_authorization_form_body, device_authorization_url,
    device_token_form_body, DeviceGrant, DevicePairing, DevicePollStep, DeviceTicket,
};
use crate::domain::hub_pairing::keyring_names::hub_keyring_account;
use crate::domain::hub_pairing::pkce::s256_challenge;
use crate::domain::hub_pairing::{
    authorize_url, check_pairing_origin, consent_machine, hub_origin, hub_status, token_url,
    AuthorizeRequest, HubMachine, HubPairingError, HubSecret, HubStatus, PairingAnswer,
    SecretString, SCOPES,
};
use crate::domain::hub_socket::connection::{
    after_local_restart, view, ConnectionFacts, HubConnection,
};
use crate::domain::hub_socket::{sends_events, RefreshEnd};
use crate::error::AppError;
use crate::ports::hub::{
    BrowserLauncher, HubAttachmentFetcher, HubDeviceEndpoint, HubPinRepository, HubPolicyReader,
    HubRefreshEndpoint, HubRequestHandler, HubSecretStore, HubTokenEndpoint,
};
use crate::shared::time::now_secs;
use crate::state::AppVersion;

use self::attachments::{HubAttachments, PairedAttachments};
use self::device_key::DeviceKey;
use self::loopback::CallbackListener;
use self::socket::session::{RequestHandlerSlot, SessionContext};
use self::socket::status::ConnectionCell;
use self::socket::{Sleeper, SocketConfig, SocketTask, SupervisorDeps, TokioSleeper};

pub mod attachments;
pub mod device_key;
#[cfg(feature = "keyring")]
pub mod keyring_store;
pub mod loopback;
pub mod sealed_store;
pub mod snapshot_source;
pub mod socket;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod token_client;

pub struct HubPairing {
    token: Arc<dyn HubTokenEndpoint>,
    /// Read each time a socket starts, for whether it relays (see
    /// [`sends_events`]).
    policy: Arc<dyn HubPolicyReader>,
    app_version: AppVersion,
    /// Held for a whole `pair`, and by a device pairing's poll task for as
    /// long as it polls. A tokio mutex because the guard lives across the
    /// browser wait, and `try_lock` because a second pairing is refused,
    /// not queued: it would race the first for the same keyring entries.
    /// `disconnect` is refused by it too, for the same reason: it would
    /// delete the device key `pair` still holds in memory, and `pair` would
    /// go on to store a refresh token and pins naming a key no longer there.
    /// A device pairing is cancelled instead, and its task awaited, before
    /// either takes it (see [`HubPairing::begin_device_pairing`]).
    in_flight: Arc<tokio::sync::Mutex<()>>,
    device: Option<Arc<dyn HubDeviceEndpoint>>,
    device_sleeper: Arc<dyn Sleeper>,
    device_flow: std::sync::Mutex<DeviceFlow>,
    refresher: Refresher,
    socket_config: Option<SocketConfig>,
    socket: std::sync::Mutex<Option<SocketTask>>,
    requests: RequestHandlerSlot,
    connection: ConnectionCell,
}

/// The pairing's refresh state, shared with the socket it starts: the
/// socket task holds a clone of this, never the [`HubPairing`] that owns
/// the task, which would be a cycle nothing drops.
#[derive(Clone)]
struct Refresher {
    pins: Arc<dyn HubPinRepository>,
    secrets: Arc<dyn HubSecretStore>,
    endpoint: Option<Arc<dyn HubRefreshEndpoint>>,
    /// The single writer of the stored refresh token (see
    /// [`HubSecretStore`]): `refresh_access` holds it from reading the token
    /// to storing its rotation, `pair` while it writes a new one, and
    /// `disconnect` while it deletes it. Queued, not refused, so a refresh
    /// waits its turn instead of failing.
    ///
    /// Not `in_flight`: `pair` holds that across a browser wait of minutes,
    /// which would stall every refresh behind it, and a pairing refused by
    /// `try_lock` because a refresh was running would report
    /// `PairingInProgress` when none was.
    lock: Arc<tokio::sync::Mutex<()>>,
    access: Arc<std::sync::Mutex<Option<HeldAccess>>>,
}

/// The device pairing in progress, if any, and the state the last one left.
struct DeviceFlow {
    state: DevicePairing,
    cancel: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
}

/// What both grants have in hand once the store holds a device key.
struct Prepared {
    origin: String,
    install_id: String,
    device_key: DeviceKey,
}

/// The access token, in this process's memory and nowhere else.
struct HeldAccess {
    token: SecretString,
    /// Unix seconds, by the local clock.
    expires_at: u64,
}

fn storage(e: AppError) -> HubPairingError {
    HubPairingError::Storage(e.to_string())
}

fn not_paired(what: &str) -> RefreshEnd {
    RefreshEnd::Refused(HubPairingError::Storage(format!(
        "cannot refresh the Hub access token: {what}"
    )))
}

/// 32 bytes from the OS, base64url: the `state`, the PKCE verifier and the
/// proof's `jti` are all unreserved-only and well inside the Hub's caps.
fn random_token() -> Result<String, HubPairingError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| HubPairingError::Storage(format!("no OS randomness: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

impl HubPairing {
    pub fn new(
        pins: Arc<dyn HubPinRepository>,
        secrets: Arc<dyn HubSecretStore>,
        token: Arc<dyn HubTokenEndpoint>,
        policy: Arc<dyn HubPolicyReader>,
    ) -> Self {
        Self {
            token,
            policy,
            app_version: AppVersion::default(),
            in_flight: Arc::default(),
            device: None,
            device_sleeper: Arc::new(TokioSleeper),
            device_flow: std::sync::Mutex::new(DeviceFlow {
                state: DevicePairing::Idle,
                cancel: None,
                task: None,
            }),
            refresher: Refresher {
                pins,
                secrets,
                endpoint: None,
                lock: Arc::default(),
                access: Arc::default(),
            },
            socket_config: None,
            socket: std::sync::Mutex::new(None),
            requests: RequestHandlerSlot::default(),
            connection: ConnectionCell::default(),
        }
    }

    /// Without one, [`Self::refresh_access`] fails rather than refreshing.
    pub fn with_refresh_endpoint(mut self, endpoint: Arc<dyn HubRefreshEndpoint>) -> Self {
        self.refresher.endpoint = Some(endpoint);
        self
    }

    /// Without one, [`Self::begin_device_pairing`] is refused.
    pub fn with_device_endpoint(mut self, endpoint: Arc<dyn HubDeviceEndpoint>) -> Self {
        self.device = Some(endpoint);
        self
    }

    /// What a device pairing waits between polls with; a test's returns at
    /// once.
    pub fn with_device_sleeper(mut self, sleeper: Arc<dyn Sleeper>) -> Self {
        self.device_sleeper = sleeper;
        self
    }

    /// Without one, [`Self::start_socket`] starts nothing.
    pub fn with_socket(mut self, config: SocketConfig) -> Self {
        self.socket_config = Some(config);
        self
    }

    /// What composition handed the socket to wait on, so a test can prove an
    /// append through `AppContext::run_events` fires it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn socket_wake(&self) -> Option<Arc<tokio::sync::Notify>> {
        self.socket_config
            .as_ref()
            .map(|config| config.wake.clone())
    }

    /// Whether [`Self::bind_requests`] has run, so a test can prove
    /// composition binds the handler.
    #[cfg(any(test, feature = "test-support"))]
    pub fn requests_bound(&self) -> bool {
        self.requests.get().is_some()
    }

    /// What every socket this pairing starts hands request frames to. A
    /// slot rather than a [`SocketConfig`] field because the handler is
    /// built over the context that holds this pairing; bound once; a second
    /// binding is ignored.
    pub fn bind_requests(&self, handler: Arc<dyn HubRequestHandler>) {
        if self.requests.set(handler).is_err() {
            tracing::warn!("the Hub request handler was already bound");
        }
    }

    /// What composition handed the socket to wait on for a snapshot, as
    /// [`Self::socket_wake`] is for `run_events`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot_wake(&self) -> Option<Arc<tokio::sync::Notify>> {
        self.socket_config
            .as_ref()
            .map(|config| config.snapshot_wake.clone())
    }

    /// The version shown on the Hub's consent page. Unset, the page shows
    /// `unknown` rather than `CARGO_PKG_VERSION` (see [`AppVersion`]).
    pub fn with_app_version(mut self, app_version: AppVersion) -> Self {
        self.app_version = app_version;
        self
    }

    /// What the Hub's consent page was, or would be, shown for this machine.
    pub fn machine(&self) -> HubMachine {
        consent_machine(
            &host_name().unwrap_or_default(),
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.app_version.get(),
        )
    }

    pub async fn pair(
        &self,
        hub_url: &str,
        launcher: &dyn BrowserLauncher,
        timeout: Duration,
    ) -> Result<HubStatus, HubPairingError> {
        let _in_flight = self
            .in_flight
            .try_lock()
            .map_err(|_| HubPairingError::PairingInProgress)?;

        let Prepared {
            origin,
            install_id,
            device_key,
        } = self.prepare(hub_url)?;
        let jkt = device_key.jkt();

        let (listener, redirect) = CallbackListener::bind()?;
        let state = random_token()?;
        let verifier = random_token()?;
        let machine = self.machine();
        let request = AuthorizeRequest {
            code_challenge: s256_challenge(&verifier),
            redirect_uri: redirect.clone(),
            dpop_jkt: jkt.clone(),
            scope: SCOPES.to_owned(),
            state: state.clone(),
            instance_name: machine.instance_name,
            os: machine.os,
            arch: machine.arch,
            app_version: machine.app_version,
        };
        launcher
            .open(&authorize_url(&origin, &request)?)
            .map_err(|_| {
                HubPairingError::Transport("could not open a browser on the Hub".into())
            })?;

        let code = listener.wait(&state, timeout).await?;

        let token_url = token_url(&origin);
        let now = now_secs();
        let proof = device_key.dpop_proof(&token_url, now, &random_token()?)?;
        let answer = self
            .token
            .exchange(
                &token_url,
                token_form_body(&code, &verifier, &redirect),
                proof,
            )
            .await?;

        self.commit(&origin, &jkt, &install_id, answer, now).await
    }

    /// What both grants do before the Hub is asked anything: the origin
    /// checked against any existing pairing, the store checked for
    /// availability (refused with its reason, when it gives one), and the
    /// device key loaded, or created and stored.
    fn prepare(&self, hub_url: &str) -> Result<Prepared, HubPairingError> {
        let origin = hub_origin(hub_url)?;
        let paired = self.refresher.pins.load().map_err(storage)?;
        check_pairing_origin(paired.as_ref().map(|p| p.hub_origin.as_str()), &origin)?;

        let store = self.refresher.secrets.status();
        if !store.available {
            return Err(store.unavailable_error());
        }

        let install_id = self.refresher.pins.install_id().map_err(storage)?;
        let device_key = self.device_key(&install_id)?;
        Ok(Prepared {
            origin,
            install_id,
            device_key,
        })
    }

    /// What both grants do with the pairing response, in the order the
    /// module header gives. `now` is when the grant was asked, which the
    /// access token's expiry is counted from.
    async fn commit(
        &self,
        origin: &str,
        jkt: &str,
        install_id: &str,
        answer: PairingAnswer,
        now: u64,
    ) -> Result<HubStatus, HubPairingError> {
        let expires_at = now.saturating_add(answer.expires_in());
        let (pins, tokens) = answer.split(origin, jkt, i64::try_from(now).unwrap_or(i64::MAX));
        let refresh_account = hub_keyring_account(install_id, HubSecret::RefreshToken);
        let _refresh_lock = self.refresher.lock.lock().await;
        self.refresher
            .secrets
            .set(&refresh_account, tokens.refresh.expose())
            .map_err(storage)?;
        if let Err(save) = self.refresher.pins.save(&pins) {
            if let Err(e) = self.refresher.secrets.delete(&refresh_account) {
                tracing::warn!(error = %e, "could not remove the Hub refresh token after its pins failed to save");
            }
            return Err(storage(save));
        }
        self.refresher.hold_access(tokens.access, expires_at);
        if let Some(previous) = self.stop_socket() {
            previous.ended().await;
        }
        self.start_socket();

        self.status()
    }

    /// Starts a pairing by device code and returns once the Hub has answered
    /// `/device_authorization`, with what to show the human. Polling goes on
    /// in a task of its own, which [`Self::device_pairing`] reports on; an
    /// approval is committed exactly as [`Self::pair`] commits one, socket
    /// included.
    ///
    /// A pending device pairing is cancelled first and its task awaited, so
    /// a second call replaces the first; [`Self::disconnect`] and
    /// [`Self::cancel_device_pairing`] cancel the same way. A cancel lands
    /// while the task waits or polls, never while it commits, so once the
    /// cancel returns there is either nothing new stored or a whole pairing.
    /// Refused with [`HubPairingError::PairingInProgress`] while a
    /// [`Self::pair`] waits on the browser, and with the store's own reason,
    /// before any network call, when the store is unavailable.
    ///
    /// A pairing that fails keeps its device key, as [`Self::pair`]'s does,
    /// for the next attempt to reuse.
    pub async fn begin_device_pairing(
        self: &Arc<Self>,
        hub_url: &str,
    ) -> Result<DeviceTicket, HubPairingError> {
        let endpoint = self.device.clone().ok_or_else(|| {
            HubPairingError::Transport("device-code pairing is not configured".into())
        })?;
        self.cancel_device_pairing().await;
        let in_flight = self
            .in_flight
            .clone()
            .try_lock_owned()
            .map_err(|_| HubPairingError::PairingInProgress)?;

        let prepared = self.prepare(hub_url)?;
        let url = device_authorization_url(&prepared.origin);
        let now = now_secs();
        let proof = prepared
            .device_key
            .dpop_proof(&url, now, &random_token()?)?;
        let form = device_authorization_form_body(SCOPES, &self.machine());
        let grant = endpoint.authorize_device(&url, form, proof).await?.accept(
            &prepared.origin,
            prepared.device_key.fingerprint(),
            now,
        )?;
        let ticket = grant.ticket.clone();

        let (cancel, cancelled) = watch::channel(false);
        let mut flow = self.lock_device_flow();
        flow.state = DevicePairing::Pending(ticket.clone());
        flow.cancel = Some(cancel);
        flow.task = Some(tokio::spawn(
            Arc::clone(self).poll_device(endpoint, prepared, grant, cancelled, in_flight),
        ));
        Ok(ticket)
    }

    /// Idle, pending with its ticket, or how the last one failed. A success
    /// reads `Idle`; [`Self::status`] is what says paired.
    pub fn device_pairing(&self) -> DevicePairing {
        self.lock_device_flow().state.clone()
    }

    /// Stops a pending device pairing, awaits its task, and leaves `Idle`.
    /// With none pending it only clears a failure.
    pub async fn cancel_device_pairing(&self) {
        let (cancel, task) = {
            let mut flow = self.lock_device_flow();
            (flow.cancel.take(), flow.task.take())
        };
        if let Some(cancel) = cancel {
            cancel.send_replace(true);
        }
        if let Some(task) = task {
            if let Err(e) = task.await {
                tracing::warn!(error = %e, "the Hub device pairing task did not end cleanly");
            }
        }
        self.lock_device_flow().state = DevicePairing::Idle;
    }

    fn lock_device_flow(&self) -> std::sync::MutexGuard<'_, DeviceFlow> {
        self.device_flow
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Polls until the Hub answers for good, holding `_in_flight` throughout.
    /// Returns without touching the state when cancelled: the canceller
    /// sets it.
    async fn poll_device(
        self: Arc<Self>,
        endpoint: Arc<dyn HubDeviceEndpoint>,
        prepared: Prepared,
        grant: DeviceGrant,
        mut cancelled: watch::Receiver<bool>,
        _in_flight: OwnedMutexGuard<()>,
    ) {
        let token_url = token_url(&prepared.origin);
        let mut interval_secs = grant.interval_secs;
        let ended = loop {
            tokio::select! {
                () = self.device_sleeper.sleep(Duration::from_secs(interval_secs)) => {}
                _ = cancelled.wait_for(|cancelled| *cancelled) => return,
            }
            let now = now_secs();
            let proof = match random_token()
                .and_then(|jti| prepared.device_key.dpop_proof(&token_url, now, &jti))
            {
                Ok(proof) => proof,
                Err(e) => break Err(e),
            };
            let body = device_token_form_body(grant.device_code.expose());
            let polled = tokio::select! {
                polled = endpoint.poll_device(&token_url, body, proof) => polled,
                _ = cancelled.wait_for(|cancelled| *cancelled) => return,
            };
            let end = match polled {
                Ok(answer) => {
                    let jkt = prepared.device_key.jkt();
                    break self
                        .commit(&prepared.origin, &jkt, &prepared.install_id, answer, now)
                        .await
                        .map(|_| ());
                }
                Err(end) => end,
            };
            match after_device_poll(end, interval_secs, now_secs(), grant.ticket.expires_at) {
                DevicePollStep::Wait {
                    interval_secs: next,
                } => interval_secs = next,
                DevicePollStep::Fail(e) => break Err(e),
            }
        };
        self.lock_device_flow().state = match ended {
            Ok(()) => DevicePairing::Idle,
            Err(e) => DevicePairing::Failed(e),
        };
    }

    /// Rotates the stored refresh token for a new access token. One `/token`
    /// call, never retried, made under the refresh lock: a second caller
    /// waits and then presents the token this one stored. The rotated
    /// refresh token is in the keyring before the access token is held or
    /// returned; if that write fails the access token is dropped with it.
    ///
    /// A missing endpoint, pairing, device key or refresh token is
    /// [`RefreshEnd::Refused`] with nothing sent.
    pub async fn refresh_access(&self) -> Result<SecretString, RefreshEnd> {
        self.refresher.refresh_access().await
    }

    /// The access token last issued, while the local clock says it is
    /// valid. The Hub is the authority on expiry (a `4001` close, an upgrade
    /// `401`); this only spares a dial that is known to fail.
    pub fn access_token(&self) -> Option<SecretString> {
        self.refresher.access_token()
    }

    /// Unix seconds, by the local clock, at which the access token last
    /// issued expires, whether or not it already has.
    pub fn access_expires_at(&self) -> Option<u64> {
        self.refresher.access_expires_at()
    }

    /// Starts the socket when this desktop is paired, holds a refresh token
    /// and a device key, and has a [`SocketConfig`]; otherwise it dials
    /// nothing and spawns nothing. A socket already running is left alone,
    /// so a second start spawns no second task; only [`Self::pair`] and
    /// [`Self::restart_socket`] replace it, by stopping it first. One that
    /// stopped on a verdict is started again, since a new pairing is what
    /// most of those verdicts ask for. A local policy that cannot be read
    /// starts nothing: a socket started without it could relay what `read`
    /// was turned off to stop.
    pub fn start_socket(&self) {
        let Some(config) = &self.socket_config else {
            return;
        };
        let mut slot = self.socket.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_some_and(SocketTask::is_running) {
            return;
        }
        match self.socket_deps(config) {
            Ok(Some(deps)) => *slot = Some(SocketTask::spawn(&config.runtime, deps)),
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "the Hub socket was not started"),
        }
    }

    /// Replaces a running socket with one that reads the local policy anew,
    /// the way [`Self::pair`] replaces it with one on new pins: stopped,
    /// awaited, and started again under the refresh lock, so a
    /// [`Self::disconnect`] either stops the new socket or has cleared the
    /// pins it would start on. A socket that is not running, stopped on a
    /// verdict included, is left alone: a policy change is not the new
    /// pairing such a verdict asks for.
    ///
    /// Once it returns the connection never reads as stopped on this
    /// restart's account: it reads as not running until the new socket
    /// records its first state, and stays so if none starts, with the
    /// cursor already reported kept (see [`after_local_restart`]).
    pub async fn restart_socket(&self) {
        let running = self
            .socket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_if(|task| task.is_running());
        let Some(previous) = running else {
            return;
        };
        previous.stop();
        let _refresh_lock = self.refresher.lock.lock().await;
        previous.ended().await;
        self.connection.update(after_local_restart);
        self.start_socket();
    }

    fn socket_deps(
        &self,
        config: &SocketConfig,
    ) -> Result<Option<SupervisorDeps>, HubPairingError> {
        let Refresher { pins, secrets, .. } = &self.refresher;
        let Some(paired) = pins.load().map_err(storage)? else {
            return Ok(None);
        };
        let install_id = pins.install_id().map_err(storage)?;
        let refresh_account = hub_keyring_account(&install_id, HubSecret::RefreshToken);
        if secrets.get(&refresh_account).map_err(storage)?.is_none() {
            return Ok(None);
        }
        let Some(key) = self.refresher.stored_device_key(&install_id)? else {
            return Ok(None);
        };
        let read_enabled = self.policy.load().map_err(storage)?.read_enabled;
        let relaying = sends_events(&paired.scopes, read_enabled);
        Ok(Some(SupervisorDeps {
            tokens: Arc::new(self.refresher.clone()),
            origin: paired.hub_origin,
            key,
            dialer: config.dialer.clone(),
            session: SessionContext::new(
                install_id,
                self.app_version.get().to_owned(),
                paired.scopes,
                config.run_events.clone(),
                config.wake.clone(),
                config.snapshots.clone(),
                config.snapshot_wake.clone(),
            )
            .with_relaying(relaying)
            .with_requests(self.requests.clone())
            .with_connection(self.connection.clone())
            .with_snapshot_debounce(config.snapshot_debounce),
            sleeper: config.sleeper.clone(),
            jitter: Box::new(config.jitter),
        }))
    }

    /// A fetcher of the attachments a request names, over the pinned origin,
    /// the stored device key and this pairing's access token; `None` when
    /// this desktop is not paired or holds no device key. Made per request
    /// rather than held, so a re-pairing is never fetched for with the old
    /// origin or key.
    pub fn attachment_fetcher(&self) -> Result<Option<HubAttachments>, HubPairingError> {
        self.refresher.attachment_fetcher()
    }

    /// [`Self::attachment_fetcher`] as one long-lived fetcher, for a handler
    /// built once: it makes that per-request fetcher on every fetch, and
    /// holds the pairing's token state, never the pairing.
    pub fn attachments(&self) -> Arc<dyn HubAttachmentFetcher> {
        Arc::new(PairedAttachments::new(self.refresher.clone()))
    }

    /// Takes the running socket, if any, and raises its stop signal.
    fn stop_socket(&self) -> Option<SocketTask> {
        let task = self
            .socket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        task.stop();
        Some(task)
    }

    /// How the socket stands, as the last socket this pairing started left
    /// it; from memory only.
    pub fn connection(&self) -> HubConnection {
        view(&self.connection.get())
    }

    /// From local storage only; no network call.
    pub fn status(&self) -> Result<HubStatus, HubPairingError> {
        let Refresher { pins, secrets, .. } = &self.refresher;
        let pinned = pins.load().map_err(storage)?;
        let keyring = secrets.status();
        let (has_device_key, has_refresh_token) = if keyring.available {
            let install_id = pins.install_id().map_err(storage)?;
            let present = |secret| {
                secrets
                    .get(&hub_keyring_account(&install_id, secret))
                    .map(|value| value.is_some())
                    .map_err(storage)
            };
            (
                present(HubSecret::DeviceKey)?,
                present(HubSecret::RefreshToken)?,
            )
        } else {
            (false, false)
        };
        Ok(hub_status(
            pinned.as_ref(),
            keyring,
            has_device_key,
            has_refresh_token,
        ))
    }

    /// Forgets the Hub on this machine only: both keyring entries, then the
    /// pins. A pending device pairing is cancelled first. Refused with
    /// [`HubPairingError::PairingInProgress`] while a `pair` is waiting on
    /// the browser; it can be retried once that ends. The Hub has no revoke route, so the instance stays listed there
    /// and its refresh family lapses on its own within 30 days. Idempotent,
    /// and it touches no table but the pins — runs carry on unaffected.
    ///
    /// The device key goes too, so a later pairing proves a fresh one.
    ///
    /// The order is what keeps a rotated refresh token from landing after
    /// its delete:
    ///
    /// 1. The socket's stop signal is raised. A refresh the socket has in
    ///    flight is dropped where it stands, inside the endpoint included,
    ///    which releases the refresh lock with its keyring write unmade.
    /// 2. The refresh lock is taken. A refresh some other caller is making
    ///    finishes, its rotation stored, before anything is deleted; none
    ///    starts until the pins are gone. A socket `pair` started while this
    ///    waited is stopped too: `pair` starts it under the same lock.
    /// 3. The socket task is awaited, so nothing it owns outlives the call,
    ///    and the connection it recorded reads as not running.
    /// 4. Under the lock still, the access token is dropped, both keyring
    ///    entries are deleted and the pins cleared.
    /// 5. The status is read back.
    ///
    /// Taking the lock before raising the stop would wait on a refresh the
    /// endpoint may never answer.
    pub async fn disconnect(&self) -> Result<HubStatus, HubPairingError> {
        self.cancel_device_pairing().await;
        let _in_flight = self
            .in_flight
            .try_lock()
            .map_err(|_| HubPairingError::PairingInProgress)?;
        let running = self.stop_socket();
        let _refresh_lock = self.refresher.lock.lock().await;
        let started = self.stop_socket();
        for task in [running, started].into_iter().flatten() {
            task.ended().await;
        }
        self.connection
            .update(|facts| *facts = ConnectionFacts::default());

        let Refresher {
            pins,
            secrets,
            access,
            ..
        } = &self.refresher;
        *access.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let install_id = pins.install_id().map_err(storage)?;
        for secret in [HubSecret::RefreshToken, HubSecret::DeviceKey] {
            secrets
                .delete(&hub_keyring_account(&install_id, secret))
                .map_err(storage)?;
        }
        pins.clear().map_err(storage)?;
        self.status()
    }

    /// The stored key, or a new one written to the keyring before it is
    /// used. A stored key that does not decode is an error, never replaced:
    /// replacing it would orphan whatever pairing names its thumbprint.
    fn device_key(&self, install_id: &str) -> Result<DeviceKey, HubPairingError> {
        if let Some(key) = self.refresher.stored_device_key(install_id)? {
            return Ok(key);
        }
        let key = DeviceKey::generate()?;
        self.refresher
            .secrets
            .set(
                &hub_keyring_account(install_id, HubSecret::DeviceKey),
                &stored_form(&key)?,
            )
            .map_err(storage)?;
        Ok(key)
    }
}

impl Refresher {
    fn attachment_fetcher(&self) -> Result<Option<HubAttachments>, HubPairingError> {
        let Some(paired) = self.pins.load().map_err(storage)? else {
            return Ok(None);
        };
        let install_id = self.pins.install_id().map_err(storage)?;
        let Some(key) = self.stored_device_key(&install_id)? else {
            return Ok(None);
        };
        HubAttachments::new(paired.hub_origin, key, Arc::new(self.clone())).map(Some)
    }

    async fn refresh_access(&self) -> Result<SecretString, RefreshEnd> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or_else(|| not_paired("no refresh endpoint is configured"))?;
        let _refresh_lock = self.lock.lock().await;

        let origin = self
            .pins
            .load()
            .map_err(|e| RefreshEnd::Refused(storage(e)))?
            .ok_or_else(|| not_paired("this desktop is not paired"))?
            .hub_origin;
        let install_id = self
            .pins
            .install_id()
            .map_err(|e| RefreshEnd::Refused(storage(e)))?;
        let refresh_account = hub_keyring_account(&install_id, HubSecret::RefreshToken);
        let refresh = self
            .secrets
            .get(&refresh_account)
            .map_err(|e| RefreshEnd::Refused(storage(e)))?
            .ok_or_else(|| not_paired("no refresh token is stored"))?;
        let device_key = self
            .stored_device_key(&install_id)
            .map_err(RefreshEnd::Refused)?
            .ok_or_else(|| not_paired("no device key is stored"))?;

        let token_url = token_url(&origin);
        let now = now_secs();
        let proof = device_key
            .dpop_proof(
                &token_url,
                now,
                &random_token().map_err(RefreshEnd::Refused)?,
            )
            .map_err(RefreshEnd::Refused)?;
        let answer = endpoint
            .refresh(&token_url, refresh_form_body(&refresh), proof)
            .await?;

        let expires_at = now.saturating_add(answer.expires_in());
        let tokens = answer.into_tokens();
        self.secrets
            .set(&refresh_account, tokens.refresh.expose())
            .map_err(|e| RefreshEnd::Refused(storage(e)))?;
        let access = tokens.access.clone();
        self.hold_access(tokens.access, expires_at);
        Ok(access)
    }

    fn access_token(&self) -> Option<SecretString> {
        let held = self.access.lock().unwrap_or_else(PoisonError::into_inner);
        held.as_ref()
            .filter(|held| now_secs() < held.expires_at)
            .map(|held| held.token.clone())
    }

    fn access_expires_at(&self) -> Option<u64> {
        let held = self.access.lock().unwrap_or_else(PoisonError::into_inner);
        held.as_ref().map(|held| held.expires_at)
    }

    fn hold_access(&self, token: SecretString, expires_at: u64) {
        *self.access.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(HeldAccess { token, expires_at });
    }

    fn stored_device_key(&self, install_id: &str) -> Result<Option<DeviceKey>, HubPairingError> {
        let account = hub_keyring_account(install_id, HubSecret::DeviceKey);
        let Some(stored) = self.secrets.get(&account).map_err(storage)? else {
            return Ok(None);
        };
        let pkcs8 = URL_SAFE_NO_PAD.decode(stored).map_err(|_| {
            HubPairingError::Storage("device key: the stored value is not base64url".into())
        })?;
        DeviceKey::from_pkcs8(&pkcs8).map(Some)
    }
}

/// How a [`DeviceKey`] is spelled in the keyring.
fn stored_form(key: &DeviceKey) -> Result<String, HubPairingError> {
    Ok(URL_SAFE_NO_PAD.encode(key.to_pkcs8()?))
}

/// The name the Hub's consent page shows for this machine. Only the lookup is
/// behind `cfg`; what is acceptable to send is [`consent_machine`]'s.
#[cfg(unix)]
fn host_name() -> Option<String> {
    let mut buf = [0u8; 256];
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

/// Windows sets `COMPUTERNAME` for every process; reading it avoids a
/// `windows-sys` feature for `GetComputerNameExW`.
#[cfg(windows)]
fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(not(any(unix, windows)))]
fn host_name() -> Option<String> {
    None
}
