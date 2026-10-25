//! Desktop pairing with the Demeteo Hub — the choreography half. Every
//! decision is [`crate::domain::hub_pairing`]'s; this module only orders the
//! I/O, and the order is the contract (spec §4.3):
//!
//! - The device key reaches the keyring **before** `/authorize`, which binds
//!   the pairing to its thumbprint. A key lost after that point would leave a
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

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use crate::domain::hub_pairing::answer::{refresh_form_body, token_form_body};
use crate::domain::hub_pairing::keyring_names::hub_keyring_account;
use crate::domain::hub_pairing::pkce::s256_challenge;
use crate::domain::hub_pairing::{
    authorize_url, check_pairing_origin, consent_label, hub_origin, hub_status, token_url,
    AuthorizeRequest, HubPairingError, HubSecret, HubStatus, SecretString, MAX_INSTANCE_NAME_CHARS,
    MAX_PLATFORM_LABEL_CHARS, SCOPES,
};
use crate::domain::hub_socket::RefreshEnd;
use crate::error::AppError;
use crate::ports::hub::{
    BrowserLauncher, HubPinRepository, HubRefreshEndpoint, HubSecretStore, HubTokenEndpoint,
};
use crate::shared::time::now_secs;
use crate::state::AppVersion;

use self::device_key::DeviceKey;
use self::loopback::CallbackListener;
use self::socket::session::SessionContext;
use self::socket::{SocketConfig, SocketTask, SupervisorDeps};

pub mod device_key;
#[cfg(feature = "keyring")]
pub mod keyring_store;
pub mod loopback;
pub mod socket;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod token_client;

pub struct HubPairing {
    token: Arc<dyn HubTokenEndpoint>,
    app_version: AppVersion,
    /// Held for a whole `pair`. A tokio mutex because the guard lives across
    /// the browser wait, and `try_lock` because a second pairing is refused,
    /// not queued: it would race the first for the same keyring entries.
    in_flight: tokio::sync::Mutex<()>,
    refresher: Refresher,
    socket_config: Option<SocketConfig>,
    socket: std::sync::Mutex<Option<SocketTask>>,
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
    ) -> Self {
        Self {
            token,
            app_version: AppVersion::default(),
            in_flight: tokio::sync::Mutex::new(()),
            refresher: Refresher {
                pins,
                secrets,
                endpoint: None,
                lock: Arc::default(),
                access: Arc::default(),
            },
            socket_config: None,
            socket: std::sync::Mutex::new(None),
        }
    }

    /// Without one, [`Self::refresh_access`] fails rather than refreshing.
    pub fn with_refresh_endpoint(mut self, endpoint: Arc<dyn HubRefreshEndpoint>) -> Self {
        self.refresher.endpoint = Some(endpoint);
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

    /// The version shown on the Hub's consent page. Unset, the page shows
    /// `unknown` rather than `CARGO_PKG_VERSION` (see [`AppVersion`]).
    pub fn with_app_version(mut self, app_version: AppVersion) -> Self {
        self.app_version = app_version;
        self
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

        let origin = hub_origin(hub_url)?;
        let paired = self.refresher.pins.load().map_err(storage)?;
        check_pairing_origin(paired.as_ref().map(|p| p.hub_origin.as_str()), &origin)?;

        if !self.refresher.secrets.status().available {
            return Err(HubPairingError::KeyringUnavailable);
        }

        let install_id = self.refresher.pins.install_id().map_err(storage)?;
        let device_key = self.device_key(&install_id)?;
        let jkt = device_key.jkt();

        let (listener, redirect) = CallbackListener::bind()?;
        let state = random_token()?;
        let verifier = random_token()?;
        let request = AuthorizeRequest {
            code_challenge: s256_challenge(&verifier),
            redirect_uri: redirect.clone(),
            dpop_jkt: jkt.clone(),
            scope: SCOPES.to_owned(),
            state: state.clone(),
            instance_name: consent_label(
                &host_name().unwrap_or_default(),
                MAX_INSTANCE_NAME_CHARS,
                "Demeteo desktop",
            ),
            os: consent_label(std::env::consts::OS, MAX_PLATFORM_LABEL_CHARS, "unknown"),
            arch: consent_label(std::env::consts::ARCH, MAX_PLATFORM_LABEL_CHARS, "unknown"),
            app_version: consent_label(self.app_version.get(), MAX_PLATFORM_LABEL_CHARS, "unknown"),
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

        let expires_at = now.saturating_add(answer.expires_in());
        let (pins, tokens) = answer.split(&origin, &jkt, i64::try_from(now).unwrap_or(i64::MAX));
        let refresh_account = hub_keyring_account(&install_id, HubSecret::RefreshToken);
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
    /// so a second start spawns no second task; only [`Self::pair`]
    /// replaces it, by stopping it first. One that stopped on a verdict is
    /// started again, since a new pairing is what most of those verdicts
    /// ask for.
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
            ),
            sleeper: config.sleeper.clone(),
            jitter: Box::new(config.jitter),
        }))
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
    /// pins. The Hub has no revoke route, so the instance stays listed there
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
    /// 3. The socket task is awaited, so nothing it owns outlives the call.
    /// 4. Under the lock still, the access token is dropped, both keyring
    ///    entries are deleted and the pins cleared.
    /// 5. The status is read back.
    ///
    /// Taking the lock before raising the stop would wait on a refresh the
    /// endpoint may never answer.
    pub async fn disconnect(&self) -> Result<HubStatus, HubPairingError> {
        let running = self.stop_socket();
        let _refresh_lock = self.refresher.lock.lock().await;
        let started = self.stop_socket();
        for task in [running, started].into_iter().flatten() {
            task.ended().await;
        }

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
/// behind `cfg`; what is acceptable to send is [`consent_label`]'s.
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
