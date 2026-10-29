//! The `/v1` handlers. Each one extracts, calls a pure decision or the store,
//! and maps the outcome to a status; none decides anything itself.
//!
//! A client is told an [`ApiError`] and nothing more: the text of an internal
//! error goes to the log, never into a response. The [`session`] module's
//! rule about the secret and the CSRF token holds here too, so nothing that
//! carries either derives `Debug`.

mod add_passkey;
mod passkeys;
mod register;
mod sign_in;

pub use add_passkey::{finish_addition, start_addition, stored_addition};
pub(crate) use passkeys::public_key_options;
pub use passkeys::{
    finish_endorsement, finish_revocation, list_passkeys, start_endorsement, start_revocation,
};
pub use register::{finish_registration, start_registration};
pub use sign_in::{finish_sign_in, start_sign_in, verified_sign_in};

use super::ceremonies::InsertError;
use super::challenge::base64url_encode;
use super::policy::Refusal;
use super::session::{self, SessionError, SessionSecret, SESSION_TTL_SECS};
use crate::attachments::envelope::AttachmentRefusal;
use crate::ports::KeyServiceError;
use crate::routes::AppState;
use crate::socket::connections::Full;
use crate::store::{sessions, StoreError, UserId};
use axum::extract::{FromRequest, FromRequestParts, Request, State};
use axum::http::header::{AsHeaderName, COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use demeteo_hub_protocol::Scope;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

/// A refusal as the client sees it: `status`, and `{"error": "<code>"}`. The
/// code is always a literal, so there is no path by which the text of the
/// error behind it reaches a response body. The one detail a body may add is
/// the [`Scope`] a refusal is for, under `"scope"`, and only when there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    scope: Option<Scope>,
}

impl ApiError {
    pub const UNAUTHENTICATED: Self = Self::new(StatusCode::UNAUTHORIZED, "unauthenticated");
    pub(crate) const INTERNAL: Self = Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    pub(crate) const INVALID_REQUEST: Self = Self::new(StatusCode::BAD_REQUEST, "invalid_request");
    pub(crate) const CEREMONY_EXPIRED: Self =
        Self::new(StatusCode::BAD_REQUEST, "ceremony_expired");
    const ASSERTION_INVALID: Self = Self::new(StatusCode::FORBIDDEN, "assertion_invalid");
    const TOO_MANY_CEREMONIES: Self =
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_ceremonies");
    const UNSUPPORTED_ALGORITHM: Self =
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, "unsupported_algorithm");
    const CREDENTIAL_EXISTS: Self = Self::new(StatusCode::CONFLICT, "credential_exists");
    pub(crate) const PAIRING_EXPIRED: Self = Self::new(StatusCode::BAD_REQUEST, "pairing_expired");
    pub(crate) const INVALID_SCOPE: Self = Self::new(StatusCode::BAD_REQUEST, "invalid_scope");
    pub(crate) const TOO_MANY_PAIRINGS: Self =
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_pairings");
    pub(crate) const INVALID_DPOP_PROOF: Self =
        Self::new(StatusCode::BAD_REQUEST, "invalid_dpop_proof");
    pub(crate) const INVALID_GRANT: Self = Self::new(StatusCode::BAD_REQUEST, "invalid_grant");
    pub(crate) const UNSUPPORTED_GRANT_TYPE: Self =
        Self::new(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    pub(crate) const KEY_SERVICE_UNAVAILABLE: Self =
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "key_service_unavailable");
    pub(crate) const TOO_MANY_REQUESTS: Self =
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_requests");
    pub(crate) const INVALID_TOKEN: Self = Self::new(StatusCode::UNAUTHORIZED, "invalid_token");
    pub(crate) const TOO_MANY_CONNECTIONS: Self =
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_connections");
    const SCOPE_NOT_GRANTED: Self = Self::new(StatusCode::CONFLICT, "scope_not_granted");
    pub const NOT_FOUND: Self = Self::new(StatusCode::NOT_FOUND, "not_found");
    pub const CHALLENGE_MISMATCH: Self = Self::new(StatusCode::CONFLICT, "challenge_mismatch");
    pub const ASSERTION_MALFORMED: Self = Self::new(StatusCode::BAD_REQUEST, "assertion_malformed");
    pub const ATTACHMENT_TOO_LARGE: Self =
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "attachment_too_large");
    pub const ATTACHMENT_UNAVAILABLE: Self =
        Self::new(StatusCode::CONFLICT, "attachment_unavailable");

    pub(crate) const fn new(status: StatusCode, code: &'static str) -> Self {
        Self {
            status,
            code,
            scope: None,
        }
    }

    pub const fn scope_not_granted(scope: Scope) -> Self {
        Self {
            scope: Some(scope),
            ..Self::SCOPE_NOT_GRANTED
        }
    }
}

