//! The consent routes: a signed-in user is shown one pairing request and
//! approves or denies it.
//!
//! All three are session-bound, and the session is the whole of the approval:
//! a pairing is behind a passkey sign-in because nothing here is reachable
//! without one. The user a request is parked for, and the one who may answer
//! it, is the session's and never a field of a body.
//!
//! Each route is its extractors over a synchronous body that takes the time
//! as an argument. None of them asks anything of the store, so the bodies are
//! reachable from a test that has no database.

use crate::auth::challenge::RandomSourceFailed;
use crate::auth::handlers::{unix_now, ApiError, CurrentSession, JsonBody};
use crate::pairing::authorize::{self, AuthorizeRequest};
use crate::pairing::grant::AuthorizationCode;
use crate::pairing::jwk::fingerprint;
use crate::pairing::pending::{ApprovedPairing, ParkError};
use crate::routes::AppState;
use crate::store::UserId;
use axum::extract::State;
use axum::Json;
use demeteo_hub_protocol::Scope;
use serde::{Deserialize, Serialize};

/// `request` is the segment `GET /authorize` sent the browser on with.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingRequest {
    request: String,
}

/// `scopes` is what the user left checked. A name that is no scope fails the
/// body, as any other malformed member does.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingApproval {
    pairing_id: String,
    scopes: Vec<Scope>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingDenial {
    pairing_id: String,
}

/// What the approval page shows. The four labels are the caller's own words
/// about itself; `fingerprint` is the one member a user can check against
/// their desktop, and is rendered here so that the page never derives it.
#[derive(Serialize)]
pub struct PairingPrompt {
    pairing_id: String,
    fingerprint: String,
    instance_name: String,
    os: String,
    arch: String,
    app_version: String,
    scopes: Vec<Scope>,
    expires_at: u64,
}

/// Where the browser goes next: the desktop's loopback listener. Deliberately
/// not `Debug`, because after an approval `redirect` holds the authorization
/// code.
#[derive(Serialize)]
pub struct ConsentAnswer {
    redirect: String,
}

fn consent_not_parked(error: ParkError) -> ApiError {
    match error {
        ParkError::Full(_) => ApiError::TOO_MANY_PAIRINGS,
        ParkError::RandomSource(_) => {
            tracing::error!(%error, "a pairing could not be started");
            ApiError::INTERNAL
        }
    }
}

fn code_not_minted(error: RandomSourceFailed) -> ApiError {
    tracing::error!(%error, "an authorization code could not be minted");
    ApiError::INTERNAL
}

/// Whether `chosen` is something a user can have left checked on a screen
/// that offered `requested`: at least one scope, each of them offered, none
/// twice. An approval only narrows. A scope the desktop did not ask for is
/// one the user was never shown, and granting nothing is a denial, which has
/// a route of its own.
pub fn narrows(requested: &[Scope], chosen: &[Scope]) -> bool {
    let offered = chosen.iter().all(|scope| requested.contains(scope));
    let repeated = (1..chosen.len()).any(|place| chosen[..place].contains(&chosen[place]));
    !chosen.is_empty() && offered && !repeated
}

/// `redirect_uri` holds no query and no fragment, and `state` and a code are
/// unreserved characters only, all by the rules of
/// [`validate`](authorize::validate): the two are joined with no encoder.
fn redirect_with(request: &AuthorizeRequest, outcome: &str) -> ConsentAnswer {
    let (redirect_uri, state) = (&request.redirect_uri, &request.state);
    ConsentAnswer {
        redirect: format!("{redirect_uri}?{outcome}&state={state}"),
    }
}

/// The segment is validated again here and not trusted for having come from
/// `/authorize`: it reached the page in a URL, so it is whatever the link
/// said.
pub fn started_pairing(
    state: &AppState,
    user: UserId,
    start: PairingRequest,
    now: u64,
) -> Result<PairingPrompt, ApiError> {
    let request = authorize::decode(&start.request).map_err(|_| ApiError::INVALID_REQUEST)?;
    let parked = state
        .pairings()
        .park_consent(user, request.clone(), now)
        .map_err(consent_not_parked)?;
    Ok(PairingPrompt {
        pairing_id: parked.pairing_id,
        fingerprint: fingerprint(&request.dpop_jkt),
        instance_name: request.instance_name,
        os: request.os,
        arch: request.arch,
        app_version: request.app_version,
        scopes: request.scopes,
        expires_at: parked.expires_at,
    })
}

/// The consent is spent before its answer is looked at, so an approval that
/// is refused for its scopes, or that finds the code store full, has used the
/// `pairing_id` up: the next one starts again from the desktop.
pub fn approved_pairing(
    state: &AppState,
    user: UserId,
    approval: PairingApproval,
    now: u64,
) -> Result<ConsentAnswer, ApiError> {
    let pairings = state.pairings();
    let request = pairings
        .take_consent(user, &approval.pairing_id, now)
        .ok_or(ApiError::PAIRING_EXPIRED)?;
    if !narrows(&request.scopes, &approval.scopes) {
        return Err(ApiError::INVALID_SCOPE);
    }
    let code = AuthorizationCode::generate().map_err(code_not_minted)?;
    let answer = redirect_with(&request, &format!("code={}", code.text()));
    let approved = ApprovedPairing {
        user,
        request,
        scopes: approval.scopes,
        approved_at: now,
    };
    pairings
        .park_code(code.hash(), approved, now)
        .map_err(|_| ApiError::TOO_MANY_PAIRINGS)?;
    Ok(answer)
}

