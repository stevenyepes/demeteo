//! What `/token` answers a pairing with, kept apart by type: the pins are
//! public and go to SQLite, the two tokens are secrets and go only to the
//! keyring. [`PairingAnswer`] is a `Deserialize` mirror of demeteo-hub's
//! `PairingResponse` (`crates/demeteo-hub/src/pairing/grant.rs`), which is
//! `Serialize`-only and in a crate the core must not depend on.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PinnedPasskey {
    pub credential_id: String,
    pub public_key: String,
}

/// The public half of a pairing. Nothing here is a credential, which is why
/// it alone may be `Serialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HubPins {
    pub hub_origin: String,
    pub instance_id: String,
    pub request_signing_key: String,
    pub rp_id: String,
    pub scopes: Vec<String>,
    pub device_jkt: String,
    /// Unix seconds.
    pub paired_at: i64,
    pub passkeys: Vec<PinnedPasskey>,
}

/// A token that must reach neither a log, an IPC payload nor SQLite: no
/// `Display`, no `Serialize`, and a `Debug` that prints `<redacted>`, so the
/// only way to its text is the deliberate [`SecretString::expose`].
pub struct SecretString(String);

impl SecretString {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[derive(Debug)]
pub struct IssuedTokens {
    pub refresh: SecretString,
    pub access: SecretString,
}

/// Fields are private so that [`PairingAnswer::split`] is the only way out,
/// and no `Debug` because it holds both tokens.
#[derive(Deserialize)]
pub struct PairingAnswer {
    access_token: String,
    /// Read by nothing yet — no Hub route accepts an access token — but
    /// declared so an answer without them is malformed rather than accepted.
    #[allow(dead_code)]
    token_type: String,
    #[allow(dead_code)]
    expires_in: u64,
    refresh_token: String,
    scope: String,
    instance_id: String,
    request_signing_key: String,
    rp_id: String,
    passkeys: Vec<AnswerPasskey>,
}

#[derive(Deserialize)]
struct AnswerPasskey {
    credential_id: String,
    public_key: String,
}

impl PairingAnswer {
    /// `origin` is [`super::hub_origin`]'s, `jkt` the device key's
    /// thumbprint, `now` unix seconds. The Hub joins its scopes with single
    /// spaces.
    pub fn split(self, origin: &str, jkt: &str, now: i64) -> (HubPins, IssuedTokens) {
        let pins = HubPins {
            hub_origin: origin.to_owned(),
            instance_id: self.instance_id,
            request_signing_key: self.request_signing_key,
            rp_id: self.rp_id,
            scopes: self.scope.split(' ').map(str::to_owned).collect(),
            device_jkt: jkt.to_owned(),
            paired_at: now,
            passkeys: self
                .passkeys
                .into_iter()
                .map(|p| PinnedPasskey {
                    credential_id: p.credential_id,
                    public_key: p.public_key,
                })
                .collect(),
        };
        let tokens = IssuedTokens {
            refresh: SecretString(self.refresh_token),
            access: SecretString(self.access_token),
        };
        (pins, tokens)
    }
}

/// The `application/x-www-form-urlencoded` body of the code redemption.
/// Encoded by hand because reqwest's `form` feature is not enabled.
pub fn token_form_body(code: &str, verifier: &str, redirect_uri: &str) -> String {
    format!(
        "grant_type=authorization_code&code={}&code_verifier={}&redirect_uri={}",
        percent_encode(code),
        percent_encode(verifier),
        percent_encode(redirect_uri)
    )
}

/// Every byte outside RFC 3986's unreserved set as `%XX`.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