impl From<session::Rejection> for ApiError {
    fn from(rejection: session::Rejection) -> Self {
        let code = match rejection {
            session::Rejection::OriginMismatch => "origin_mismatch",
            session::Rejection::CsrfMissing => "csrf_required",
            session::Rejection::CsrfMismatch => "csrf_mismatch",
        };
        Self::new(StatusCode::FORBIDDEN, code)
    }
}

impl From<Refusal> for ApiError {
    fn from(refusal: Refusal) -> Self {
        let (status, code) = match refusal {
            Refusal::UnknownCredential => (StatusCode::NOT_FOUND, "not_found"),
            Refusal::NotPending => (StatusCode::CONFLICT, "not_pending"),
            Refusal::AlreadyRevoked => (StatusCode::CONFLICT, "already_revoked"),
            Refusal::LastPasskey => (StatusCode::CONFLICT, "last_passkey"),
            Refusal::TooManyPasskeys => (StatusCode::CONFLICT, "too_many_passkeys"),
            Refusal::SelfEndorsement => (StatusCode::FORBIDDEN, "self_endorsement"),
            Refusal::EndorserNotTrusted => (StatusCode::FORBIDDEN, "endorser_not_trusted"),
            Refusal::SelfRevocation => (StatusCode::FORBIDDEN, "self_revocation"),
            Refusal::RevokerNotTrusted => (StatusCode::FORBIDDEN, "revoker_not_trusted"),
        };
        Self::new(status, code)
    }
}

impl From<AttachmentRefusal> for ApiError {
    fn from(refusal: AttachmentRefusal) -> Self {
        match refusal {
            AttachmentRefusal::Invalid => Self::INVALID_REQUEST,
            AttachmentRefusal::NotFound => Self::NOT_FOUND,
            AttachmentRefusal::Unavailable => Self::ATTACHMENT_UNAVAILABLE,
        }
    }
}

impl From<Full> for ApiError {
    fn from(full: Full) -> Self {
        match full {
            Full => Self::TOO_MANY_CONNECTIONS,
        }
    }
}

/// A failed attachment data-key call: a service that is sealed or away is
/// one to wait for, and a name or answer it refuses is the Hub's own fault.
impl From<KeyServiceError> for ApiError {
    fn from(error: KeyServiceError) -> Self {
        match error {
            KeyServiceError::Sealed
            | KeyServiceError::UnknownKey(_)
            | KeyServiceError::Transport(_)
            | KeyServiceError::Upstream { .. } => {
                tracing::warn!(%error, "an attachment key could not be had");
                Self::KEY_SERVICE_UNAVAILABLE
            }
            KeyServiceError::InvalidKeyName(_)
            | KeyServiceError::InvalidMount(_)
            | KeyServiceError::Malformed(_) => {
                tracing::error!(%error, "the attachment transit key is misconfigured");
                Self::INTERNAL
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: self.code,
            scope: self.scope,
        };
        (self.status, Json(body)).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<Scope>,
}

pub(crate) fn store_failed(error: StoreError) -> ApiError {
    tracing::error!(%error, "a store call failed");
    ApiError::INTERNAL
}

pub(crate) fn ceremony_not_held(error: InsertError) -> ApiError {
    match error {
        InsertError::Full => ApiError::TOO_MANY_CEREMONIES,
        InsertError::RandomSource(_) => {
            tracing::error!(%error, "a ceremony could not be started");
            ApiError::INTERNAL
        }
    }
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since_epoch| since_epoch.as_secs())
}

/// A JSON body, refused under one code whatever is wrong with it. axum's own
/// rejection answers in prose, under a status that depends on the fault.
pub struct JsonBody<T>(pub T);

impl<T, S> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(body)) => Ok(Self(body)),
            Err(_) => Err(ApiError::INVALID_REQUEST),
        }
    }
}

