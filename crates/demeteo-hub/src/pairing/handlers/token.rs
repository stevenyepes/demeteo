//! `POST /token`, and the grants behind it: an authorization code redeemed
//! for the first token pair of an instance and the three values it pins, a
//! refresh token rotated for the next pair, and a runner's device code
//! polled for (`super::device`).
//!
//! [`token`] is the route. It reads the form and the `DPoP` header, and hands
//! both to the grant the form names. [`redeemed`] is the pairing grant past
//! those extractors, and the order of its steps is what it guarantees:
//!
//! 1. The proof is verified and then remembered, before the code is looked
//!    at. A proof is spent by being presented, whatever becomes of the
//!    request it came with.
//! 2. The code is taken before anything about the redemption is checked, so a
//!    redemption that fails has used it up and the desktop starts again.
//! 3. The signing key is read before the store is asked for anything. A key
//!    service that cannot answer therefore leaves no instance and no token
//!    behind: there would be no response to hand them over in.
//!
//! The third step, and what follows it, is [`issued`], which the device-code
//! grant shares: both pairing grants end in the one issuance.
//!
//! [`refreshed`] is the rotation, and it begins with the same first step. It
//! never asks the key service for anything: its answer carries none of the
//! pinned values, so a sealed key service does not stop a paired instance
//! from keeping its tokens.
//!
//! Nothing is decided here. Each verdict is a synchronous call into
//! [`dpop`], [`pending`](crate::pairing::pending) or [`grant`], and what a
//! refusal is answered with is one of the `From` impls below. None of them
//! has a wildcard arm: a variant added to a refusal has to be given its
//! answer here before the crate compiles.

use super::device::polled;
use crate::auth::challenge::RandomSourceFailed;
use crate::auth::handlers::{store_failed, unix_now, ApiError};
use crate::pairing::authorize::PairingAsk;
use crate::pairing::dpop::{self, Expected, ProofError, VerifiedProof};
use crate::pairing::grant::{
    self, AuthorizationCode, ExpiryOutOfRange, FamilyChange, FreshPair, PairingResponse,
    RedeemRefusal, RefreshRefusal, RefreshResponse, TokenSecret,
};
use crate::pairing::pending::{ApprovedPairing, RememberRefusal};
use crate::ports::KeyServiceError;
use crate::routes::AppState;
use crate::store::device_keys::NewDeviceKey;
use crate::store::instances::{pair_instance, InstanceKind, NewInstanceLabels, NewPairing, Paired};
use crate::store::tokens::{rotate_family, FamilyWritten, TokenHash, TokenRecord};
use crate::store::{StoreError, UserId};
use axum::extract::{Form, FromRequest, Request, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use demeteo_hub_protocol::Scope;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::future::Future;
use uuid::Uuid;

/// A proof is made for the URL its request is sent to, so this is also the
/// path the token route has to be mounted at.
pub const TOKEN_PATH: &str = "/token";
const TOKEN_METHOD: &str = "POST";
const PROOF_HEADER: &str = "dpop";

/// What a `grant_type=authorization_code` form carries beside its grant
/// type. Deliberately not `Debug`: `code` is the approval and
/// `code_verifier` the secret its challenge was derived from.
#[derive(Deserialize)]
pub struct CodeRedemption {
    code: String,
    code_verifier: String,
    redirect_uri: String,
}

/// What a `grant_type=refresh_token` form carries beside its grant type. Not
/// `Debug` either: `refresh_token` is the token.
#[derive(Deserialize)]
pub struct TokenRefresh {
    refresh_token: String,
}

/// What a device-code poll carries beside its grant type. Not `Debug`:
/// `device_code` is what the tokens are polled for with.
#[derive(Deserialize)]
pub struct DevicePoll {
    pub(super) device_code: String,
}

impl DevicePoll {
    pub fn new(device_code: &str) -> Self {
        Self {
            device_code: device_code.to_owned(),
        }
    }
}

/// A token request, told apart by its `grant_type`. A form without one, or
/// with one sent twice, is not a `Grant` at all and is refused as any other
/// unreadable form is; one whose `grant_type` names nothing below is
/// [`Unsupported`](Self::Unsupported) whatever else it carries.
#[derive(Deserialize)]
#[serde(tag = "grant_type", rename_all = "snake_case")]
pub enum Grant {
    AuthorizationCode(CodeRedemption),
    RefreshToken(TokenRefresh),
    /// RFC 8628 §3.4's grant type, which no case rule spells.
    #[serde(rename = "urn:ietf:params:oauth:grant-type:device_code")]
    DeviceCode(DevicePoll),
    #[serde(other)]
    Unsupported,
}

/// A form body, refused under one code whatever is wrong with it: what
/// [`JsonBody`](crate::auth::handlers::JsonBody) is to `Json`, for the one
/// route whose clients send a form.
pub struct FormBody<T>(pub T);

impl<T, S> FromRequest<S> for FormBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        match Form::<T>::from_request(request, state).await {
            Ok(Form(body)) => Ok(Self(body)),
            Err(_) => Err(ApiError::INVALID_REQUEST),
        }
    }
}

