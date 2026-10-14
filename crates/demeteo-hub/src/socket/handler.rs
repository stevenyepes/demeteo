//! The instance socket's route: what a request to [`SOCKET_PATH`] has to
//! present before its upgrade is acted on, who it is admitted as, and the
//! WebSocket a session is then run over.
//!
//! [`socket`] is the route. It reads two headers, asks [`admitted`], seats
//! the connection and only then answers `101`. Every refusal is therefore
//! plain HTTP, and none is a `101` followed by a close: a peer that could be
//! made to open a WebSocket before it was judged would hold a task and a
//! registry entry for as long as it liked.
//!
//! [`admitted`] is the whole of admission past the extractors, and the order
//! of its steps is what it guarantees:
//!
//! 1. One proof and one token that reads as a token, before anything is
//!    verified. A request that names no key or no token costs no ECDSA.
//! 2. The proof is verified and then remembered before any row is read, the
//!    order `/token` keeps and for its reason: a proof is spent by being
//!    presented, whatever becomes of the request it came with.
//! 3. `ath` before the store is asked. A proof that does not name the token
//!    beside it was made for another request, and what that token's row says
//!    is not this caller's to learn.
//! 4. The token's row, then [`access_verdict`](grant::access_verdict) over it,
//!    then the instance of the key. The tenant and the instance are the row's
//!    and the key's: nothing the request says chooses either.
//!
//! # One answer
//!
//! Every credential failure is [`ApiError::INVALID_TOKEN`], a missing header
//! as much as a retired token, for the reason the refusals of a code share
//! one in `pairing/handlers/token.rs`. A captured upgrade request is exactly
//! a token with something missing, and an answer per fault would say what.
//!
//! `ProofError` and `RememberRefusal` already convert into `ApiError`, with
//! `/token`'s `400`. **A bare `?` on either compiles here and answers that
//! `400`.** The mappings below are therefore functions, applied by name at
//! every site, and none has a wildcard arm: a variant added to a refusal has
//! to be given the socket's answer before the crate compiles.

use super::connections::{ConnectionId, Registered};
use super::port::{PeerGone, Received, Transport};
use super::postgres::PostgresStore;
use super::session::{self, Limits};
use super::{MAX_MESSAGE_BYTES, PING_INTERVAL_SECS, PONG_DEADLINE_SECS};
use crate::auth::challenge::base64url_encode;
use crate::auth::handlers::{header_text, store_failed, unix_now, ApiError};
use crate::pairing::dpop::{self, Expected, ProofError, VerifiedProof};
use crate::pairing::grant::{self, AccessRefusal, TokenSecret};
use crate::pairing::handlers::one_proof;
use crate::pairing::pending::RememberRefusal;
use crate::routes::AppState;
use crate::store::tokens::{self, TokenHash, TokenRecord};
use crate::store::{device_keys, InstanceId, StoreError, UserId};
use async_trait::async_trait;
use aws_lc_rs::constant_time::verify_slices_are_equal;
use axum::body::Bytes;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::time::Duration;

/// A proof is made for the URL its request is sent to, so this is also the
/// path the socket has to be mounted at.
pub const SOCKET_PATH: &str = "/socket";
const SOCKET_METHOD: &str = "GET";
const TOKEN_SCHEME: &str = "DPoP";

/// Who an upgrade was admitted as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admission {
    pub user: UserId,
    pub instance: InstanceId,
    /// Unix seconds: the `expires_at` of the token presented. It is carried
    /// out because admission happens once and a socket outlives it. A
    /// connection held open past this would outlive the bound an expiry is
    /// there to put on whoever holds the token.
    pub token_expires_at: u64,
}

/// The access token of an `Authorization` header, in both the forms admission
/// needs: the text for `ath`, the hash for the lookup. Deliberately not
/// `Debug`: `text` is the token.
struct PresentedToken<'a> {
    text: &'a str,
    hash: TokenHash,
}

/// `DPoP <token>`, with exactly one space. The scheme is matched in any case
/// (RFC 9110 §11.1); the token is not trimmed or normalised, since `ath` is
/// a digest of its text and there is one text a client could have hashed.
fn presented_token(authorization: &str) -> Option<PresentedToken<'_>> {
    let (scheme, text) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case(TOKEN_SCHEME) {
        return None;
    }
    let hash = TokenSecret::parse(text)?.hash();
    Some(PresentedToken { text, hash })
}

