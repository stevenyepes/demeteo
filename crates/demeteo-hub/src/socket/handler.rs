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
//! Admission itself is [`pairing::access`](crate::pairing::access), shared
//! with every other route an instance calls with its access token; this
//! module only says which method and URL the socket's proofs are made for.

use super::connections::{ConnectionId, Registered};
use super::port::{PeerGone, Received, Transport};
use super::postgres::PostgresStore;
use super::session::{self, Limits};
use super::{MAX_MESSAGE_BYTES, PING_INTERVAL_SECS, PONG_DEADLINE_SECS};
use crate::auth::handlers::{header_text, unix_now, ApiError};
use crate::pairing::access::{self, refusal};
use crate::pairing::dpop::Expected;
use crate::pairing::handlers::one_proof;
use crate::routes::AppState;
use crate::store::tokens::{self, TokenHash, TokenRecord};
use crate::store::{device_keys, InstanceId, StoreError, UserId};
use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use axum::response::Response;
use std::future::Future;
use std::time::Duration;

pub use crate::pairing::access::Admission;

/// A proof is made for the URL its request is sent to, so this is also the
/// path the socket has to be mounted at.
pub const SOCKET_PATH: &str = "/socket";
const SOCKET_METHOD: &str = "GET";

/// [`access::admitted`] for an upgrade to [`SOCKET_PATH`].
///
/// `htu` is the `https` spelling, though the client dials `wss`: it is built
/// by [`access::htu`], and there is one spelling of it.
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
    let htu = access::htu(state, SOCKET_PATH);
    let expected = Expected {
        htm: SOCKET_METHOD,
        htu: &htu,
    };
    access::admitted(
        state,
        authorization,
        proof,
        &expected,
        now,
        find_token,
        instance_of_key,
    )
    .await
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
    use crate::socket::connections::Closing;

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

    /// The store is never dialled.
    fn state() -> AppState {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        };
        lazy_state("postgres://hub@db.invalid/hub", rp).unwrap()
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
