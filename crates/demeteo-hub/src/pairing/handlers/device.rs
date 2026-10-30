//! The runner's two requests of a device-code pairing:
//! `POST /device_authorization`, which starts one, and the device-code grant
//! of `POST /token`, which polls it.
//!
//! Both are held to a DPoP proof, as the token route is, and for its reasons
//! sit outside `/v1`. The proof is what names the key: there is no `dpop_jkt`
//! to send, so a pairing is only ever started for a key its starter holds,
//! and only that key's proof can poll it.

use super::token::{issued, spent_proof_for, DevicePoll, FormBody, Granted, TOKEN_PATH};
use crate::auth::handlers::{unix_now, ApiError};
use crate::pairing::authorize::{validate_device, DeviceAuthorizationParams};
use crate::pairing::device::{Poll, StartedDevice};
use crate::pairing::grant::{AuthorizationCode, PairingResponse};
use crate::pairing::pending::ParkError;
use crate::pairing::{DEVICE_CODE_TTL_SECS, DEVICE_POLL_INTERVAL_SECS};
use crate::routes::AppState;
use crate::store::instances::{InstanceKind, NewPairing, Paired};
use crate::store::{StoreError, UserId};
use axum::extract::State;
use axum::http::header::CACHE_CONTROL;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::future::Future;

/// A proof is made for the URL its request is sent to, so this is also the
/// path the route has to be mounted at.
pub const DEVICE_AUTHORIZATION_PATH: &str = "/device_authorization";

/// hub-web's route for the device approval page.
const VERIFICATION_PAGE: &str = "/#/device";

/// What a runner is told to print and to poll with. Deliberately not
/// `Debug`: `device_code` is the secret half.
#[derive(Serialize)]
pub struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}

fn device_not_started(error: ParkError) -> ApiError {
    match error {
        ParkError::Full(_) => ApiError::TOO_MANY_PAIRINGS,
        ParkError::RandomSource(_) => {
            tracing::error!(%error, "a device pairing could not be started");
            ApiError::INTERNAL
        }
    }
}

/// Everything `POST /device_authorization` does past its extractors. The
/// proof is verified and spent before the form is judged, as it is on
/// `/token`: a proof is used up by being presented.
pub fn started_device(
    state: &AppState,
    form: &DeviceAuthorizationParams,
    proof_header: Option<&str>,
    now: u64,
) -> Result<DeviceAuthorization, ApiError> {
    let proof = spent_proof_for(state, DEVICE_AUTHORIZATION_PATH, proof_header, now)?;
    let ask = validate_device(form, &proof.jkt).map_err(|_| ApiError::INVALID_REQUEST)?;
    let StartedDevice {
        device_code,
        user_code,
        expires_at: _,
    } = state
        .pairings()
        .devices()
        .start(ask, now)
        .map_err(device_not_started)?;
    let verification_uri = format!("{}{VERIFICATION_PAGE}", state.rp().origin);
    let user_code = user_code.shown();
    Ok(DeviceAuthorization {
        device_code: device_code.text(),
        verification_uri_complete: format!("{verification_uri}/{user_code}"),
        user_code,
        verification_uri,
        expires_in: DEVICE_CODE_TTL_SECS,
        interval: DEVICE_POLL_INTERVAL_SECS,
    })
}

/// `POST /device_authorization`. Every answer is `no-store`, for the reason
/// [`token`](super::token) gives.
pub async fn device_authorization(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<FormBody<DeviceAuthorizationParams>, ApiError>,
) -> Response {
    let answer = form.and_then(|FormBody(form)| {
        started_device(&state, &form, super::one_proof(&headers), unix_now()).map(Json)
    });
    ([(CACHE_CONTROL, "no-store")], answer).into_response()
}

/// The device-code grant past its extractors. `pair` is as
/// [`redeemed`](super::redeemed) takes it.
///
/// Every answer but the approval is a refusal under its RFC 8628 §3.5 code,
/// and they are told apart on purpose, unlike a code's: the runner holds the
/// device code and its key both, and polling is how it learns which. A text
/// that is no device code is an unknown one, and a proof by any key but the
/// one that started the pairing is told the same.
pub async fn polled<P, F>(
    state: &AppState,
    form: DevicePoll,
    proof_header: Option<&str>,
    now: u64,
    pair: P,
) -> Result<PairingResponse, ApiError>
where
    P: FnOnce(UserId, NewPairing) -> F,
    F: Future<Output = Result<Paired, StoreError>>,
{
    let proof = spent_proof_for(state, TOKEN_PATH, proof_header, now)?;
    let device = AuthorizationCode::parse(&form.device_code)
        .ok_or(ApiError::INVALID_GRANT)?
        .hash();
    let approved = match state.pairings().devices().poll(&device, &proof.jkt, now) {
        Poll::Approved(approved) => approved,
        Poll::Pending => return Err(ApiError::AUTHORIZATION_PENDING),
        Poll::SlowDown => return Err(ApiError::SLOW_DOWN),
        Poll::Denied => return Err(ApiError::ACCESS_DENIED),
        Poll::Expired => return Err(ApiError::EXPIRED_TOKEN),
        Poll::Unknown => return Err(ApiError::INVALID_GRANT),
    };
    let granted = Granted {
        user: approved.user,
        ask: &approved.ask,
        scopes: &approved.scopes,
        kind: InstanceKind::Runner,
    };
    issued(state, &granted, &proof, now, pair).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn a_full_device_store_is_503_and_a_failed_random_source_is_500() {
        use crate::auth::challenge::RandomSourceFailed;
        use crate::pairing::pending::TooManyPending;
        assert_eq!(
            device_not_started(ParkError::Full(TooManyPending)),
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_pairings")
        );
        assert_eq!(
            device_not_started(ParkError::RandomSource(RandomSourceFailed)),
            ApiError::INTERNAL
        );
    }
}
