//! The device approval routes: a signed-in user types the code a runner
//! printed, is shown what it claims and its fingerprint, and approves with a
//! passkey assertion or denies.
//!
//! The session is not the approval here, as it is on the desktop's consent
//! routes. A desktop's code can only be redeemed through the loopback of the
//! machine that asked, which ties the session's answer to that machine; a
//! device code has no such anchor, so approving takes an assertion over a
//! challenge bound to the one pairing, its key and the scopes approved, and
//! the Hub verifies it against the tenant's trusted passkeys before it
//! records anything. Denying binds nothing to anyone and takes the session
//! alone.
//!
//! Every route that names a pairing by `user_code` looks it up the same way,
//! and so every one of them counts a miss
//! ([`DeviceCodes::lookup`](crate::pairing::device::DeviceCodes::lookup)).
//! Each is its extractors over a synchronous body that takes the time, and
//! the tenant's passkeys where it needs them, as arguments.

use super::consent::narrows;
use crate::auth::assertion;
use crate::auth::ceremonies::Ceremony;
use crate::auth::challenge::{base64url_encode, device_approval_challenge, ChallengeBinding};
use crate::auth::handlers::{
    assertion_options, ceremony_not_held, store_failed, unix_now, ApiError, CurrentSession,
    JsonBody,
};
use crate::auth::policy::Status;
use crate::pairing::device::{DeviceAnswer, DevicePrompt, LookupRefusal, UserCode};
use crate::pairing::jwk::fingerprint;
use crate::routes::AppState;
use crate::store::passkeys::{self, Passkey};
use crate::store::UserId;
use axum::extract::State;
use axum::Json;
use demeteo_hub_protocol::{Scope, WebAuthnAssertion};
use serde::{Deserialize, Serialize};

/// `user_code` as the human typed it: [`UserCode::parse`] says what it
/// accepts.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceLookup {
    user_code: String,
}

/// `scopes` is what the user left checked, and is what the passkey signs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceChallengeRequest {
    user_code: String,
    scopes: Vec<Scope>,
}

/// It names no pairing and no scope: what is approved is what the held
/// challenge was computed over.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceApproval {
    ceremony_id: String,
    assertion: WebAuthnAssertion,
}

/// What the approval page shows. As on the desktop's, the four labels are
/// the runner's own words and `fingerprint` is rendered here, never by the
/// page.
#[derive(Serialize)]
pub struct DevicePromptView {
    user_code: String,
    fingerprint: String,
    instance_name: String,
    os: String,
    arch: String,
    app_version: String,
    scopes: Vec<Scope>,
    expires_at: u64,
}

#[derive(Serialize)]
pub struct DeviceChallenge {
    ceremony_id: String,
    options: serde_json::Value,
}

#[derive(Serialize)]
pub struct DeviceAnswered {
    answer: &'static str,
}

fn lookup_refused(refusal: LookupRefusal) -> ApiError {
    match refusal {
        LookupRefusal::Unknown => ApiError::PAIRING_EXPIRED,
        LookupRefusal::TooManyMisses => ApiError::TOO_MANY_LOOKUPS,
    }
}

/// A text that cannot be a code is refused without being counted: it could
/// not have found anyone's pairing.
fn prompt_of(
    state: &AppState,
    user: UserId,
    user_code: &str,
    now: u64,
) -> Result<DevicePrompt, ApiError> {
    let user_code = UserCode::parse(user_code).ok_or(ApiError::INVALID_REQUEST)?;
    state
        .pairings()
        .devices()
        .lookup(user, &user_code, now)
        .map_err(lookup_refused)
}

fn trusted(passkeys: &[Passkey]) -> impl Iterator<Item = &Passkey> {
    passkeys
        .iter()
        .filter(|passkey| passkey.status == Status::Trusted)
}

