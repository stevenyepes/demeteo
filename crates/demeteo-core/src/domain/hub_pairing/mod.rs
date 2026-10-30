//! Desktop pairing with the Demeteo Hub — the policy half (docs/HUB.md §4).
//!
//! See [`crate::domain`] for why nothing here is `async`. The Hub's half
//! lives in `crates/demeteo-hub/src/pairing/`, which this crate must not
//! depend on, so the few rules the two sides have to agree on byte for byte
//! — the origin spelling, the authorize parameters, the loopback redirect —
//! are mirrored here and table-tested in `src-tauri/tests/infrastructure/hub.rs`.
//!
//! [`hub_origin`] is the single source of the origin: `/authorize`, `/token`,
//! `/socket` and the DPoP `htu` are all derived from it. A trailing slash, an
//! uppercase host or an explicit `:443` that reached any one of them would
//! make every proof fail the Hub's exact `htu` comparison.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::Url;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub mod answer;
pub mod callback;
pub mod device;
pub mod dpop;
pub mod jwk;
pub mod keyring_names;
pub mod pkce;
pub mod sealed;

pub use answer::{
    HubPins, IssuedTokens, PairingAnswer, PinnedPasskey, RefreshAnswer, SecretString,
};

/// The scopes a desktop requests: every [`demeteo_hub_protocol::Scope`], in its
/// wire spelling, space-separated as the OAuth `scope` parameter carries them.
pub const SCOPES: &str = "read spend gates";

/// The Hub's consent TTL (300 s) plus its authorization-code TTL (60 s),
/// plus a minute for the browser round trip. Past this the Hub can no
/// longer issue a code the desktop could redeem, so waiting longer only
/// holds the loopback port open.
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(300 + 60 + 60);

/// The Hub's `MAX_REDIRECT_URI_LEN` in `pairing/authorize.rs`.
const MAX_REDIRECT_URI_LEN: usize = 255;

/// The Hub's `MAX_INSTANCE_NAME_CHARS` and `MAX_PLATFORM_LABEL_CHARS` in
/// `pairing/authorize.rs`.
pub const MAX_INSTANCE_NAME_CHARS: usize = 64;
pub const MAX_PLATFORM_LABEL_CHARS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubSecret {
    DeviceKey,
    RefreshToken,
}

/// Whether this install is paired, judged from local storage alone — the
/// pins and both keyring halves, see [`pairing_state`] — with no call to the
/// Hub. Not socket liveness: a socket that has stopped, after a refused
/// refresh for instance, still reads `Paired`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HubPairingState {
    Unpaired,
    Paired,
    Incomplete,
}

/// Pairing state from what storage holds: the pins in SQLite, and the two
/// keyring entries.
///
/// A device key on its own is still `Unpaired`: it is kept across a failed
/// or abandoned pair so the next attempt reuses it, and that leaves nothing
/// for the user to repair.
pub fn pairing_state(
    has_pins: bool,
    has_device_key: bool,
    has_refresh_token: bool,
) -> HubPairingState {
    match (has_pins, has_device_key, has_refresh_token) {
        (true, true, true) => HubPairingState::Paired,
        (false, _, false) => HubPairingState::Unpaired,
        _ => HubPairingState::Incomplete,
    }
}

/// Every message here reaches the user through an IPC error string, so no
/// variant carries a token, an authorization code, a PKCE verifier or a
/// response body — `HubRefused` holds only the Hub's OAuth `error` code.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HubPairingError {
    #[error("invalid Hub URL: {0}")]
    InvalidHubUrl(String),
    #[error("no OS keyring is available to hold the Hub credentials")]
    KeyringUnavailable,
    /// A store that can say why it is unavailable, as a sealed one can.
    #[error("the {backend} store cannot hold the Hub credentials: {reason}")]
    SecretStoreUnavailable { backend: String, reason: String },
    #[error("a Hub pairing is already in progress")]
    PairingInProgress,
    #[error("already paired with {origin}; disconnect before pairing with another Hub")]
    AlreadyPairedElsewhere { origin: String },
    #[error("the Hub approval was not completed in time")]
    CallbackTimeout,
    #[error("the pairing was denied on the Hub")]
    Denied,
    #[error("the pairing code expired before it was approved on the Hub")]
    DeviceCodeExpired,
    #[error("the Hub callback was refused")]
    CallbackRefused,
    #[error("the Hub refused the pairing: {error}")]
    HubRefused { error: String },
    #[error("the Hub rejected this device's proof; check that the system clock is correct")]
    ClockSkew,
    #[error("could not reach the Hub: {0}")]
    Transport(String),
    #[error("the Hub sent an answer this app cannot read: {0}")]
    MalformedAnswer(String),
    #[error("could not store the Hub pairing: {0}")]
    Storage(String),
}