/// Whether a proof's `ath` is the one RFC 9449 §4.2 defines for `token`:
/// unpadded base64url of SHA-256 over the token's ASCII text. Not
/// [`TokenHash`], which is a digest of the 32 bytes that text encodes; the
/// two never agree.
fn names_token(ath: Option<&str>, token: &str) -> bool {
    let expected = base64url_encode(&Sha256::digest(token.as_bytes()));
    ath.is_some_and(|ath| verify_slices_are_equal(ath.as_bytes(), expected.as_bytes()).is_ok())
}

fn proof_refused(error: ProofError) -> ApiError {
    match error {
        ProofError::Malformed
        | ProofError::Type
        | ProofError::Algorithm
        | ProofError::Key
        | ProofError::Method
        | ProofError::Uri
        | ProofError::Stale
        | ProofError::Signature => ApiError::INVALID_TOKEN,
    }
}

/// `Full` is about the Hub and not about the caller, so it keeps the answer
/// `/token` gives it: a client that retries a `503` is right to, and one told
/// its token was invalid would refresh a token that was fine.
fn proof_not_remembered(refusal: RememberRefusal) -> ApiError {
    match refusal {
        RememberRefusal::Replayed => ApiError::INVALID_TOKEN,
        RememberRefusal::Full => ApiError::TOO_MANY_REQUESTS,
    }
}

fn token_refused(refusal: AccessRefusal) -> ApiError {
    match refusal {
        AccessRefusal::NotAccess
        | AccessRefusal::KeyMismatch
        | AccessRefusal::Retired
        | AccessRefusal::Expired => ApiError::INVALID_TOKEN,
    }
}

/// The proof an upgrade carried, verified and spent.
///
/// `htu` is the configured origin and [`SOCKET_PATH`]: the `https` spelling,
/// though the client dials `wss`. `htu` is compared as a whole string, so
/// there has to be one spelling, and the one built from configuration is the
/// one `/token` already has a client build. Never from `Host`, for the reason
/// given where `/token` builds its own.
fn spent_proof(state: &AppState, presented: &str, now: u64) -> Result<VerifiedProof, ApiError> {
    let htu = format!("{}{SOCKET_PATH}", state.rp().origin);
    let expected = Expected {
        htm: SOCKET_METHOD,
        htu: &htu,
    };
    let proof = dpop::verify(presented, &expected, now).map_err(proof_refused)?;
    state
        .pairings()
        .remember_proof(&proof, now)
        .map_err(proof_not_remembered)?;
    Ok(proof)
}

/// Everything admission does. `authorization` is the request's
/// `Authorization` header and `proof` its one `DPoP` header, `None` when it
/// carried none or more than one. `find_token` and `instance_of_key` are the
/// two things this asks of a store, so the whole of it is reachable from a
/// test that has no database; the route passes
/// [`find_token_by_hash`](crate::store::tokens::find_token_by_hash) and
/// [`instance_of_key`](crate::store::device_keys::instance_of_key).
///
/// `instance_of_key` is asked last and only for a token that passed: its
/// `None` is a key that was revoked, or whose instance was deleted, under a
/// token that has not yet expired.
pub async fn admitted<T, TF, I, IF>(
    state: &AppState,
    authorization: Option<&str>,
    proof: Option<&str>,
    now: u64,
    find_token: T,
    instance_of_key: I,
) -> Result<Admission, ApiError>
where
    T: FnOnce(TokenHash) -> TF,
    TF: Future<Output = Result<Option<TokenRecord>, StoreError>>,
    I: FnOnce(UserId, String) -> IF,
    IF: Future<Output = Result<Option<InstanceId>, StoreError>>,
{
    let presented = proof.ok_or(ApiError::INVALID_TOKEN)?;
    let token = authorization
        .and_then(presented_token)
        .ok_or(ApiError::INVALID_TOKEN)?;
    let proof = spent_proof(state, presented, now)?;
    if !names_token(proof.ath.as_deref(), token.text) {
        return Err(ApiError::INVALID_TOKEN);
    }
    let row = find_token(token.hash)
        .await
        .map_err(store_failed)?
        .ok_or(ApiError::INVALID_TOKEN)?;
    grant::access_verdict(&row, &proof.jkt, now).map_err(token_refused)?;
    let token_expires_at =
        u64::try_from(row.expires_at.unix_timestamp()).map_err(|_| ApiError::INVALID_TOKEN)?;
    let instance = instance_of_key(row.user_id, proof.jkt)
        .await
        .map_err(store_failed)?
        .ok_or(ApiError::INVALID_TOKEN)?;
    Ok(Admission {
        user: row.user_id,
        instance,
        token_expires_at,
    })
}