/// The proof of a request that carries exactly one `DPoP` header. A request
/// with two is not asked which it meant (RFC 9449 §4.3): the proof names the
/// key the tokens are bound to, and picking one would be picking the key.
pub(crate) fn one_proof(headers: &HeaderMap) -> Option<&str> {
    let mut presented = headers.get_all(PROOF_HEADER).iter();
    let proof = presented.next()?;
    match presented.next() {
        None => proof.to_str().ok(),
        Some(_) => None,
    }
}

impl From<ProofError> for ApiError {
    fn from(error: ProofError) -> Self {
        match error {
            ProofError::Malformed
            | ProofError::Type
            | ProofError::Algorithm
            | ProofError::Key
            | ProofError::Method
            | ProofError::Uri
            | ProofError::Stale
            | ProofError::Signature => Self::INVALID_DPOP_PROOF,
        }
    }
}

impl From<RememberRefusal> for ApiError {
    fn from(refusal: RememberRefusal) -> Self {
        match refusal {
            RememberRefusal::Replayed => Self::INVALID_DPOP_PROOF,
            RememberRefusal::Full => Self::TOO_MANY_REQUESTS,
        }
    }
}

/// One answer for all four, and the answer an unknown or a used code gets:
/// told apart, they would tell whoever holds a code which of the things they
/// do not have is the one still missing.
impl From<RedeemRefusal> for ApiError {
    fn from(refusal: RedeemRefusal) -> Self {
        match refusal {
            RedeemRefusal::Expired
            | RedeemRefusal::RedirectMismatch
            | RedeemRefusal::VerifierMismatch
            | RedeemRefusal::KeyMismatch => Self::INVALID_GRANT,
        }
    }
}

/// One answer for all five, for the reason a code's refusals share one.
/// [`Reused`](RefreshRefusal::Reused) is among them and is no less acted on:
/// its family is retired by the store, under this refusal.
impl From<RefreshRefusal> for ApiError {
    fn from(refusal: RefreshRefusal) -> Self {
        match refusal {
            RefreshRefusal::Unknown
            | RefreshRefusal::NotRefresh
            | RefreshRefusal::KeyMismatch
            | RefreshRefusal::Reused
            | RefreshRefusal::Expired => Self::INVALID_GRANT,
        }
    }
}

/// The `503`s end without the Hub being touched: an operator unseals, the
/// key service comes back, or the key is created. The `500`s are a name or a
/// mount the Hub was configured with and cannot use, or an answer it cannot
/// read, and asking again changes neither.
///
/// A function and not a `From` impl, because the split is this read's alone:
/// another caller of the key service owes its own answer.
fn signing_key_not_read(error: KeyServiceError) -> ApiError {
    match error {
        KeyServiceError::Sealed
        | KeyServiceError::UnknownKey(_)
        | KeyServiceError::Transport(_)
        | KeyServiceError::Upstream { .. } => {
            tracing::warn!(%error, "the request-signing key could not be read");
            ApiError::KEY_SERVICE_UNAVAILABLE
        }
        KeyServiceError::InvalidKeyName(_)
        | KeyServiceError::InvalidMount(_)
        | KeyServiceError::Malformed(_) => {
            tracing::error!(%error, "the request-signing key is misconfigured");
            ApiError::INTERNAL
        }
    }
}

fn pair_not_minted(error: RandomSourceFailed) -> ApiError {
    tracing::error!(%error, "a token pair could not be minted");
    ApiError::INTERNAL
}

fn rows_not_built(error: ExpiryOutOfRange) -> ApiError {
    tracing::error!(%error, "a token pair could not be issued");
    ApiError::INTERNAL
}

/// The proof a request to the token endpoint carried, verified and spent.
fn spent_proof(
    state: &AppState,
    proof_header: Option<&str>,
    now: u64,
) -> Result<VerifiedProof, ApiError> {
    spent_proof_for(state, TOKEN_PATH, proof_header, now)
}

/// The proof a `POST` to `path` carried, verified and spent.
///
/// `htu` is built from the configured origin. A proof made for the name a
/// request's `Host` gave would verify at whatever name the caller chose.
pub(super) fn spent_proof_for(
    state: &AppState,
    path: &str,
    proof_header: Option<&str>,
    now: u64,
) -> Result<VerifiedProof, ApiError> {
    let presented = proof_header.ok_or(ApiError::INVALID_DPOP_PROOF)?;
    let htu = format!("{}{path}", state.rp().origin);
    let expected = Expected {
        htm: TOKEN_METHOD,
        htu: &htu,
    };
    let proof = dpop::verify(presented, &expected, now)?;
    state.pairings().remember_proof(&proof, now)?;
    Ok(proof)
}

fn taken_code(state: &AppState, presented: &str, now: u64) -> Result<ApprovedPairing, ApiError> {
    AuthorizationCode::parse(presented)
        .and_then(|code| state.pairings().take_code(&code.hash(), now))
        .ok_or(ApiError::INVALID_GRANT)
}