/// What a refused `/token` means. The Hub answers every failure with a
/// literal `{"error": code}` (`ApiError` in `crates/demeteo-hub/src/auth/handlers/mod.rs`),
/// and a proof outside its `iat` window is refused as `invalid_dpop_proof`
/// — the one refusal the user can act on, by fixing the clock.
pub fn token_error(code: &str) -> HubPairingError {
    match code {
        "invalid_dpop_proof" => HubPairingError::ClockSkew,
        _ => HubPairingError::HubRefused {
            error: code.to_owned(),
        },
    }
}

/// The `{"error": code}` of a Hub refusal body, `/token`'s or `/socket`'s.
/// Anything but a code in the Hub's own spelling is `None`, so a body from a
/// proxy or a hostile server never reaches the user's error text.
pub fn hub_error_code(body: &[u8]) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct ErrorBody {
        error: String,
    }
    let is_hub_code = |code: &str| {
        !code.is_empty()
            && code.len() <= 64
            && code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error)
        .filter(|code| is_hub_code(code))
}

/// [`token_error`] of a non-2xx `/token` body; one without a
/// [`hub_error_code`] is malformed.
pub fn token_refusal(body: &[u8]) -> HubPairingError {
    match hub_error_code(body) {
        Some(error) => token_error(&error),
        None => {
            HubPairingError::MalformedAnswer("the Hub refused /token without an error code".into())
        }
    }
}

/// One install pairs with one Hub. Re-pairing with the same origin is
/// allowed and keeps the device key; another origin needs a disconnect
/// first, so its pins are never silently replaced.
pub fn check_pairing_origin(paired: Option<&str>, origin: &str) -> Result<(), HubPairingError> {
    match paired {
        Some(paired) if paired != origin => Err(HubPairingError::AlreadyPairedElsewhere {
            origin: paired.to_owned(),
        }),
        _ => Ok(()),
    }
}

/// A label the Hub's `label` rule accepts as sent: no control or unseen
/// character, trimmed, 1 to `max_chars` characters. The Hub refuses the
/// whole `/authorize` over one bad label, and the user cannot correct a
/// hostname from the browser, so this repairs rather than refuses, and
/// falls back when nothing is left.
pub fn consent_label(raw: &str, max_chars: usize, fallback: &str) -> String {
    let unseen = |c: char| {
        matches!(
            c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
        )
    };
    let kept: String = raw
        .chars()
        .filter(|&c| !c.is_control() && !unseen(c))
        .collect();
    let label: String = kept.trim().chars().take(max_chars).collect();
    match label.trim_end() {
        "" => fallback.to_owned(),
        label => label.to_owned(),
    }
}

/// This machine as the Hub's consent page names it — the four labels of
/// [`AuthorizeRequest`] that describe the desktop rather than the request.
/// Recomputed on every call rather than stored at pairing, so a renamed host
/// reads here under its new name while the Hub still shows the old one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HubMachine {
    pub instance_name: String,
    pub os: String,
    pub arch: String,
    pub app_version: String,
}