pub fn looked_up_device(
    state: &AppState,
    user: UserId,
    lookup: DeviceLookup,
    now: u64,
) -> Result<DevicePromptView, ApiError> {
    let prompt = prompt_of(state, user, &lookup.user_code, now)?;
    let ask = prompt.ask;
    Ok(DevicePromptView {
        user_code: prompt.user_code.shown(),
        fingerprint: fingerprint(&ask.dpop_jkt),
        instance_name: ask.instance_name,
        os: ask.os,
        arch: ask.arch,
        app_version: ask.app_version,
        scopes: ask.scopes,
        expires_at: prompt.expires_at,
    })
}

/// Holds a challenge over the pairing `request.user_code` names, its key and
/// the scopes chosen, under the session's passkey `holder`. Any of the
/// tenant's trusted passkeys may answer it. The scopes only narrow, as on the
/// desktop's consent.
pub fn device_challenge(
    state: &AppState,
    user: UserId,
    holder: Vec<u8>,
    request: DeviceChallengeRequest,
    passkeys: &[Passkey],
    now: u64,
) -> Result<DeviceChallenge, ApiError> {
    let prompt = prompt_of(state, user, &request.user_code, now)?;
    if !narrows(&prompt.ask.scopes, &request.scopes) {
        return Err(ApiError::INVALID_SCOPE);
    }
    let binding = ChallengeBinding::mint(now).map_err(|failed| ceremony_not_held(failed.into()))?;
    let challenge = device_approval_challenge(&binding, &prompt.ask.dpop_jkt, &request.scopes)
        .map_err(|_| ApiError::INVALID_REQUEST)?;
    let allowed: Vec<String> = trusted(passkeys)
        .map(|passkey| base64url_encode(&passkey.credential_id))
        .collect();
    let options = assertion_options(state.rp(), &challenge, allowed.iter().map(String::as_str));
    let ceremony = Ceremony::Device {
        user,
        holder,
        device: prompt.device,
        dpop_jkt: prompt.ask.dpop_jkt,
        scopes: request.scopes,
        challenge,
    };
    let ceremony_id = state
        .ceremonies()
        .insert(now, ceremony)
        .map_err(ceremony_not_held)?;
    Ok(DeviceChallenge {
        ceremony_id,
        options,
    })
}

/// Records the approval the held challenge describes, once its assertion
/// verifies under one of `passkeys` that is trusted.
///
/// The challenge is taken before anything is judged, so a refused assertion
/// spends it and the page asks for another. The pairing is left open by
/// every refusal before the last step: a wrong passkey must not be a way to
/// close someone's pairing.
pub fn approved_device(
    state: &AppState,
    user: UserId,
    holder: &[u8],
    approval: DeviceApproval,
    passkeys: &[Passkey],
    now: u64,
) -> Result<DeviceAnswered, ApiError> {
    let taken = state.ceremonies().take(now, &approval.ceremony_id);
    let Some(Ceremony::Device {
        user: started_by,
        holder: held_by,
        device,
        dpop_jkt,
        scopes,
        challenge,
    }) = taken
    else {
        return Err(ApiError::CEREMONY_EXPIRED);
    };
    if started_by != user || held_by != holder {
        return Err(ApiError::CEREMONY_EXPIRED);
    }
    let signer = trusted(passkeys)
        .find(|passkey| {
            base64url_encode(&passkey.credential_id) == approval.assertion.credential_id
        })
        .ok_or(ApiError::ASSERTION_INVALID)?;
    assertion::verify(
        state.rp(),
        &signer.public_key,
        &challenge,
        &approval.assertion,
    )
    .map_err(|_| ApiError::ASSERTION_INVALID)?;
    let answer = DeviceAnswer::Approved { user, scopes };
    if !state
        .pairings()
        .devices()
        .answer(&device, &dpop_jkt, answer, now)
    {
        return Err(ApiError::PAIRING_EXPIRED);
    }
    Ok(DeviceAnswered { answer: "approved" })
}