/// The desktop is told, in the one place it is listening, so that it stops
/// waiting. RFC 6749 §4.1.2.1 names the error.
pub fn denied_pairing(
    state: &AppState,
    user: UserId,
    denial: PairingDenial,
    now: u64,
) -> Result<ConsentAnswer, ApiError> {
    let request = state
        .pairings()
        .take_consent(user, &denial.pairing_id, now)
        .ok_or(ApiError::PAIRING_EXPIRED)?;
    Ok(redirect_with(&request, "error=access_denied"))
}

pub async fn start_pairing(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(start): JsonBody<PairingRequest>,
) -> Result<Json<PairingPrompt>, ApiError> {
    started_pairing(&state, current.user, start, unix_now()).map(Json)
}

pub async fn approve_pairing(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(approval): JsonBody<PairingApproval>,
) -> Result<Json<ConsentAnswer>, ApiError> {
    approved_pairing(&state, current.user, approval, unix_now()).map(Json)
}

pub async fn deny_pairing(
    State(state): State<AppState>,
    current: CurrentSession,
    JsonBody(denial): JsonBody<PairingDenial>,
) -> Result<Json<ConsentAnswer>, ApiError> {
    denied_pairing(&state, current.user, denial, unix_now()).map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::testing::lazy_state;
    use crate::auth::RelyingParty;
    use crate::pairing::pending::TooManyPending;
    use crate::pairing::MAX_PENDING_PER_USER;
    use axum::http::StatusCode;
    use Scope::{Gates, Read, Spend};

    const NOW: u64 = 1_700_000_000;

    fn request() -> AuthorizeRequest {
        AuthorizeRequest {
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_owned(),
            redirect_uri: "http://127.0.0.1:49152/callback".to_owned(),
            dpop_jkt: "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I".to_owned(),
            scopes: vec![Read, Gates],
            state: "xyz-123".to_owned(),
            instance_name: "Workstation".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            app_version: "1.4.0".to_owned(),
        }
    }

    fn state() -> AppState {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        };
        lazy_state("postgres://hub@db.invalid/hub", rp).unwrap()
    }

    fn start() -> PairingRequest {
        PairingRequest {
            request: authorize::encode(&request()),
        }
    }

    #[test]
    fn an_approval_narrows_only_to_a_non_empty_set_of_what_was_requested() {
        let requested = [Read, Gates];
        for chosen in [&[Read, Gates][..], &[Gates, Read], &[Read], &[Gates]] {
            assert!(narrows(&requested, chosen), "{chosen:?}");
        }
        for chosen in [
            &[][..],
            &[Spend],
            &[Read, Spend],
            &[Read, Read],
            &[Read, Gates, Read],
        ] {
            assert!(!narrows(&requested, chosen), "{chosen:?}");
        }
        assert!(!narrows(&[], &[]));
        assert!(!narrows(&[], &[Read]));
    }

    #[test]
    fn a_full_consent_store_is_503_and_a_failed_random_source_is_500() {
        assert_eq!(
            consent_not_parked(ParkError::Full(TooManyPending)),
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_pairings")
        );
        assert_eq!(
            consent_not_parked(ParkError::RandomSource(RandomSourceFailed)),
            ApiError::INTERNAL
        );
        assert_eq!(code_not_minted(RandomSourceFailed), ApiError::INTERNAL);
    }

    #[tokio::test]
    async fn a_user_with_every_place_taken_is_told_503_and_another_user_is_served() {
        let state = state();
        let (mine, theirs) = (UserId::generate(), UserId::generate());
        for _ in 0..MAX_PENDING_PER_USER {
            assert!(started_pairing(&state, mine, start(), NOW).is_ok());
        }

        let refused = started_pairing(&state, mine, start(), NOW).err();

        assert_eq!(refused, Some(ApiError::TOO_MANY_PAIRINGS));
        assert!(started_pairing(&state, theirs, start(), NOW).is_ok());
    }

    #[tokio::test]
    async fn a_segment_that_is_not_a_valid_request_starts_nothing() {
        let state = state();
        let user = UserId::generate();
        let unscoped = AuthorizeRequest {
            scopes: Vec::new(),
            ..request()
        };
        for segment in [String::new(), "!".to_owned(), authorize::encode(&unscoped)] {
            let start = PairingRequest { request: segment };
            let refused = started_pairing(&state, user, start, NOW).err();
            assert_eq!(refused, Some(ApiError::INVALID_REQUEST));
        }
    }

    #[test]
    fn a_body_with_a_member_nobody_named_or_a_scope_the_hub_does_not_have_is_no_body() {
        let parses = |body: serde_json::Value| serde_json::from_value::<PairingApproval>(body);
        assert!(parses(serde_json::json!({ "pairing_id": "id", "scopes": ["read"] })).is_ok());
        assert!(parses(serde_json::json!({ "pairing_id": "id", "scopes": ["logs"] })).is_err());
        assert!(parses(serde_json::json!({ "pairing_id": "id", "scopes": "read" })).is_err());
        assert!(parses(
            serde_json::json!({ "pairing_id": "id", "scopes": ["read"], "user": "another" })
        )
        .is_err());
    }
}