/// The single spelling of [`HubMachine`], which pairing sends and settings
/// shows, so the two cannot drift. An unreadable host name arrives as `""`.
pub fn consent_machine(host_name: &str, os: &str, arch: &str, app_version: &str) -> HubMachine {
    HubMachine {
        instance_name: consent_label(host_name, MAX_INSTANCE_NAME_CHARS, "Demeteo desktop"),
        os: consent_label(os, MAX_PLATFORM_LABEL_CHARS, "unknown"),
        arch: consent_label(arch, MAX_PLATFORM_LABEL_CHARS, "unknown"),
        app_version: consent_label(app_version, MAX_PLATFORM_LABEL_CHARS, "unknown"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyringStatus {
    pub backend: &'static str,
    pub available: bool,
    /// Why it is unavailable, when the store can tell.
    pub reason: Option<&'static str>,
}

impl KeyringStatus {
    /// What pairing refuses with when a store is `!available`.
    pub fn unavailable_error(&self) -> HubPairingError {
        match self.reason {
            Some(reason) => HubPairingError::SecretStoreUnavailable {
                backend: self.backend.to_owned(),
                reason: reason.to_owned(),
            },
            None => HubPairingError::KeyringUnavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HubStatus {
    pub state: HubPairingState,
    /// The normalized origin from [`hub_origin`].
    pub hub_url: Option<String>,
    pub instance_id: Option<String>,
    /// `SHA256:xxxx xxxx xxxx xxxx`, the Hub's exact spelling, so the user
    /// can compare it with what the Hub shows.
    pub fingerprint: Option<String>,
    pub rp_id: Option<String>,
    pub scopes: Vec<String>,
    pub paired_at: Option<i64>,
    pub keyring_backend: String,
    pub keyring_available: bool,
}

/// What `hub_status` reports from what storage holds. A keyring that is
/// unavailable cannot show either entry, so its pins read as `Incomplete`.
pub fn hub_status(
    pins: Option<&HubPins>,
    keyring: KeyringStatus,
    has_device_key: bool,
    has_refresh_token: bool,
) -> HubStatus {
    HubStatus {
        state: pairing_state(
            pins.is_some(),
            keyring.available && has_device_key,
            keyring.available && has_refresh_token,
        ),
        hub_url: pins.map(|p| p.hub_origin.clone()),
        instance_id: pins.map(|p| p.instance_id.clone()),
        fingerprint: pins.map(|p| jwk::fingerprint(&p.device_jkt)),
        rp_id: pins.map(|p| p.rp_id.clone()),
        scopes: pins.map(|p| p.scopes.clone()).unwrap_or_default(),
        paired_at: pins.map(|p| p.paired_at),
        keyring_backend: keyring.backend.to_owned(),
        keyring_available: keyring.available,
    }
}

/// `scheme://host[:port]` as demeteo-hub's `public_origin` spells it: host
/// lowercased, the default port dropped.
///
/// The scheme rule is the Hub's own startup rule: `https`, or `http` only for
/// `localhost`, the one plain-HTTP origin WebAuthn accepts. A Hub served
/// under a sub-path is refused rather than truncated, since `htu` would then
/// name a URL the Hub never serves.
pub fn hub_origin(hub_url: &str) -> Result<String, HubPairingError> {
    let invalid = |reason: &str| HubPairingError::InvalidHubUrl(reason.to_owned());
    let url = Url::parse(hub_url.trim()).map_err(|e| invalid(&e.to_string()))?;
    let host = url
        .host_str()
        .ok_or_else(|| invalid("the URL has no host"))?;
    match url.scheme() {
        "https" => {}
        "http" if host == "localhost" => {}
        "http" => return Err(invalid("plain http is allowed only for localhost")),
        _ => return Err(invalid("the URL must use https")),
    }
    if url.path() != "/" {
        return Err(invalid(
            "a Hub is served at the root of its origin, not under a path",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("the URL must not carry a query or fragment"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("the URL must not carry credentials"));
    }
    Ok(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// The token endpoint, which is also the DPoP `htu` of every proof to it.
pub fn token_url(origin: &str) -> String {
    format!("{origin}/token")
}

/// The DPoP `htu` of a `GET /socket` proof: the origin's `https` spelling,
/// never the `wss` one [`socket_dial_url`] dials. The Hub builds the `htu`
/// it compares from its configured origin (`spent_proof` in
/// `crates/demeteo-hub/src/socket/handler.rs`), so this is the only spelling
/// a proof can match.
pub fn socket_url(origin: &str) -> String {
    format!("{origin}/socket")
}

/// The URL the socket is dialled at. `origin` is one [`hub_origin`] returned,
/// so `http` here is only ever `localhost`.
pub fn socket_dial_url(origin: &str) -> Result<String, HubPairingError> {
    let (scheme, rest) = if let Some(rest) = origin.strip_prefix("https://") {
        ("wss", rest)
    } else if let Some(rest) = origin.strip_prefix("http://") {
        ("ws", rest)
    } else {
        return Err(HubPairingError::InvalidHubUrl(
            "the Hub origin must use https".into(),
        ));
    };
    Ok(format!("{scheme}://{rest}/socket"))
}

/// A proof's `ath` (RFC 9449 §4.2): unpadded base64url of SHA-256 over the
/// access token's bytes, taken as presented.
pub fn access_token_hash(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

/// The variable parameters of the Hub's `GET /authorize`. The Hub requires
/// every one, and `response_type=code` and `code_challenge_method=S256`
/// besides; [`authorize_url`] adds those two, which have no other legal
/// value. There is no `client_id`: the device key's thumbprint (`dpop_jkt`)
/// identifies the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeRequest {
    pub code_challenge: String,
    pub redirect_uri: String,
    pub dpop_jkt: String,
    pub scope: String,
    pub state: String,
    pub instance_name: String,
    pub os: String,
    pub arch: String,
    pub app_version: String,
}

pub fn authorize_url(origin: &str, request: &AuthorizeRequest) -> Result<String, HubPairingError> {
    let mut url = Url::parse(&format!("{origin}/authorize"))
        .map_err(|e| HubPairingError::InvalidHubUrl(e.to_string()))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &request.code_challenge)
        .append_pair("redirect_uri", &request.redirect_uri)
        .append_pair("dpop_jkt", &request.dpop_jkt)
        .append_pair("scope", &request.scope)
        .append_pair("state", &request.state)
        .append_pair("instance_name", &request.instance_name)
        .append_pair("os", &request.os)
        .append_pair("arch", &request.arch)
        .append_pair("app_version", &request.app_version);
    Ok(url.into())
}

/// `http://127.0.0.1:<port>/<path>`, or `None` where the Hub's
/// `is_loopback_redirect` (`pairing/authorize.rs`) would refuse it: port 0,
/// an empty path, a path byte outside the unreserved set, or a URI over the
/// Hub's length cap. That rule compares bytes, so a URI it refuses here
/// would fail `/authorize` with no way to correct it from the browser.
pub fn redirect_uri(port: u16, path: &str) -> Option<String> {
    let unreserved =
        |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
    if port == 0 || path.is_empty() || !path.bytes().all(unreserved) {
        return None;
    }
    let uri = format!("http://127.0.0.1:{port}/{path}");
    (uri.len() <= MAX_REDIRECT_URI_LEN).then_some(uri)
}