/// What a human approved, whichever grant asked for it: the tenant, the
/// key and labels asked about, and the scopes left checked. `ask.scopes` is
/// what was asked for, before the user narrowed it, and is never granted.
pub struct Granted<'a> {
    pub user: UserId,
    pub ask: &'a PairingAsk,
    pub scopes: &'a [Scope],
    pub kind: InstanceKind,
}

/// What the store is handed for `granted`, redeemed with `proof`.
///
/// The key is the proof's, rebuilt from the point it decoded to, and by the
/// time this is called the grant has said it is the one the user approved.
fn new_pairing(
    granted: &Granted<'_>,
    proof: &VerifiedProof,
    fresh: &FreshPair,
    family_id: Uuid,
    now: u64,
) -> Result<NewPairing, ExpiryOutOfRange> {
    let ask = granted.ask;
    let rows = fresh.rows(family_id, &proof.jkt, now)?;
    Ok(NewPairing {
        kind: granted.kind,
        instance: NewInstanceLabels {
            name: ask.instance_name.clone(),
            os: ask.os.clone(),
            arch: ask.arch.clone(),
            version: ask.app_version.clone(),
        },
        device_key: NewDeviceKey {
            public_jwk: proof.jwk.canonical_json(),
            jkt: proof.jkt.clone(),
        },
        scopes: granted.scopes.to_vec(),
        family_id,
        access: rows.access,
        refresh: rows.refresh,
    })
}

/// Everything a pairing grant does. `proof_header` is the request's one
/// `DPoP` header, and `None` when it carried none or more than one. `pair` is
/// the one thing this asks of a store, so the whole grant is reachable from a
/// test that has no database; the route passes
/// [`pair_instance`](crate::store::instances::pair_instance).
///
/// The passkeys in the answer are the ones `pair` read while it wrote, not
/// the ones the tenant had when the user approved.
pub async fn redeemed<P, F>(
    state: &AppState,
    form: CodeRedemption,
    proof_header: Option<&str>,
    now: u64,
    pair: P,
) -> Result<PairingResponse, ApiError>
where
    P: FnOnce(UserId, NewPairing) -> F,
    F: Future<Output = Result<Paired, StoreError>>,
{
    let proof = spent_proof(state, proof_header, now)?;
    let approved = taken_code(state, &form.code, now)?;
    grant::redeem(
        &approved,
        &form.redirect_uri,
        &form.code_verifier,
        &proof.jkt,
        now,
    )?;
    let granted = Granted {
        user: approved.user,
        ask: &approved.request.ask,
        scopes: &approved.scopes,
        kind: InstanceKind::Desktop,
    };
    issued(state, &granted, &proof, now, pair).await
}

/// The first token pair of an instance, and the three values it pins, for a
/// grant that has already said `proof` is by the key `granted` names.
pub(super) async fn issued<P, F>(
    state: &AppState,
    granted: &Granted<'_>,
    proof: &VerifiedProof,
    now: u64,
    pair: P,
) -> Result<PairingResponse, ApiError>
where
    P: FnOnce(UserId, NewPairing) -> F,
    F: Future<Output = Result<Paired, StoreError>>,
{
    let signing_key = state
        .signing_key()
        .public_key(state.keys())
        .await
        .map_err(signing_key_not_read)?;
    let fresh = FreshPair::generate().map_err(pair_not_minted)?;
    let new = new_pairing(granted, proof, &fresh, Uuid::new_v4(), now).map_err(rows_not_built)?;
    let paired = pair(granted.user, new).await.map_err(store_failed)?;
    Ok(grant::pairing_response(
        fresh,
        granted.scopes,
        &paired,
        &signing_key,
        &state.rp().rp_id,
    ))
}

/// The pair a rotation issued, or why it issued none. The outer error is
/// [`refresh_change`](grant::refresh_change)'s own, and is not a refusal.
pub type Rotated = Result<Result<FreshPair, RefreshRefusal>, ExpiryOutOfRange>;

/// What a rotation has a store run over the rows it locked for it.
///
/// It owns everything it reads, the pair it would issue included, and hands
/// that pair back in its answer. A store call outlives any borrow
/// [`refreshed`] could lend it, and a [`FreshPair`] cannot be copied to leave
/// one behind.
pub type Rotation = Box<dyn FnOnce(&[TokenRecord]) -> (FamilyChange, Rotated) + Send>;

/// [`refresh_change`](grant::refresh_change) in the shape
/// [`rotate_family`] takes a decision in, and nothing beside it: the change
/// is the decision's whatever its verdict, which is what lets the store
/// commit a reused family's retirement under a refusal.
fn rotation(presented: TokenHash, proof_jkt: String, now: u64, fresh: FreshPair) -> Rotation {
    Box::new(move |family: &[TokenRecord]| {
        match grant::refresh_change(family, &presented, &proof_jkt, now, &fresh) {
            Ok(decision) => (decision.change, Ok(decision.verdict.map(|()| fresh))),
            Err(error) => (FamilyChange::default(), Err(error)),
        }
    })
}

