//! Pairing by device code (RFC 8628, docs/HUB.md §4 *Pairing a runner by
//! device code*) — the policy half. Device key, DPoP binding and pairing
//! response are the authorization-code grant's; what is new is the two calls
//! that stand in for the browser round trip, and when to poll again.

use serde::{Deserialize, Serialize};

use super::answer::{percent_encode, SecretString};
use super::{hub_error_code, token_error, HubMachine, HubPairingError};

pub const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// What a `slow_down` adds to the interval (RFC 8628 §3.5).
pub const SLOW_DOWN_STEP_SECS: u64 = 5;

/// The Hub's `user_code` is eight characters with one `-`; the cap only
/// keeps a hostile answer from reaching a terminal at length.
const MAX_USER_CODE_CHARS: usize = 32;

/// Also the `htu` of the proof the call carries.
pub fn device_authorization_url(origin: &str) -> String {
    format!("{origin}/device_authorization")
}

/// The form of `POST /device_authorization`: the labels `/authorize` takes,
/// and no `dpop_jkt`, because the key is proven by the proof's `jwk`.
pub fn device_authorization_form_body(scope: &str, machine: &HubMachine) -> String {
    format!(
        "scope={}&instance_name={}&os={}&arch={}&app_version={}",
        percent_encode(scope),
        percent_encode(&machine.instance_name),
        percent_encode(&machine.os),
        percent_encode(&machine.arch),
        percent_encode(&machine.app_version),
    )
}

pub fn device_token_form_body(device_code: &str) -> String {
    format!(
        "grant_type={}&device_code={}",
        percent_encode(DEVICE_CODE_GRANT),
        percent_encode(device_code)
    )
}

/// A `Deserialize` mirror of the Hub's `200` to `/device_authorization`.
/// Private fields and no `Debug`: `device_code` is what redeems the pairing.
#[derive(Deserialize)]
pub struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}

/// What the runner shows the human: the code to type, where, and the
/// fingerprint to compare with the one the Hub shows beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceTicket {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub fingerprint: String,
    /// Unix seconds, by the local clock.
    pub expires_at: u64,
}

pub struct DeviceGrant {
    pub ticket: DeviceTicket,
    pub device_code: SecretString,
    pub interval_secs: u64,
}

impl DeviceAuthorization {
    /// The answer, once everything a human will read from it is something
    /// this Hub could have sent: a short code of letters, digits and `-`,
    /// and verification URIs under `origin` with nothing in them a terminal
    /// would act on. An interval of `0` is read as `1`, so no poll loop spins.
    pub fn accept(
        self,
        origin: &str,
        fingerprint: String,
        now: u64,
    ) -> Result<DeviceGrant, HubPairingError> {
        let malformed = |what: &str| HubPairingError::MalformedAnswer(what.to_owned());
        let code_ok = !self.user_code.is_empty()
            && self.user_code.chars().count() <= MAX_USER_CODE_CHARS
            && self
                .user_code
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !code_ok {
            return Err(malformed("the user code is not one this app can show"));
        }
        let under_origin = |uri: &str| {
            uri.strip_prefix(origin)
                .is_some_and(|rest| rest.starts_with('/'))
                && uri.chars().all(|c| c.is_ascii_graphic())
        };
        if !under_origin(&self.verification_uri) || !under_origin(&self.verification_uri_complete) {
            return Err(malformed("the verification URI is not on the Hub's origin"));
        }
        if self.expires_in == 0 || self.device_code.is_empty() {
            return Err(malformed("the device code is empty or already expired"));
        }
        Ok(DeviceGrant {
            ticket: DeviceTicket {
                user_code: self.user_code,
                verification_uri: self.verification_uri,
                verification_uri_complete: self.verification_uri_complete,
                fingerprint,
                expires_at: now.saturating_add(self.expires_in),
            },
            device_code: SecretString::new(self.device_code),
            interval_secs: self.interval.max(1),
        })
    }
}

/// Why a device-code `/token` poll returned no pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceGrantEnd {
    Pending,
    SlowDown,
    Denied,
    Expired,
    /// No answer that could be read. Only an answer spends the code, so it
    /// may be presented again; if the lost answer was the approval, the next
    /// poll is `invalid_grant`, and that ends the pairing.
    Unanswered,
    Refused(HubPairingError),
}

/// The meaning of a non-2xx device-code poll, by its `error` code.
pub fn device_grant_refusal(body: &[u8]) -> DeviceGrantEnd {
    match hub_error_code(body).as_deref() {
        Some("authorization_pending") => DeviceGrantEnd::Pending,
        Some("slow_down") => DeviceGrantEnd::SlowDown,
        Some("access_denied") => DeviceGrantEnd::Denied,
        Some("expired_token") => DeviceGrantEnd::Expired,
        Some(code) => DeviceGrantEnd::Refused(token_error(code)),
        None => DeviceGrantEnd::Refused(HubPairingError::MalformedAnswer(
            "the Hub refused /token without an error code".into(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePollStep {
    /// Poll again after this many seconds, and keep it as the interval.
    Wait {
        interval_secs: u64,
    },
    Fail(HubPairingError),
}

/// What follows a poll that ended in `end`, at `now`, for a code that
/// expires at `expires_at` (both Unix seconds). Past expiry nothing is
/// polled, whatever the answer, so an unreachable Hub cannot keep the
/// pairing pending forever.
pub fn after_device_poll(
    end: DeviceGrantEnd,
    interval_secs: u64,
    now: u64,
    expires_at: u64,
) -> DevicePollStep {
    let wait = |interval_secs| {
        if now < expires_at {
            DevicePollStep::Wait { interval_secs }
        } else {
            DevicePollStep::Fail(HubPairingError::DeviceCodeExpired)
        }
    };
    match end {
        DeviceGrantEnd::Pending | DeviceGrantEnd::Unanswered => wait(interval_secs),
        DeviceGrantEnd::SlowDown => wait(interval_secs.saturating_add(SLOW_DOWN_STEP_SECS)),
        DeviceGrantEnd::Denied => DevicePollStep::Fail(HubPairingError::Denied),
        DeviceGrantEnd::Expired => DevicePollStep::Fail(HubPairingError::DeviceCodeExpired),
        DeviceGrantEnd::Refused(error) => DevicePollStep::Fail(error),
    }
}

/// A device pairing as it stands. A success leaves `Idle` behind it, and
/// `HubPairing::status` reads `paired`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePairing {
    Idle,
    Pending(DeviceTicket),
    Failed(HubPairingError),
}