/// A header's value when it is visible ASCII. Right for `Origin` and the CSRF
/// token, which are each compared for equality with an ASCII string, and
/// wrong for `Cookie`: see [`cookie_text`].
pub(crate) fn header_text(headers: &HeaderMap, name: impl AsHeaderName) -> Option<&str> {
    headers.get(name)?.to_str().ok()
}

/// The `Cookie` header, whatever bytes its other cookies hold. The mutation
/// guard and the session extractor both read through this: were they to
/// disagree about which `Cookie` a request carries, the token would be
/// checked against one session and the request served as another.
///
/// Every cookie of the host arrives in the one header, and a value outside
/// ASCII in any of them is not the Hub's to refuse. The lossy view changes
/// none of what is looked for: the pair delimiters and the Hub's own
/// base64url value are ASCII, which it passes through as it is.
pub(crate) fn cookie_text(headers: &HeaderMap) -> Option<Cow<'_, str>> {
    let cookie = headers.get(COOKIE)?;
    Some(String::from_utf8_lossy(cookie.as_bytes()))
}

fn presented_secret(headers: &HeaderMap) -> Option<SessionSecret> {
    SessionSecret::from_cookie_header(&cookie_text(headers)?)
}

/// The signed-in tenant. Extracting it answers `401` when there is none.
pub struct CurrentSession {
    secret: SessionSecret,
    pub(crate) user: UserId,
    /// The passkey that signed in.
    pub(crate) credential_id: Vec<u8>,
}

impl FromRequestParts<AppState> for CurrentSession {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let secret = presented_secret(&parts.headers).ok_or(ApiError::UNAUTHENTICATED)?;
        let found = sessions::find_session(state.db(), &secret.hash())
            .await
            .map_err(store_failed)?
            .ok_or(ApiError::UNAUTHENTICATED)?;
        Ok(Self {
            secret,
            user: found.user_id,
            credential_id: found.credential_id,
        })
    }
}

#[derive(Serialize)]
pub struct SessionView {
    csrf_token: String,
    credential_id: String,
}

impl SessionView {
    fn of(secret: &SessionSecret, credential_id: &[u8]) -> Self {
        Self {
            csrf_token: secret.csrf_token(),
            credential_id: base64url_encode(credential_id),
        }
    }
}

/// How a page that was reloaded gets its CSRF token back: the cookie is
/// `HttpOnly`, so script cannot derive the token itself.
pub async fn read_session(current: CurrentSession) -> Json<SessionView> {
    Json(SessionView::of(&current.secret, &current.credential_id))
}

/// When the row of a session that starts now stops being one: the moment the
/// `Max-Age` of [`session::set_cookie`] has the browser drop its cookie.
fn session_expiry() -> OffsetDateTime {
    OffsetDateTime::now_utc() + Duration::from_secs(SESSION_TTL_SECS)
}

fn session_not_opened(error: SessionError) -> ApiError {
    tracing::error!(%error, "a session could not be started");
    ApiError::INTERNAL
}