/// What a refused upgrade is answered. A `401` names the scheme it wanted
/// (RFC 9110 §15.5.2 requires a challenge of one, RFC 9449 §7.1 says which),
/// and it is added here, by status, so that no refusal of [`admitted`] can
/// be a `401` without it.
///
/// **Rejected: a challenge on `ApiError` itself.** The `/v1` routes answer
/// `401` to a browser with no session, and a challenge there would name a
/// scheme those routes do not take.
fn refusal(error: ApiError) -> Response {
    let mut response = error.into_response();
    if response.status() == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static(TOKEN_SCHEME));
    }
    response
}

/// One connection's entry in the registry, given up when this is dropped.
///
/// A drop and not a call after the session returns. The entry is made before
/// the `101` is written, and for a peer that goes away between the two axum
/// drops what it was to call with the WebSocket and never calls it. A call
/// placed after [`session::run`] would not be reached then, and the entry
/// would stay, counted against the bound and waking nobody, until that
/// instance connected again.
struct Seat {
    state: AppState,
    instance: InstanceId,
    connection: ConnectionId,
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.state
            .connections()
            .release(self.instance, self.connection);
    }
}

async fn served(socket: WebSocket, seat: Seat, registered: Registered, admission: Admission) {
    let store = PostgresStore::new(seat.state.db().clone(), admission.user, admission.instance);
    let limits = Limits {
        ping_interval: Duration::from_secs(PING_INTERVAL_SECS),
        pong_deadline: Duration::from_secs(PONG_DEADLINE_SECS),
    };
    let ended = session::run(socket, store, registered, admission, unix_now, limits).await;
    tracing::info!(
        instance = %admission.instance.0,
        connection = ?seat.connection,
        ?ended,
        "an instance socket ended"
    );
}

/// [`socket`] past its extractors, with the refusal still an [`ApiError`].
///
/// `upgrade` is looked at only once the request is admitted, so a caller
/// with no credentials learns nothing from here about what else its request
/// lacked. It is looked at before the connection is seated: seating displaces
/// the session an instance already has, and a request that cannot become a
/// socket must not cost the instance the one it holds.
async fn opened(
    state: &AppState,
    headers: &HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let db = state.db();
    let admission = admitted(
        state,
        header_text(headers, AUTHORIZATION),
        one_proof(headers),
        unix_now(),
        |hash| async move { tokens::find_token_by_hash(db, &hash).await },
        |user, jkt| async move { device_keys::instance_of_key(user, db, &jkt).await },
    )
    .await?;
    let upgrade = upgrade.map_err(|_| ApiError::INVALID_REQUEST)?;
    let registered = state.connections().register(admission.instance)?;
    let seat = Seat {
        state: state.clone(),
        instance: admission.instance,
        connection: registered.connection,
    };
    Ok(upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| served(socket, seat, registered, admission)))
}

/// `GET` [`SOCKET_PATH`].
///
/// The upgrade is taken as a `Result` so that this runs for a request that
/// is not one. Refused by the extractor itself, an upgrade with no
/// credentials would be told what was wrong with its WebSocket headers
/// before it was told it is nobody, and one sent without those headers would
/// be told it is not a WebSocket and never asked who it is.
pub async fn socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    match opened(&state, &headers, upgrade).await {
        Ok(switching) => switching,
        Err(refused) => refusal(refused),
    }
}

/// What one reading of the WebSocket is to a session. A close, the end of
/// the stream and an error are the three ways a peer is gone;
/// [`Received::Closed`] says why they are one. A message over
/// [`MAX_MESSAGE_BYTES`] arrives here as such an error.
fn heard(message: Option<Result<Message, axum::Error>>) -> Received {
    match message {
        Some(Ok(Message::Text(text))) => Received::Text(text.as_str().to_owned()),
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => Received::Alive,
        Some(Ok(Message::Binary(_))) => Received::Unsupported,
        Some(Ok(Message::Close(_)) | Err(_)) | None => Received::Closed,
    }
}