/// Everything a rotation does. `proof_header` is as [`redeemed`] takes it.
/// `rotate` is the one thing this asks of a store, so the whole grant is
/// reachable from a test that has no database; the route passes
/// [`rotate_family`].
///
/// A text that is not a token is a token no row holds, and is refused as one
/// without the store being asked. Of what `rotate` answers only the pair is
/// read: the rows it wrote are the store's account of itself.
pub async fn refreshed<R, F>(
    state: &AppState,
    form: TokenRefresh,
    proof_header: Option<&str>,
    now: u64,
    rotate: R,
) -> Result<RefreshResponse, ApiError>
where
    R: FnOnce(TokenHash, Rotation) -> F,
    F: Future<Output = Result<FamilyWritten<Rotated>, StoreError>>,
{
    let proof = spent_proof(state, proof_header, now)?;
    let presented = TokenSecret::parse(&form.refresh_token)
        .ok_or(RefreshRefusal::Unknown)?
        .hash();
    let fresh = FreshPair::generate().map_err(pair_not_minted)?;
    let decide = rotation(presented.clone(), proof.jkt, now, fresh);
    let written = rotate(presented, decide).await.map_err(store_failed)?;
    let fresh = written.answer.map_err(rows_not_built)??;
    Ok(grant::refresh_response(fresh))
}

async fn granted(
    state: &AppState,
    headers: &HeaderMap,
    form: Result<FormBody<Grant>, ApiError>,
) -> Result<Response, ApiError> {
    let FormBody(grant) = form?;
    let (db, proof) = (state.db(), one_proof(headers));
    match grant {
        Grant::AuthorizationCode(form) => {
            let pair = |user, new: NewPairing| async move { pair_instance(user, db, &new).await };
            let response = redeemed(state, form, proof, unix_now(), pair).await?;
            Ok(Json(response).into_response())
        }
        Grant::RefreshToken(form) => {
            let rotate = |presented: TokenHash, decide: Rotation| async move {
                rotate_family(db, &presented, decide).await
            };
            let response = refreshed(state, form, proof, unix_now(), rotate).await?;
            Ok(Json(response).into_response())
        }
        Grant::DeviceCode(form) => {
            let pair = |user, new: NewPairing| async move { pair_instance(user, db, &new).await };
            let response = polled(state, form, proof, unix_now(), pair).await?;
            Ok(Json(response).into_response())
        }
        Grant::Unsupported => Err(ApiError::UNSUPPORTED_GRANT_TYPE),
    }
}