/// The answer to the request that started a session: its cookie, and what
/// [`read_session`] would say of it. The caller has stored `secret`'s hash.
fn session_started(secret: &SessionSecret, credential_id: &[u8]) -> Response {
    (
        [(SET_COOKIE, session::set_cookie(secret))],
        Json(SessionView::of(secret, credential_id)),
    )
        .into_response()
}

pub async fn end_session(
    State(state): State<AppState>,
    current: CurrentSession,
) -> Result<impl IntoResponse, ApiError> {
    sessions::delete_session(current.user, state.db(), &current.secret.hash())
        .await
        .map_err(store_failed)?;
    Ok((
        [(SET_COOKIE, session::clear_cookie())],
        Json(serde_json::json!({})),
    ))
}

#[derive(Serialize)]
pub struct CeremonyStarted {
    ceremony_id: String,
    options: serde_json::Value,
}

/// What finishes a registration or a sign-in. `credential` is whatever JSON
/// the browser made of its authenticator's answer; only the verifier reads it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CeremonyCredential {
    ceremony_id: String,
    credential: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::challenge::RandomSourceFailed;
    use crate::auth::testing::lazy_state;
    use crate::auth::RelyingParty;
    use axum::http::{HeaderValue, Request};

    fn rp() -> RelyingParty {
        RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        }
    }

    async fn rendered(error: ApiError) -> (StatusCode, String) {
        let response = error.into_response();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[test]
    fn a_key_service_to_wait_for_is_503_and_a_misconfigured_one_500() {
        for waiting in [
            KeyServiceError::Sealed,
            KeyServiceError::UnknownKey("transit".to_owned()),
            KeyServiceError::Transport("refused".to_owned()),
            KeyServiceError::Upstream {
                status: 500,
                detail: "down".to_owned(),
            },
        ] {
            assert_eq!(ApiError::from(waiting), ApiError::KEY_SERVICE_UNAVAILABLE);
        }
        for broken in [
            KeyServiceError::InvalidKeyName("a/b".to_owned()),
            KeyServiceError::InvalidMount("m".to_owned()),
            KeyServiceError::Malformed("x".to_owned()),
        ] {
            assert_eq!(ApiError::from(broken), ApiError::INTERNAL);
        }
    }

    #[test]
    fn every_guard_rejection_is_403_under_its_own_code() {
        for (rejection, code) in [
            (session::Rejection::OriginMismatch, "origin_mismatch"),
            (session::Rejection::CsrfMissing, "csrf_required"),
            (session::Rejection::CsrfMismatch, "csrf_mismatch"),
        ] {
            assert_eq!(
                ApiError::from(rejection),
                ApiError::new(StatusCode::FORBIDDEN, code),
                "{rejection:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_error_is_rendered_as_its_status_and_its_code_alone() {
        assert_eq!(
            rendered(ApiError::UNAUTHENTICATED).await,
            (
                StatusCode::UNAUTHORIZED,
                r#"{"error":"unauthenticated"}"#.to_owned()
            )
        );
        assert_eq!(
            rendered(session::Rejection::CsrfMissing.into()).await,
            (
                StatusCode::FORBIDDEN,
                r#"{"error":"csrf_required"}"#.to_owned()
            )
        );
    }

    #[tokio::test]
    async fn a_scope_refusal_is_rendered_with_the_scope_it_is_for() {
        assert_eq!(
            rendered(ApiError::scope_not_granted(Scope::Spend)).await,
            (
                StatusCode::CONFLICT,
                r#"{"error":"scope_not_granted","scope":"spend"}"#.to_owned()
            )
        );
        assert_eq!(
            rendered(ApiError::NOT_FOUND).await,
            (StatusCode::NOT_FOUND, r#"{"error":"not_found"}"#.to_owned())
        );
    }

    #[tokio::test]
    async fn a_started_session_is_answered_with_its_cookie_and_its_view() {
        let secret = SessionSecret::generate().unwrap();

        let response = session_started(&secret, &[0xfb, 0xff, 0xfe]);

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header_text(response.headers(), SET_COOKIE),
            Some(session::set_cookie(&secret).as_str())
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({
                "csrf_token": secret.csrf_token(),
                "credential_id": "-__-",
            })
        );
    }

    #[test]
    fn a_session_row_expires_when_its_cookie_does() {
        let ttl = Duration::from_secs(SESSION_TTL_SECS);
        let before = OffsetDateTime::now_utc();

        let expiry = session_expiry();

        assert!(expiry >= before + ttl, "{expiry}");
        assert!(expiry <= OffsetDateTime::now_utc() + ttl, "{expiry}");
    }

    #[tokio::test]
    async fn each_attachment_refusal_has_its_own_answer() {
        for (refusal, status, body) in [
            (
                AttachmentRefusal::Invalid,
                StatusCode::BAD_REQUEST,
                r#"{"error":"invalid_request"}"#,
            ),
            (
                AttachmentRefusal::NotFound,
                StatusCode::NOT_FOUND,
                r#"{"error":"not_found"}"#,
            ),
            (
                AttachmentRefusal::Unavailable,
                StatusCode::CONFLICT,
                r#"{"error":"attachment_unavailable"}"#,
            ),
        ] {
            assert_eq!(
                rendered(ApiError::from(refusal)).await,
                (status, body.to_owned()),
                "{refusal:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_store_failure_tells_the_client_nothing_of_its_cause() {
        let error = StoreError::TooLarge { bytes: 4242 };
        let (status, body) = rendered(store_failed(error)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, r#"{"error":"internal"}"#);
    }

    /// The state's database is never dialled, and a query against it fails: a
    /// lookup made for any of these requests would answer `500`, not `401`.
    #[tokio::test]
    async fn a_request_without_a_usable_session_cookie_is_401_before_any_lookup() {
        let state = lazy_state("postgres://hub@db.invalid/hub", rp()).unwrap();
        let short = base64url_encode(&[7u8; 31]);
        for cookie in [
            None,
            Some("theme=dark".to_owned()),
            Some(format!("{}=", session::COOKIE_NAME)),
            Some(format!("{}={short}", session::COOKIE_NAME)),
        ] {
            let mut request = Request::builder().uri("/v1/session");
            if let Some(cookie) = &cookie {
                request = request.header(COOKIE, cookie);
            }
            let (mut parts, ()) = request.body(()).unwrap().into_parts();
            let refused = CurrentSession::from_request_parts(&mut parts, &state)
                .await
                .err();
            assert_eq!(refused, Some(ApiError::UNAUTHENTICATED), "{cookie:?}");
        }
    }

    /// `é` once as UTF-8 and once as the single Latin-1 byte, which is not
    /// UTF-8 at all: a header value may hold either, and a strict UTF-8 read
    /// would pass the first and still lose the session beside the second.
    #[test]
    fn the_session_cookie_is_found_beside_a_cookie_that_is_not_ascii() {
        let secret = SessionSecret::generate().unwrap();
        let session_pair = format!("{}={}", session::COOKIE_NAME, secret.cookie_value());
        for other in [&b"theme=caf\xc3\xa9"[..], &b"theme=caf\xe9"[..]] {
            let cookie = [other, b"; ", session_pair.as_bytes()].concat();
            let mut headers = HeaderMap::new();
            headers.insert(COOKIE, HeaderValue::from_bytes(&cookie).unwrap());

            let found = presented_secret(&headers);

            assert_eq!(
                found.map(|found| found.cookie_value()),
                Some(secret.cookie_value()),
                "beside {other:x?}"
            );
        }
    }

    #[test]
    fn a_full_ceremony_map_is_503_and_a_failed_random_source_is_500() {
        assert_eq!(
            ceremony_not_held(InsertError::Full),
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_ceremonies")
        );
        assert_eq!(
            ceremony_not_held(InsertError::RandomSource(RandomSourceFailed)),
            ApiError::INTERNAL
        );
    }
}