/// The runner is told `access_denied` at its next poll.
pub fn denied_device(
    state: &AppState,
    user: UserId,
    denial: DeviceLookup,
    now: u64,
) -> Result<DeviceAnswered, ApiError> {
    let prompt = prompt_of(state, user, &denial.user_code, now)?;
    let devices = state.pairings().devices();
    if !devices.answer(
        &prompt.device,
        &prompt.ask.dpop_jkt,
        DeviceAnswer::Denied,
        now,
    ) {
        return Err(ApiError::PAIRING_EXPIRED);
    }
    Ok(DeviceAnswered { answer: "denied" })
}

pub async fn lookup_device(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(lookup): JsonBody<DeviceLookup>,
) -> Result<Json<DevicePromptView>, ApiError> {
    looked_up_device(&state, current.user, lookup, unix_now()).map(Json)
}

pub async fn challenge_device(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(request): JsonBody<DeviceChallengeRequest>,
) -> Result<Json<DeviceChallenge>, ApiError> {
    let passkeys = passkeys::list_passkeys(current.user, state.db())
        .await
        .map_err(store_failed)?;
    let (user, holder) = (current.user, current.credential_id);
    device_challenge(&state, user, holder, request, &passkeys, unix_now()).map(Json)
}

pub async fn approve_device(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(approval): JsonBody<DeviceApproval>,
) -> Result<Json<DeviceAnswered>, ApiError> {
    let passkeys = passkeys::list_passkeys(current.user, state.db())
        .await
        .map_err(store_failed)?;
    let (user, holder) = (current.user, &current.credential_id);
    approved_device(&state, user, holder, approval, &passkeys, unix_now()).map(Json)
}

pub async fn deny_device(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(denial): JsonBody<DeviceLookup>,
) -> Result<Json<DeviceAnswered>, ApiError> {
    denied_device(&state, current.user, denial, unix_now()).map(Json)
}

/// The constructors a caller outside this crate needs to drive the bodies
/// above without HTTP.
impl DeviceLookup {
    pub fn new(user_code: &str) -> Self {
        Self {
            user_code: user_code.to_owned(),
        }
    }
}

impl DeviceChallengeRequest {
    pub fn new(user_code: &str, scopes: Vec<Scope>) -> Self {
        Self {
            user_code: user_code.to_owned(),
            scopes,
        }
    }
}

impl DeviceApproval {
    pub fn new(ceremony_id: &str, assertion: WebAuthnAssertion) -> Self {
        Self {
            ceremony_id: ceremony_id.to_owned(),
            assertion,
        }
    }
}

impl DeviceChallenge {
    pub fn ceremony_id(&self) -> &str {
        &self.ceremony_id
    }

    /// The challenge the options ask a passkey to sign, decoded. `None` when
    /// the options hold none, which no answer of [`device_challenge`] does.
    pub fn challenge(&self) -> Option<[u8; 32]> {
        let text = self.options["publicKey"]["challenge"].as_str()?;
        crate::auth::challenge::base64url_decode(text)?
            .try_into()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn a_miss_is_pairing_expired_and_the_bound_is_429() {
        assert_eq!(
            lookup_refused(LookupRefusal::Unknown),
            ApiError::PAIRING_EXPIRED
        );
        assert_eq!(
            lookup_refused(LookupRefusal::TooManyMisses),
            ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_lookups")
        );
    }

    #[test]
    fn an_approval_body_names_a_ceremony_and_an_assertion_and_nothing_else() {
        let assertion = serde_json::json!({
            "credential_id": "a", "authenticator_data": "b",
            "client_data_json": "c", "signature": "d",
        });
        let parses =
            |body: serde_json::Value| serde_json::from_value::<DeviceApproval>(body).is_ok();
        assert!(parses(
            serde_json::json!({ "ceremony_id": "x", "assertion": assertion })
        ));
        assert!(!parses(
            serde_json::json!({ "ceremony_id": "x", "assertion": assertion, "scopes": ["read"] })
        ));
        assert!(!parses(serde_json::json!({ "ceremony_id": "x" })));
    }
}