/// Through `recv` and `send`, the WebSocket's inherent methods, and not its
/// `Stream` and `Sink`: naming those traits takes `futures-util`, which is
/// in the build and is not a dependency of this crate.
#[async_trait]
impl Transport for WebSocket {
    /// `recv` is one `next` of a stream, which keeps what it has read when
    /// its future is dropped: the property [`Transport::received`] asks for.
    /// A peer's ping is answered by the library and still surfaces here.
    async fn received(&mut self) -> Received {
        heard(self.recv().await)
    }

    async fn send_text(&mut self, text: String) -> Result<(), PeerGone> {
        self.send(Message::Text(Utf8Bytes::from(text)))
            .await
            .map_err(|_| PeerGone)
    }

    async fn send_ping(&mut self) -> Result<(), PeerGone> {
        self.send(Message::Ping(Bytes::new()))
            .await
            .map_err(|_| PeerGone)
    }

    /// Sends the close frame and drops the connection, without waiting for
    /// the peer's own close in answer. Waiting needs a bound, since a peer
    /// could withhold its answer for as long as it liked, and no such bound
    /// has been decided. What that costs: a peer still sending when this is
    /// called can see its connection reset before it has read `code`.
    async fn close(mut self, code: u16) {
        let frame = CloseFrame {
            code,
            reason: Utf8Bytes::from_static(""),
        };
        let _ = self.send(Message::Close(Some(frame))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::testing::lazy_state;
    use crate::auth::RelyingParty;
    use crate::pairing::grant::FreshPair;
    use crate::pairing::testing::{claims, MemoryFamily, SoftDeviceKey};
    use crate::socket::connections::Closing;
    use serde_json::json;

    const NOW: u64 = 1_700_000_000;
    const SOCKET_URL: &str = "https://hub.example/socket";

    const INVALID_TOKEN: ApiError = ApiError::new(StatusCode::UNAUTHORIZED, "invalid_token");

    #[test]
    fn every_credential_refusal_is_the_one_401_and_a_full_window_is_503() {
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
            assert_eq!(proof_refused(fault), INVALID_TOKEN, "{fault:?}");
        }
        for refusal in [
            AccessRefusal::NotAccess,
            AccessRefusal::KeyMismatch,
            AccessRefusal::Retired,
            AccessRefusal::Expired,
        ] {
            assert_eq!(token_refused(refusal), INVALID_TOKEN, "{refusal:?}");
        }
        assert_eq!(
            proof_not_remembered(RememberRefusal::Replayed),
            INVALID_TOKEN
        );
        assert_eq!(
            proof_not_remembered(RememberRefusal::Full),
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "too_many_requests")
        );
    }

    #[test]
    fn a_token_is_read_from_one_spelling_of_the_header_and_the_scheme_in_any_case() {
        let token = TokenSecret::generate().unwrap();
        let text = token.text();
        let read = |authorization: String| {
            presented_token(&authorization).map(|token| (token.text.to_owned(), token.hash))
        };

        for scheme in ["DPoP", "dpop", "DPOP"] {
            assert_eq!(
                read(format!("{scheme} {text}")),
                Some((text.clone(), token.hash())),
                "{scheme}"
            );
        }
        for unreadable in [
            text.clone(),
            format!("Bearer {text}"),
            format!("DPoP{text}"),
            format!("DPoP  {text}"),
            format!("DPoP {text} "),
            format!("DPoP {text}="),
            format!("DPoP {}", &text[1..]),
            "DPoP ".to_owned(),
            "DPoP".to_owned(),
            String::new(),
        ] {
            assert!(read(unreadable.clone()).is_none(), "{unreadable:?}");
        }
    }