/// `POST /token`. Every answer is marked `no-store`, a refusal as much as a
/// grant, so that which answers hold a token is never a question a cache
/// between the Hub and a desktop gets to answer.
///
/// The form is taken as a `Result` for that reason: refused by the extractor
/// itself, a body this cannot read would be answered before this ran, and
/// without the header.
pub async fn token(
    State(state): State<AppState>,
    headers: HeaderMap,
    form: Result<FormBody<Grant>, ApiError>,
) -> Response {
    let answer = granted(&state, &headers, form).await;
    ([(CACHE_CONTROL, "no-store")], answer).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::memory_keys::MemoryKeys;
    use crate::auth::testing::lazy_state_with_keys;
    use crate::auth::RelyingParty;
    use crate::pairing::authorize::{AuthorizeRequest, PairingAsk};
    use crate::pairing::testing::{MemoryFamily, SoftDeviceKey};
    use crate::store::tokens::TokenKind;
    use crate::store::InstanceId;
    use axum::body::Body;
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{HeaderValue, StatusCode};
    use demeteo_hub_protocol::Scope;
    use std::sync::Arc;
    use time::OffsetDateTime;

    const NOW: u64 = 1_700_000_000;
    const TOKEN_URL: &str = "https://hub.example/token";
    const SIGNING_KEY: &str = "signing";
    /// RFC 7636 Appendix B.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    const REDIRECT: &str = "http://127.0.0.1:49152/callback";

    const INVALID_DPOP_PROOF: ApiError =
        ApiError::new(StatusCode::BAD_REQUEST, "invalid_dpop_proof");
    const INVALID_GRANT: ApiError = ApiError::new(StatusCode::BAD_REQUEST, "invalid_grant");
    const INVALID_REQUEST: ApiError = ApiError::new(StatusCode::BAD_REQUEST, "invalid_request");

    const FORM: &str = "application/x-www-form-urlencoded";
    const REDEMPTION: &str = "grant_type=authorization_code&code=abc&code_verifier=a-b.c_d~e\
        &redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcallback";

    /// What the route's extractor makes of `body`: the fields of the grant
    /// it names, or `None` for a grant type the Hub does not serve.
    async fn read(content_type: &str, body: &str) -> Result<Option<Vec<String>>, ApiError> {
        let request = Request::builder()
            .method(TOKEN_METHOD)
            .uri(TOKEN_PATH)
            .header(CONTENT_TYPE, content_type)
            .body(Body::from(body.to_owned()))
            .unwrap();
        let FormBody(grant) = FormBody::<Grant>::from_request(request, &()).await?;
        Ok(match grant {
            Grant::AuthorizationCode(form) => {
                Some(vec![form.code, form.code_verifier, form.redirect_uri])
            }
            Grant::RefreshToken(form) => Some(vec![form.refresh_token]),
            Grant::DeviceCode(form) => Some(vec![form.device_code]),
            Grant::Unsupported => None,
        })
    }

    #[tokio::test]
    async fn a_redemption_is_read_from_a_form_decoded_and_whatever_else_the_form_carries() {
        let fields = ["abc", "a-b.c_d~e", REDIRECT].map(str::to_owned).to_vec();

        assert_eq!(read(FORM, REDEMPTION).await, Ok(Some(fields.clone())));
        let with_more = format!("client_id=desktop&{REDEMPTION}&resource=x");
        assert_eq!(read(FORM, &with_more).await, Ok(Some(fields)));
    }

    #[tokio::test]
    async fn a_refresh_is_read_from_a_form_that_names_its_token_once() {
        let refresh = "grant_type=refresh_token&refresh_token=abc";

        let token = vec!["abc".to_owned()];

        assert_eq!(read(FORM, refresh).await, Ok(Some(token.clone())));
        let with_more = format!("scope=read&{refresh}&resource=x");
        assert_eq!(read(FORM, &with_more).await, Ok(Some(token)));
        for unreadable in [
            "grant_type=refresh_token".to_owned(),
            format!("{refresh}&refresh_token=abc"),
            format!("{refresh}&{REDEMPTION}"),
        ] {
            let read = read(FORM, &unreadable).await;
            assert_eq!(read, Err(INVALID_REQUEST), "{unreadable:?}");
        }
    }

    #[tokio::test]
    async fn a_device_code_poll_is_read_under_its_urn_and_no_other_spelling() {
        let poll =
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code&device_code=abc";

        assert_eq!(read(FORM, poll).await, Ok(Some(vec!["abc".to_owned()])));
        for unserved in [
            "device_code",
            "urn:ietf:params:oauth:grant-type:Device_Code",
        ] {
            let form = format!("grant_type={unserved}&device_code=abc");
            assert_eq!(read(FORM, &form).await, Ok(None), "{unserved:?}");
        }
        assert_eq!(
            read(FORM, &format!("{poll}&device_code=abc")).await,
            Err(INVALID_REQUEST)
        );
    }

    #[tokio::test]
    async fn any_other_grant_type_is_unsupported_whatever_the_form_carries_beside_it() {
        let unserved = [
            "password",
            "client_credentials",
            "AUTHORIZATION_CODE",
            "refresh",
            "",
        ];
        for grant_type in unserved {
            assert_eq!(
                read(FORM, &format!("grant_type={grant_type}")).await,
                Ok(None),
                "{grant_type:?}"
            );
            let beside = format!("code=abc&grant_type={grant_type}&code=abc");
            assert_eq!(read(FORM, &beside).await, Ok(None), "{grant_type:?}");
        }
    }

    #[tokio::test]
    async fn a_form_that_cannot_be_read_is_invalid_request_whatever_is_wrong_with_it() {
        assert!(matches!(read(FORM, REDEMPTION).await, Ok(Some(_))));

        let without = |field: &str| {
            let kept = REDEMPTION
                .split('&')
                .filter(|pair| !pair.starts_with(field));
            kept.collect::<Vec<_>>().join("&")
        };
        let unreadable = [
            String::new(),
            without("grant_type="),
            without("code="),
            without("code_verifier="),
            without("redirect_uri="),
            format!("{REDEMPTION}&code=abc"),
            format!("{REDEMPTION}&grant_type=authorization_code"),
            format!("grant_type=password&{REDEMPTION}"),
        ];
        for body in unreadable {
            assert_eq!(read(FORM, &body).await, Err(INVALID_REQUEST), "{body:?}");
        }
        for content_type in ["application/json", "text/plain"] {
            assert_eq!(
                read(content_type, REDEMPTION).await,
                Err(INVALID_REQUEST),
                "{content_type}"
            );
        }
    }

    #[test]
    fn a_proof_is_read_from_a_request_that_carries_exactly_one() {
        let carrying = |proofs: &[&[u8]]| {
            let mut headers = HeaderMap::new();
            for proof in proofs {
                headers.append("DPoP", HeaderValue::from_bytes(proof).unwrap());
            }
            headers
        };

        assert_eq!(one_proof(&carrying(&[b"a.b.c"])), Some("a.b.c"));
        assert_eq!(one_proof(&carrying(&[])), None);
        assert_eq!(one_proof(&carrying(&[b"a.b.c", b"a.b.c"])), None);
        assert_eq!(one_proof(&carrying(&[b"a.b.c", b"d.e.f"])), None);
        assert_eq!(one_proof(&carrying(&[b"a.b.\xff"])), None);
    }

    /// The database is never dialled: `pair` stands where it would be.
    fn state() -> AppState {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        };
        let keys = Arc::new(MemoryKeys::with_keys([SIGNING_KEY]));
        lazy_state_with_keys("postgres://hub@db.invalid/hub", rp, keys, SIGNING_KEY).unwrap()
    }

    /// What a user approved at `NOW` for `key`: `read` and `gates` asked
    /// for, `gates` left checked.
    fn approval(key: &SoftDeviceKey) -> ApprovedPairing {
        ApprovedPairing {
            user: UserId::generate(),
            request: AuthorizeRequest {
                code_challenge: CHALLENGE.to_owned(),
                redirect_uri: REDIRECT.to_owned(),
                state: "xyz-123".to_owned(),
                ask: PairingAsk {
                    dpop_jkt: key.thumbprint(),
                    scopes: vec![Scope::Read, Scope::Gates],
                    instance_name: "Workstation".to_owned(),
                    os: "linux".to_owned(),
                    arch: "x86_64".to_owned(),
                    app_version: "1.4.0".to_owned(),
                },
            },
            scopes: vec![Scope::Gates],
            approved_at: NOW,
        }
    }

    /// [`approval`] parked under a fresh code, and the code's text.
    fn parked(state: &AppState, key: &SoftDeviceKey) -> String {
        let code = AuthorizationCode::generate().unwrap();
        state
            .pairings()
            .park_code(code.hash(), approval(key), NOW)
            .unwrap();
        code.text()
    }

    fn form(code: &str, code_verifier: &str) -> CodeRedemption {
        CodeRedemption {
            code: code.to_owned(),
            code_verifier: code_verifier.to_owned(),
            redirect_uri: REDIRECT.to_owned(),
        }
    }

    /// A redemption at `NOW` whose store answers an instance with no
    /// passkeys: the refusal, or `None` for a grant.
    async fn refusal(
        state: &AppState,
        form: CodeRedemption,
        proof: Option<&str>,
    ) -> Option<ApiError> {
        let paired = Paired {
            instance_id: InstanceId::generate(),
            passkeys: Vec::new(),
        };
        redeemed(state, form, proof, NOW, |_, _| async { Ok(paired) })
            .await
            .err()
    }

    #[test]
    fn every_fault_of_a_proof_and_a_replay_of_one_is_400_and_a_full_window_is_503() {
        for fault in [
            ProofError::Malformed,
            ProofError::Type,
            ProofError::Algorithm,
            ProofError::Key,
            ProofError::Method,
            ProofError::Uri,
            ProofError::Stale,
            ProofError::Signature,
        ] {
            assert_eq!(ApiError::from(fault), INVALID_DPOP_PROOF, "{fault:?}");
        }
        assert_eq!(
            ApiError::from(RememberRefusal::Replayed),
            INVALID_DPOP_PROOF
        );
        assert_eq!(
            ApiError::from(RememberRefusal::Full),
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_requests")
        );
    }

    #[test]
    fn every_refusal_of_a_code_is_the_one_400() {
        for refusal in [
            RedeemRefusal::Expired,
            RedeemRefusal::RedirectMismatch,
            RedeemRefusal::VerifierMismatch,
            RedeemRefusal::KeyMismatch,
        ] {
            assert_eq!(ApiError::from(refusal), INVALID_GRANT, "{refusal:?}");
        }
    }

    #[test]
    fn every_refusal_of_a_refresh_token_is_the_one_400() {
        for refusal in [
            RefreshRefusal::Unknown,
            RefreshRefusal::NotRefresh,
            RefreshRefusal::KeyMismatch,
            RefreshRefusal::Reused,
            RefreshRefusal::Expired,
        ] {
            assert_eq!(ApiError::from(refusal), INVALID_GRANT, "{refusal:?}");
        }
    }

    #[test]
    fn a_key_service_that_may_yet_answer_is_503_and_one_that_is_misconfigured_is_500() {
        for waiting in [
            KeyServiceError::Sealed,
            KeyServiceError::UnknownKey("signing".to_owned()),
            KeyServiceError::Transport("refused".to_owned()),
            KeyServiceError::Upstream {
                status: 403,
                detail: "permission denied".to_owned(),
            },
        ] {
            let shown = format!("{waiting:?}");
            assert_eq!(
                signing_key_not_read(waiting),
                ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "key_service_unavailable"),
                "{shown}"
            );
        }
        for misconfigured in [
            KeyServiceError::InvalidKeyName("a/b".to_owned()),
            KeyServiceError::InvalidMount("a b".to_owned()),
            KeyServiceError::Malformed("31 bytes".to_owned()),
        ] {
            let shown = format!("{misconfigured:?}");
            assert_eq!(
                signing_key_not_read(misconfigured),
                ApiError::INTERNAL,
                "{shown}"
            );
        }
        assert_eq!(pair_not_minted(RandomSourceFailed), ApiError::INTERNAL);
        assert_eq!(rows_not_built(ExpiryOutOfRange), ApiError::INTERNAL);
    }

    #[test]
    fn the_store_is_handed_the_proofs_key_the_narrowed_scopes_and_one_familys_rows() {
        let key = SoftDeviceKey::new();
        let proof = dpop::verify(
            &key.proof("POST", TOKEN_URL, NOW, "jti-1"),
            &Expected {
                htm: "POST",
                htu: TOKEN_URL,
            },
            NOW,
        )
        .unwrap();
        let fresh = FreshPair::generate().unwrap();
        let family_id = Uuid::from_u128(7);

        let approved = approval(&key);
        let granted = Granted {
            user: approved.user,
            ask: &approved.request.ask,
            scopes: &approved.scopes,
            kind: InstanceKind::Desktop,
        };
        let new = new_pairing(&granted, &proof, &fresh, family_id, NOW).unwrap();

        assert_eq!(
            new.instance,
            NewInstanceLabels {
                name: "Workstation".to_owned(),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                version: "1.4.0".to_owned(),
            }
        );
        assert_eq!(
            new.device_key,
            NewDeviceKey {
                public_jwk: proof.jwk.canonical_json(),
                jkt: key.thumbprint(),
            }
        );
        assert_eq!(new.scopes, [Scope::Gates]);
        assert_eq!(new.kind, InstanceKind::Desktop);
        assert_eq!(new.family_id, family_id);
        assert_eq!(new.access.kind, TokenKind::Access);
        assert_eq!(new.access.hash, fresh.access.hash());
        assert_eq!(new.refresh.kind, TokenKind::Refresh);
        assert_eq!(new.refresh.hash, fresh.refresh.hash());
        let at = |unix| OffsetDateTime::from_unix_timestamp(unix).unwrap();
        assert_eq!(new.access.expires_at, at(1_700_000_900));
        assert_eq!(new.refresh.expires_at, at(1_702_592_000));
        for row in [&new.access, &new.refresh] {
            assert_eq!(row.family_id, family_id);
            assert_eq!(row.jkt, key.thumbprint());
        }
    }

    #[tokio::test]
    async fn a_request_without_a_proof_is_refused_before_its_code_is_taken() {
        let state = state();
        let key = SoftDeviceKey::new();
        let code = parked(&state, &key);

        let without = refusal(&state, form(&code, VERIFIER), None).await;

        assert_eq!(without, Some(INVALID_DPOP_PROOF));
        let proof = key.proof("POST", TOKEN_URL, NOW, "jti-1");
        let with = refusal(&state, form(&code, VERIFIER), Some(&proof)).await;
        assert_eq!(with, None);
    }

    #[tokio::test]
    async fn a_proof_presented_twice_is_refused_and_leaves_the_second_requests_code_alone() {
        let state = state();
        let key = SoftDeviceKey::new();
        let (first, second) = (parked(&state, &key), parked(&state, &key));
        let proof = key.proof("POST", TOKEN_URL, NOW, "jti-1");
        assert_eq!(
            refusal(&state, form(&first, VERIFIER), Some(&proof)).await,
            None
        );

        let replayed = refusal(&state, form(&second, VERIFIER), Some(&proof)).await;

        assert_eq!(replayed, Some(INVALID_DPOP_PROOF));
        let another = key.proof("POST", TOKEN_URL, NOW, "jti-2");
        assert_eq!(
            refusal(&state, form(&second, VERIFIER), Some(&another)).await,
            None
        );
    }

    #[tokio::test]
    async fn a_failed_redemption_burns_its_code_and_spends_its_proof() {
        let state = state();
        let key = SoftDeviceKey::new();
        let code = parked(&state, &key);
        let wrong_verifier = "a".repeat(43);
        let proof = key.proof("POST", TOKEN_URL, NOW, "jti-1");

        let failed = refusal(&state, form(&code, &wrong_verifier), Some(&proof)).await;

        assert_eq!(failed, Some(INVALID_GRANT));
        assert_eq!(
            refusal(&state, form(&code, VERIFIER), Some(&proof)).await,
            Some(INVALID_DPOP_PROOF)
        );
        let another = key.proof("POST", TOKEN_URL, NOW, "jti-2");
        assert_eq!(
            refusal(&state, form(&code, VERIFIER), Some(&another)).await,
            Some(INVALID_GRANT)
        );
        let third = key.proof("POST", TOKEN_URL, NOW, "jti-3");
        assert_eq!(
            refusal(&state, form("not-a-code", VERIFIER), Some(&third)).await,
            Some(INVALID_GRANT)
        );
    }

    /// A Hub whose key service answers nothing: a rotation that is granted
    /// on it did not ask.
    fn sealed_state() -> AppState {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        };
        let keys = Arc::new(MemoryKeys::sealed());
        lazy_state_with_keys("postgres://hub@db.invalid/hub", rp, keys, SIGNING_KEY).unwrap()
    }

    /// One refresh at `now` against a store holding `family`: what it was
    /// answered, and the change the store was handed, if it was asked. The
    /// change is written whatever the answer, as [`rotate_family`] writes it.
    async fn refresh(
        state: &AppState,
        family: &mut MemoryFamily,
        refresh_token: &str,
        proof: Option<&str>,
        now: u64,
    ) -> (Result<RefreshResponse, ApiError>, Option<FamilyChange>) {
        let form = TokenRefresh {
            refresh_token: refresh_token.to_owned(),
        };
        let mut handed = None;
        let rotate = |_, decide: Rotation| {
            let (change, answer) = decide(family.rows());
            family.apply(&change, now);
            handed = Some(change);
            let written = FamilyWritten {
                retired: Vec::new(),
                inserted: Vec::new(),
                answer,
            };
            async { Ok(written) }
        };
        let answer = refreshed(state, form, proof, now, rotate).await;
        (answer, handed)
    }

    fn live(family: &MemoryFamily) -> usize {
        let live = |row: &&TokenRecord| row.retired_at.is_none();
        family.rows().iter().filter(live).count()
    }

    #[tokio::test]
    async fn a_refresh_without_a_proof_or_of_a_text_that_is_no_token_never_asks_the_store() {
        let state = sealed_state();
        let key = SoftDeviceKey::new();
        let first = FreshPair::generate().unwrap();
        let mut family = MemoryFamily::paired(&first, &key.thumbprint(), NOW);
        let token = first.refresh.text();

        let (without, asked) = refresh(&state, &mut family, &token, None, NOW).await;

        assert_eq!(without.err(), Some(INVALID_DPOP_PROOF));
        assert!(asked.is_none());

        let proof = key.proof("POST", TOKEN_URL, NOW, "jti-1");
        let (no_token, asked) = refresh(&state, &mut family, "abc", Some(&proof), NOW).await;

        assert_eq!(no_token.err(), Some(INVALID_GRANT));
        assert!(asked.is_none());
        let (replayed, asked) = refresh(&state, &mut family, &token, Some(&proof), NOW).await;
        assert_eq!(replayed.err(), Some(INVALID_DPOP_PROOF));
        assert!(asked.is_none());
        assert_eq!(live(&family), 2);
    }

    #[tokio::test]
    async fn a_rotation_answers_the_pair_whose_rows_the_store_was_handed() {
        let state = sealed_state();
        let key = SoftDeviceKey::new();
        let first = FreshPair::generate().unwrap();
        let mut family = MemoryFamily::paired(&first, &key.thumbprint(), NOW);
        let presented = family.find(&first.refresh.hash()).unwrap().clone();
        let proof = key.proof("POST", TOKEN_URL, NOW + 60, "jti-1");

        let (answer, handed) = refresh(
            &state,
            &mut family,
            &first.refresh.text(),
            Some(&proof),
            NOW + 60,
        )
        .await;

        let answer = serde_json::to_value(answer.unwrap()).unwrap();
        let change = handed.unwrap();
        assert_eq!(change.retire, [presented.id]);
        let [access, refresh] = change.insert.as_slice() else {
            panic!("{change:?}");
        };
        let hash_of = |member: &str| {
            let text = answer[member].as_str().unwrap();
            TokenSecret::parse(text).unwrap().hash()
        };
        assert_eq!(access.kind, TokenKind::Access);
        assert_eq!(access.hash, hash_of("access_token"));
        assert_eq!(refresh.kind, TokenKind::Refresh);
        assert_eq!(refresh.hash, hash_of("refresh_token"));
        for row in [access, refresh] {
            assert_eq!(row.family_id, presented.family_id);
            assert_eq!(row.jkt, key.thumbprint());
        }
        assert_eq!(
            answer,
            serde_json::json!({
                "access_token": answer["access_token"],
                "token_type": "DPoP",
                "expires_in": 900,
                "refresh_token": answer["refresh_token"],
            })
        );
    }

    #[tokio::test]
    async fn a_refusal_is_answered_after_its_change_reached_the_store() {
        let state = sealed_state();
        let (key, another) = (SoftDeviceKey::new(), SoftDeviceKey::new());
        let first = FreshPair::generate().unwrap();
        let mut family = MemoryFamily::paired(&first, &key.thumbprint(), NOW);
        let token = first.refresh.text();
        let proof = |key: &SoftDeviceKey, jti: &str| key.proof("POST", TOKEN_URL, NOW, jti);

        let by_another = proof(&another, "jti-1");
        let (refused, handed) = refresh(&state, &mut family, &token, Some(&by_another), NOW).await;

        assert_eq!(refused.err(), Some(INVALID_GRANT));
        let change = handed.unwrap();
        assert!(change.retire.is_empty() && change.insert.is_empty());
        assert_eq!(live(&family), 2);

        let rotating = proof(&key, "jti-2");
        let (rotated, _) = refresh(&state, &mut family, &token, Some(&rotating), NOW).await;
        assert!(rotated.is_ok());
        assert_eq!(live(&family), 3);

        let again = proof(&key, "jti-3");
        let (reused, handed) = refresh(&state, &mut family, &token, Some(&again), NOW).await;

        assert_eq!(reused.err(), Some(INVALID_GRANT));
        let change = handed.unwrap();
        assert_eq!(change.retire.len(), 3);
        assert!(change.insert.is_empty());
        assert_eq!(live(&family), 0);
    }
}
