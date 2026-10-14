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
//! - The access token is dropped: no Hub route accepts one yet.
//!
//! Nothing here formats a token, the code, the verifier or the PKCS#8 into
//! an error or a `tracing` field.

use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use crate::domain::hub_pairing::answer::token_form_body;
use crate::domain::hub_pairing::keyring_names::hub_keyring_account;
use crate::domain::hub_pairing::pkce::s256_challenge;
use crate::domain::hub_pairing::{
    authorize_url, check_pairing_origin, consent_label, hub_origin, hub_status, token_url,
    AuthorizeRequest, HubPairingError, HubSecret, HubStatus, MAX_INSTANCE_NAME_CHARS,
    MAX_PLATFORM_LABEL_CHARS, SCOPES,
};
use crate::error::AppError;
use crate::ports::hub::{BrowserLauncher, HubPinRepository, HubSecretStore, HubTokenEndpoint};
use crate::shared::time::now_secs;
use crate::state::AppVersion;

use self::device_key::DeviceKey;
use self::loopback::CallbackListener;

pub mod device_key;
#[cfg(feature = "keyring")]
pub mod keyring_store;
pub mod loopback;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod token_client;

pub struct HubPairing {
    pins: Arc<dyn HubPinRepository>,
    secrets: Arc<dyn HubSecretStore>,
    token: Arc<dyn HubTokenEndpoint>,
    app_version: AppVersion,
    /// Held for a whole `pair`. A tokio mutex because the guard lives across
    /// the browser wait, and `try_lock` because a second pairing is refused,
    /// not queued: it would race the first for the same keyring entries.
    in_flight: tokio::sync::Mutex<()>,
}

fn storage(e: AppError) -> HubPairingError {
    HubPairingError::Storage(e.to_string())
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
            pins,
            secrets,
            token,
            app_version: AppVersion::default(),
            in_flight: tokio::sync::Mutex::new(()),
        }
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
        let paired = self.pins.load().map_err(storage)?;
        check_pairing_origin(paired.as_ref().map(|p| p.hub_origin.as_str()), &origin)?;

        if !self.secrets.status().available {
            return Err(HubPairingError::KeyringUnavailable);
        }

        let install_id = self.pins.install_id().map_err(storage)?;
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

        let (pins, tokens) = answer.split(&origin, &jkt, i64::try_from(now).unwrap_or(i64::MAX));
        let refresh_account = hub_keyring_account(&install_id, HubSecret::RefreshToken);
        self.secrets
            .set(&refresh_account, tokens.refresh.expose())
            .map_err(storage)?;
        if let Err(save) = self.pins.save(&pins) {
            if let Err(e) = self.secrets.delete(&refresh_account) {
                tracing::warn!(error = %e, "could not remove the Hub refresh token after its pins failed to save");
            }
            return Err(storage(save));
        }

        self.status()
    }

    /// From local storage only; no network call.
    pub fn status(&self) -> Result<HubStatus, HubPairingError> {
        let pins = self.pins.load().map_err(storage)?;
        let keyring = self.secrets.status();
        let (has_device_key, has_refresh_token) = if keyring.available {
            let install_id = self.pins.install_id().map_err(storage)?;
            let present = |secret| {
                self.secrets
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
            pins.as_ref(),
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
    pub fn disconnect(&self) -> Result<HubStatus, HubPairingError> {
        let install_id = self.pins.install_id().map_err(storage)?;
        for secret in [HubSecret::RefreshToken, HubSecret::DeviceKey] {
            self.secrets
                .delete(&hub_keyring_account(&install_id, secret))
                .map_err(storage)?;
        }
        self.pins.clear().map_err(storage)?;
        self.status()
    }

    /// The stored key, or a new one written to the keyring before it is
    /// used. A stored key that does not decode is an error, never replaced:
    /// replacing it would orphan whatever pairing names its thumbprint.
    fn device_key(&self, install_id: &str) -> Result<DeviceKey, HubPairingError> {
        let account = hub_keyring_account(install_id, HubSecret::DeviceKey);
        if let Some(stored) = self.secrets.get(&account).map_err(storage)? {
            let pkcs8 = URL_SAFE_NO_PAD.decode(stored).map_err(|_| {
                HubPairingError::Storage("device key: the stored value is not base64url".into())
            })?;
            return DeviceKey::from_pkcs8(&pkcs8);
        }
        let key = DeviceKey::generate()?;
        self.secrets
            .set(&account, &URL_SAFE_NO_PAD.encode(key.to_pkcs8()?))
            .map_err(storage)?;
        Ok(key)
    }
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