    #[test]
    fn a_401_carries_the_dpop_challenge_and_no_other_refusal_does() {
        let challenges = |error: ApiError| {
            let response = refusal(error);
            let challenges: Vec<_> = response
                .headers()
                .get_all(WWW_AUTHENTICATE)
                .iter()
                .cloned()
                .collect();
            (response.status(), challenges)
        };

        assert_eq!(
            challenges(ApiError::INVALID_TOKEN),
            (
                StatusCode::UNAUTHORIZED,
                vec![HeaderValue::from_static("DPoP")]
            )
        );
        for (other, status) in [
            (ApiError::INVALID_REQUEST, StatusCode::BAD_REQUEST),
            (ApiError::TOO_MANY_REQUESTS, StatusCode::SERVICE_UNAVAILABLE),
            (
                ApiError::TOO_MANY_CONNECTIONS,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (ApiError::INTERNAL, StatusCode::INTERNAL_SERVER_ERROR),
        ] {
            assert_eq!(challenges(other), (status, vec![]), "{other:?}");
        }
    }

    #[test]
    fn a_websocket_message_is_text_a_sign_of_life_unsupported_or_the_peer_gone() {
        let close = CloseFrame {
            code: 1000,
            reason: Utf8Bytes::from_static("bye"),
        };

        assert_eq!(
            heard(Some(Ok(Message::Text(Utf8Bytes::from_static("{}"))))),
            Received::Text("{}".to_owned())
        );
        for alive in [
            Message::Ping(Bytes::new()),
            Message::Pong(Bytes::from_static(b"1")),
        ] {
            assert_eq!(heard(Some(Ok(alive))), Received::Alive);
        }
        assert_eq!(
            heard(Some(Ok(Message::Binary(Bytes::from_static(b"{}"))))),
            Received::Unsupported
        );
        for gone in [
            Some(Ok(Message::Close(Some(close)))),
            Some(Ok(Message::Close(None))),
            Some(Err(axum::Error::new(PeerGone))),
            None,
        ] {
            assert_eq!(heard(gone), Received::Closed);
        }
    }

    /// The store is never dialled: the two closures stand where it would be.
    fn state() -> AppState {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        };
        lazy_state("postgres://hub@db.invalid/hub", rp).unwrap()
    }

    #[tokio::test]
    async fn a_store_that_fails_either_read_is_500_and_the_same_reads_answered_admit() {
        let state = state();
        let key = SoftDeviceKey::new();
        let pair = FreshPair::generate().unwrap();
        let family = MemoryFamily::paired(&pair, &key.thumbprint(), NOW);
        let token = pair.access.text();
        let authorization = format!("DPoP {token}");
        let instance = InstanceId::generate();
        let proof = |jti: &str| {
            let mut claims = claims("GET", SOCKET_URL, NOW, jti);
            claims["ath"] = json!(base64url_encode(&Sha256::digest(token.as_bytes())));
            key.proof_with(&key.header(), &claims)
        };
        let found = |hash: TokenHash| {
            let row = family.find(&hash).cloned();
            async { Ok(row) }
        };
        let failed = || StoreError::TooLarge { bytes: 0 };

        let admission = admitted(
            &state,
            Some(&authorization),
            Some(&proof("jti-1")),
            NOW,
            found,
            |_, _| async { Ok(Some(instance)) },
        )
        .await;
        assert_eq!(admission.map(|admission| admission.instance), Ok(instance));

        let no_token = admitted(
            &state,
            Some(&authorization),
            Some(&proof("jti-2")),
            NOW,
            |_| async { Err(failed()) },
            |_, _| async { Ok(Some(instance)) },
        )
        .await;
        assert_eq!(no_token, Err(ApiError::INTERNAL));

        let no_instance = admitted(
            &state,
            Some(&authorization),
            Some(&proof("jti-3")),
            NOW,
            found,
            |_, _| async { Err(failed()) },
        )
        .await;
        assert_eq!(no_instance, Err(ApiError::INTERNAL));
    }

    /// A registration that was released and one that was displaced are told
    /// apart by what their `close` answers, so the later registration here
    /// makes the answer come at once either way.
    #[tokio::test]
    async fn a_seat_that_is_dropped_gives_up_its_entry_and_only_its_own() {
        let state = state();
        let instance = InstanceId::generate();
        let seated = |registered: &Registered| Seat {
            state: state.clone(),
            instance,
            connection: registered.connection,
        };

        let mut released = state.connections().register(instance).unwrap();
        drop(seated(&released));
        let mut displaced = state.connections().register(instance).unwrap();
        assert_eq!(released.close.closing().await, Closing::ShuttingDown);

        let stale = seated(&displaced);
        let mut successor = state.connections().register(instance).unwrap();
        assert_eq!(displaced.close.closing().await, Closing::Displaced);
        drop(stale);
        let _later = state.connections().register(instance).unwrap();
        assert_eq!(successor.close.closing().await, Closing::Displaced);
    }
}
