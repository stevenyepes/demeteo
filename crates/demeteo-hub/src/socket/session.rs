//! One instance's session: its frames read one at a time, each answered
//! before the next is read.
//!
//! [`next`] decides what a frame does and [`run`] does it. Nothing between a
//! frame and a store call is decided in `run`, so every refusal below can be
//! reached from a test that has neither a transport nor a store.
//!
//! A frame is processed to completion before the next is read. That is the
//! back-pressure: a peer that sends faster than the store writes waits in its
//! own send buffer, and there is no queue here for it to fill.
//!
//! Requests go the other way and wait for no frame: the queue is read after
//! the `hello` and again whenever the session is woken, and [`outbound`]
//! says what becomes of each request in it.
//!
//! A session also ends for reasons no frame carries: its peer went quiet,
//! the token it was admitted with expired, a later connection of the same
//! instance took its place, or the Hub is stopping. Each is an [`Ended`], and
//! [`close_code`] is the one place a cause is given the code its peer reads.

use super::connections::{Close, Closing, ConnectionId, Registered};
use super::cursor;
use super::frame::{self, FrameError, Inbound};
use super::handler::Admission;
use super::port::{PeerGone, Received, SocketStore, Transport};
use super::requests::{delivery_verdict, named_request, outcome_change, Delivery};
use crate::auth::challenge::base64url_encode;
use crate::store::requests::{QueuedEnvelope, RequestId};
use demeteo_hub_protocol::{
    CursorAck, Hello, RequestOutcome, RunEvent, SignedRequest, Snapshot, PROTOCOL_VERSION,
};
use std::collections::HashSet;
use std::future::Future;
use std::time::Duration;
use tokio::time::{sleep, timeout};

/// RFC 6455 §7.4.1, for an endpoint that is going away.
const GOING_AWAY: u16 = 1001;

/// RFC 6455 §7.4.1, for a peer that broke the protocol.
const POLICY_VIOLATION: u16 = 1008;
/// RFC 6455 §7.4.1, for a Hub that could not do what a valid frame asked. It
/// tells the peer that coming back and sending the same thing may succeed.
const INTERNAL_ERROR: u16 = 1011;
/// From the range RFC 6455 §7.4.2 leaves to applications, as is
/// [`DISPLACED`]. The peer is to refresh its token and connect again.
const TOKEN_EXPIRED: u16 = 4001;
/// The peer is told apart from one whose token expired because it must not
/// do the same thing about it: an instance that reconnects on being displaced
/// displaces its successor, which reconnects in its turn.
const DISPLACED: u16 = 4002;

/// How often a session pings its peer, and how long it goes on hearing
/// nothing before it takes the peer for gone.
///
/// Handed to [`run`], which reads neither
/// [`PING_INTERVAL_SECS`](super::PING_INTERVAL_SECS) nor
/// [`PONG_DEADLINE_SECS`](super::PONG_DEADLINE_SECS): the workspace builds
/// `tokio` without `test-util`, so a test cannot pause a clock and passes
/// milliseconds where the route passes those two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub ping_interval: Duration,
    /// Counted from the last frame of any kind and not from the last pong:
    /// a peer that is sending events is there, whatever became of a pong.
    /// It is also how long one write to the peer may stay pending, counted
    /// from when it began.
    pub pong_deadline: Duration,
}

/// What a peer did that the protocol does not allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    /// A message that is not `hello`, sent first.
    BeforeHello,
    /// A change of scopes is reported by reconnecting, so a second `hello`
    /// has nothing to mean.
    SecondHello,
    Binary,
    Frame(FrameError),
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The peer closed, the connection failed, or a send found it gone.
    PeerGone,
    /// Nothing came from the peer within [`Limits::pong_deadline`], or a
    /// write to it stayed pending that long: a peer that reads nothing is
    /// as gone as one that sends nothing.
    Silent,
    /// The clock reached the expiry of the token the connection was admitted
    /// with.
    TokenExpired,
    /// The registry asked the session to end.
    Asked(Closing),
    Refused(Violation),
    StoreFailed,
    /// A frame of the Hub's own did not serialise. No value of the types it
    /// sends can cause that; this is where the `Err` goes in place of a
    /// panic.
    Unencodable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Unannounced,
    Greeted,
}

#[derive(Debug, PartialEq)]
enum Next {
    /// The peer is there and said nothing a session acts on.
    Wait,
    Greet(Hello),
    Ingest(RunEvent),
    Replace(Snapshot),
    Record(RequestId, RequestOutcome),
    /// A result whose `request_id` is not the text of any request's id.
    Unmatched,
    End(Ended),
}

/// What the session does about `received`, given whether the peer has sent
/// its `hello`.
///
/// The second `match` has no wildcard, and an arm per message once the peer
/// is greeted: a message the protocol gains has to be placed here before the
/// crate compiles, and giving one of them a handler is a change to its arm
/// alone.
fn next(phase: Phase, received: Received) -> Next {
    let text = match received {
        Received::Text(text) => text,
        Received::Alive => return Next::Wait,
        Received::Unsupported => return Next::End(Ended::Refused(Violation::Binary)),
        Received::Closed => return Next::End(Ended::PeerGone),
    };
    let message = match frame::parse(&text) {
        Ok(message) => message,
        Err(error) => return Next::End(Ended::Refused(Violation::Frame(error))),
    };
    match (phase, message) {
        (Phase::Unannounced, Inbound::Hello(hello)) => Next::Greet(hello),
        (
            Phase::Unannounced,
            Inbound::RunEvent(_) | Inbound::Snapshot(_) | Inbound::RequestResult(_),
        ) => Next::End(Ended::Refused(Violation::BeforeHello)),
        (Phase::Greeted, Inbound::Hello(_)) => Next::End(Ended::Refused(Violation::SecondHello)),
        (Phase::Greeted, Inbound::RunEvent(event)) => Next::Ingest(event),
        (Phase::Greeted, Inbound::Snapshot(snapshot)) => Next::Replace(snapshot),
        (Phase::Greeted, Inbound::RequestResult(result)) => {
            match named_request(&result.request_id) {
                Some(id) => Next::Record(id, result.outcome),
                None => Next::Unmatched,
            }
        }
    }
}

/// The code the peer is closed with, or `None` when there is no peer left to
/// tell.
///
/// A peer that went quiet is one of those. The protocol gives silence no
/// code, and the connection is dropped without one: a peer that was there
/// after all sees its socket fail and reconnects, which is what it would do
/// about any code it could be sent.
fn close_code(ended: Ended) -> Option<u16> {
    match ended {
        Ended::PeerGone | Ended::Silent => None,
        Ended::TokenExpired => Some(TOKEN_EXPIRED),
        Ended::Asked(Closing::Displaced) => Some(DISPLACED),
        Ended::Asked(Closing::ShuttingDown) => Some(GOING_AWAY),
        Ended::Refused(_) => Some(POLICY_VIOLATION),
        Ended::StoreFailed | Ended::Unencodable => Some(INTERNAL_ERROR),
    }
}

/// How long a session may still run on a token that expires at
/// `token_expires_at`, or `None` once `now` has reached it. Both are Unix
/// seconds. A token is good strictly before its expiry at admission
/// ([`access_verdict`](crate::pairing::grant::access_verdict)), and a socket
/// is held to the same second.
fn lifetime_left(token_expires_at: u64, now: u64) -> Option<Duration> {
    match token_expires_at.checked_sub(now) {
        None | Some(0) => None,
        Some(seconds) => Some(Duration::from_secs(seconds)),
    }
}

/// How a write to the peer finished, or what it was still pending at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handover {
    Done,
    Failed,
    Asked(Closing),
    TokenRanOut,
    Overdue,
}

/// What a session does after a write: go on, or end for the cause the write
/// met. A write given up on ends the session for what it was waiting beside,
/// exactly as a read that was waiting would have.
fn handed_over(handover: Handover) -> Result<(), Ended> {
    match handover {
        Handover::Done => Ok(()),
        Handover::Failed => Err(Ended::PeerGone),
        Handover::Asked(closing) => Err(Ended::Asked(closing)),
        Handover::TokenRanOut => Err(Ended::TokenExpired),
        Handover::Overdue => Err(Ended::Silent),
    }
}

/// The ends a session waits under, a read or a write: a request to end, the
/// admitted token's expiry, and the deadline.
struct Bounds<C> {
    close: Close,
    clock: C,
    token_expires_at: u64,
    deadline: Duration,
}

impl<C: Fn() -> u64> Bounds<C> {
    fn now(&self) -> u64 {
        (self.clock)()
    }

    fn lifetime_left(&self) -> Option<Duration> {
        lifetime_left(self.token_expires_at, self.now())
    }

    /// Every write of a session's but the close goes through here, so that
    /// none can wait on a peer that stopped reading for longer than a read
    /// would wait on one that stopped sending. Once `send` is dropped
    /// unresolved the session ends, and a close is all it may still send.
    ///
    /// The request to end is looked at ahead of the send, as the read loop
    /// looks at it ahead of a frame: a frame not yet handed over when the
    /// session is asked to end is not sent.
    async fn sent(
        &mut self,
        send: impl Future<Output = Result<(), PeerGone>>,
    ) -> Result<(), Ended> {
        tokio::pin!(send);
        let overdue = sleep(self.deadline);
        tokio::pin!(overdue);
        let handover = loop {
            let Some(left) = self.lifetime_left() else {
                break Handover::TokenRanOut;
            };
            break tokio::select! {
                biased;
                closing = self.close.closing() => Handover::Asked(closing),
                sent = &mut send => match sent {
                    Ok(()) => Handover::Done,
                    Err(PeerGone) => Handover::Failed,
                },
                () = &mut overdue => Handover::Overdue,
                () = sleep(left) => continue,
            };
        };
        handed_over(handover)
    }

    /// The close comes after the departure was written, so giving up on it
    /// costs no write, and it is not cut short by a request to end or by
    /// the token: either is what it reports.
    async fn closed(&self, transport: impl Transport, code: u16) {
        if timeout(self.deadline, transport.close(code)).await.is_err() {
            tracing::debug!(code, "a close was given up on unsent");
        }
    }
}

fn ack(offset: u64) -> Result<String, serde_json::Error> {
    serde_json::to_string(&CursorAck {
        v: PROTOCOL_VERSION,
        offset,
    })
}

async fn acked<C: Fn() -> u64>(
    transport: &mut impl Transport,
    bounds: &mut Bounds<C>,
    cursor: u64,
) -> Result<(), Ended> {
    let ack = ack(cursor).map_err(|_| Ended::Unencodable)?;
    bounds.sent(transport.send_text(ack)).await
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Outbound<'a> {
    Send(&'a SignedRequest),
    Expire,
    Leave,
    /// Left as well, and worth an error: no session can send it, so it waits
    /// for its expiry or a hand.
    Unreadable,
}

/// What a session does about one request of its queue, given the verdict on
/// its expiry, its envelope as it was read, and whether this connection has
/// already carried it.
///
/// A request stays `queued` until it is answered, so a session that is woken
/// reads back the ones it has sent. They are left alone: the peer of this
/// connection has them, and a second copy would only be refused as replayed.
/// What may have been lost is a frame on a connection that then dropped, and
/// that one is sent again by the next session, which starts with nothing
/// sent.
///
/// Expiry is looked at first, so a request sent here and still unanswered
/// when it runs out is marked on the next reading and does not wait for a
/// reconnect to be judged. An envelope that did not parse is judged the same
/// way: its expiry is a column of its own, so it is marked like any other
/// once it is overdue, and until then it is left and the requests behind it
/// go out.
fn outbound(
    verdict: Delivery,
    envelope: &QueuedEnvelope,
    sent_on_this_connection: bool,
) -> Outbound<'_> {
    match (verdict, envelope, sent_on_this_connection) {
        (Delivery::Expire, _, _) => Outbound::Expire,
        (Delivery::Deliver, QueuedEnvelope::Unreadable, _) => Outbound::Unreadable,
        (Delivery::Deliver, QueuedEnvelope::Readable(_), true) => Outbound::Leave,
        (Delivery::Deliver, QueuedEnvelope::Readable(envelope), false) => Outbound::Send(envelope),
    }
}

/// Reads the instance's queue and deals with each request in it, in queue
/// order, as [`outbound`] says. `sent` is what this connection has carried
/// so far.
///
/// The clock is read once for each request, when its turn comes and not
/// once for the queue: the one before it may have been slow to send.
///
/// A request is counted as sent after its frame was handed to the transport
/// and never before. A session that ends between the two leaves a request
/// that went out and was not counted, which the next session sends again
/// under the same id; the other order counts a delivery that never happened.
/// A send that failed or was given up on counts nothing. The reading, the
/// send and the count are three steps and not one transaction around the
/// send, which a peer that is slow to read would hold open over the rows of
/// the queue.
///
/// What is sent is the stored envelope, serialised. Nothing here can sign,
/// so nothing here can send a request other than the one that was queued.
async fn delivered<T, S, C>(
    transport: &mut T,
    store: &S,
    connection: ConnectionId,
    bounds: &mut Bounds<C>,
    sent: &mut HashSet<RequestId>,
) -> Result<(), Ended>
where
    T: Transport,
    S: SocketStore,
    C: Fn() -> u64,
{
    let store_failed = |error| {
        tracing::error!(%error, ?connection, "the request queue could not be read or marked");
        Ended::StoreFailed
    };
    for request in store.queued_requests().await.map_err(store_failed)? {
        let now = bounds.now();
        let verdict = delivery_verdict(request.expires_at, now);
        match outbound(verdict, &request.envelope, sent.contains(&request.id)) {
            Outbound::Leave => {}
            Outbound::Unreadable => {
                tracing::error!(
                    request = %base64url_encode(&request.id),
                    ?connection,
                    "a queued request's envelope could not be read; it is left queued and unsent"
                );
            }
            Outbound::Expire => {
                store
                    .request_expired(request.id)
                    .await
                    .map_err(store_failed)?;
            }
            Outbound::Send(envelope) => {
                let frame = serde_json::to_string(envelope).map_err(|_| Ended::Unencodable)?;
                bounds.sent(transport.send_text(frame)).await?;
                sent.insert(request.id);
                store
                    .request_sent(request.id, now)
                    .await
                    .map_err(store_failed)?;
            }
        }
    }
    Ok(())
}

/// Runs the session of one admitted connection to its end. `clock` gives
/// Unix seconds and is asked at each write, so a test moves time by moving
/// what it returns. Of `admission` only the token's expiry is read: the
/// tenant and the instance are already in `store`.
///
/// Everything a session waits for it waits for from its first moment, a
/// `hello` included. A connection that was admitted and then said nothing is
/// as much a socket a shutdown would wait on as one that greeted.
///
/// The token's expiry is looked at before every wait, against `clock`, and
/// the wait is bounded by what was left of it at that reading. So a session
/// with nothing to read still ends in the second its token does, and does
/// not wait for a ping or a frame to look again.
///
/// A write to the peer is waited for under the same ends as a read, and
/// under [`Limits::pong_deadline`] counted from its start: a peer that keeps
/// its connection up and stops reading would otherwise hold the session in
/// a send for as long as it liked, past its token, the deadline and a
/// request to end. A write still pending at one of those ends the session
/// for it, as [`handed_over`] says.
///
/// An ack is sent after the write it reports has returned and never before:
/// the instance resumes from what it was acked, so an ack ahead of the row
/// is an event it will not send again.
///
/// A snapshot is answered with nothing. The protocol has no frame that
/// acknowledges one, and the instance keeps no cursor for them to resume
/// from: it sends the whole of its state after every `hello` and on every
/// change, so one that was lost is made good by the next.
///
/// A result is answered with nothing either, and is recorded as
/// [`outcome_change`] says. One that names no request of the instance, or
/// whose id is not an id at all, changes no row and does not end the session.
/// The protocol has no frame that acknowledges a result, so the instance is
/// waiting on nothing a close could tell it; closing would only make the
/// events and results behind this one wait on a reconnect. What is logged of
/// such a result is that it came: its `request_id` is the peer's text.
///
/// The queue is read after the `hello` was acked and again on every wake-up,
/// and never before the `hello`: a wake-up that comes first is kept until
/// the peer has announced itself. Nothing is taken from the wake-up but the
/// fact of it ([`Connections`](super::connections::Connections) says why),
/// and it is looked for ahead of the peer's next frame. A peer that never
/// stops sending would otherwise keep its own requests waiting, and the
/// reverse cannot happen: any number of wake-ups are one, and one costs a
/// single reading of the queue.
///
/// Once `greeted` has succeeded the session ends with `departed`, whatever
/// ended it, and before the close is sent: whoever sees the close then reads
/// a row that already says absent, where the other order leaves a moment in
/// which a closed instance still reads as present. The close is waited for
/// for [`Limits::pong_deadline`] at most. The registration is
/// counted until both have returned, so a stop that waits on
/// [`Connections::drained`](super::connections::Connections::drained) waits
/// for the departure and the close and not only for the loop. A session
/// whose task is dropped while it waits, which is what a stop does to one
/// that outlasts its bound, writes no departure;
/// [`Instance::connection_id`](crate::store::instances::Instance::connection_id)
/// says what that leaves behind.
pub async fn run<T, S>(
    mut transport: T,
    store: S,
    connection: Registered,
    admission: Admission,
    clock: impl Fn() -> u64,
    limits: Limits,
) -> Ended
where
    T: Transport,
    S: SocketStore,
{
    let Registered {
        connection,
        wake,
        close,
        counted,
    } = connection;
    let mut bounds = Bounds {
        close,
        clock,
        token_expires_at: admission.token_expires_at,
        deadline: limits.pong_deadline,
    };
    let mut phase = Phase::Unannounced;
    let mut sent = HashSet::new();
    let ping = sleep(limits.ping_interval);
    let silence = sleep(limits.pong_deadline);
    tokio::pin!(ping, silence);
    let ended = loop {
        let Some(left) = bounds.lifetime_left() else {
            break Ended::TokenExpired;
        };
        // In this order and not at random: a frame that arrived while a
        // write overran the deadline is taken before the deadline is, and
        // counts as the peer having been heard.
        let received = tokio::select! {
            biased;
            closing = bounds.close.closing() => break Ended::Asked(closing),
            () = wake.woken(), if phase == Phase::Greeted => {
                if let Err(ended) =
                    delivered(&mut transport, &store, connection, &mut bounds, &mut sent).await
                {
                    break ended;
                }
                continue;
            }
            received = transport.received() => received,
            () = &mut silence => break Ended::Silent,
            () = &mut ping => {
                if let Err(ended) = bounds.sent(transport.send_ping()).await {
                    break ended;
                }
                ping.set(sleep(limits.ping_interval));
                continue;
            }
            () = sleep(left) => continue,
        };
        silence.set(sleep(limits.pong_deadline));
        match next(phase, received) {
            Next::Wait => {}
            Next::End(ended) => break ended,
            Next::Greet(hello) => {
                let cursor = match store.greeted(&hello, connection, bounds.now()).await {
                    Ok(cursor) => cursor,
                    Err(error) => {
                        tracing::error!(%error, ?connection, "a hello could not be recorded");
                        break Ended::StoreFailed;
                    }
                };
                phase = Phase::Greeted;
                if let Err(ended) = acked(&mut transport, &mut bounds, cursor).await {
                    break ended;
                }
                if let Err(ended) =
                    delivered(&mut transport, &store, connection, &mut bounds, &mut sent).await
                {
                    break ended;
                }
            }
            Next::Ingest(event) => {
                let cursor = match store.ingested(&event, cursor::advanced).await {
                    Ok(cursor) => cursor,
                    Err(error) => {
                        tracing::error!(
                            %error,
                            ?connection,
                            offset = event.offset,
                            "an event could not be stored"
                        );
                        break Ended::StoreFailed;
                    }
                };
                if let Err(ended) = acked(&mut transport, &mut bounds, cursor).await {
                    break ended;
                }
            }
            Next::Replace(snapshot) => {
                if let Err(error) = store.snapshot_replaced(&snapshot, bounds.now()).await {
                    tracing::error!(%error, ?connection, "a snapshot could not be stored");
                    break Ended::StoreFailed;
                }
            }
            Next::Unmatched => {
                tracing::warn!("a request result named nothing that is a request id");
            }
            Next::Record(id, outcome) => {
                let decide = |state| outcome_change(state, outcome);
                match store.outcome_recorded(id, &decide, bounds.now()).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!("a request result named no request of this instance");
                    }
                    Err(error) => {
                        tracing::error!(%error, ?connection, "an outcome could not be recorded");
                        break Ended::StoreFailed;
                    }
                }
            }
        }
    };

    if phase == Phase::Greeted {
        if let Err(error) = store.departed(connection, bounds.now()).await {
            tracing::error!(%error, ?connection, "a departure could not be recorded");
        }
    }
    if let Some(code) = close_code(ended) {
        bounds.closed(transport, code).await;
    }
    drop(counted);
    ended
}

#[cfg(test)]
mod tests {
    use super::*;
    use demeteo_hub_protocol::{
        CancelFeature, RefusalReason, RequestPayload, RequestResult, RunEventKind, RunEventPayload,
    };

    fn text<T: serde::Serialize>(message: &T) -> Received {
        Received::Text(serde_json::to_string(message).unwrap())
    }

    fn hello() -> Hello {
        Hello {
            v: PROTOCOL_VERSION,
            install_id: "inst-1".to_owned(),
            app_version: "1.4.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            scopes: Vec::new(),
        }
    }

    fn refused(violation: Violation) -> Next {
        Next::End(Ended::Refused(violation))
    }

    #[test]
    fn only_a_hello_is_taken_first_and_only_once() {
        assert_eq!(
            next(Phase::Unannounced, text(&hello())),
            Next::Greet(hello())
        );
        assert_eq!(
            next(Phase::Greeted, text(&hello())),
            refused(Violation::SecondHello)
        );

        let result = text(&RequestResult {
            v: PROTOCOL_VERSION,
            request_id: "r".to_owned(),
            outcome: RequestOutcome::Accepted,
        });
        assert_eq!(
            next(Phase::Unannounced, result),
            refused(Violation::BeforeHello)
        );
    }

    #[test]
    fn a_result_is_recorded_under_the_id_it_spells_and_one_that_spells_none_ends_nothing() {
        let refused_as_replayed = RequestOutcome::Refused {
            reason: RefusalReason::Replayed,
        };
        let result = |request_id: &str| RequestResult {
            v: PROTOCOL_VERSION,
            request_id: request_id.to_owned(),
            outcome: refused_as_replayed,
        };
        assert_eq!(
            next(Phase::Greeted, text(&result("BwcHBwcHBwcHBwcHBwcHBw"))),
            Next::Record([7; 16], refused_as_replayed)
        );
        for not_an_id in ["", "req-9b2e41", "BwcHBwcHBwcHBwcHBwcHBw=="] {
            assert_eq!(
                next(Phase::Greeted, text(&result(not_an_id))),
                Next::Unmatched,
                "{not_an_id}"
            );
        }
    }

    #[test]
    fn a_run_event_is_ingested_once_the_peer_is_greeted_and_refused_before() {
        let event = RunEvent {
            v: PROTOCOL_VERSION,
            feature_id: "f-1".to_owned(),
            offset: 7,
            kind: RunEventKind::StepStarted,
            occurred_at: 1_700_000_000,
            payload: RunEventPayload::default(),
        };
        assert_eq!(
            next(Phase::Unannounced, text(&event)),
            refused(Violation::BeforeHello)
        );
        assert_eq!(next(Phase::Greeted, text(&event)), Next::Ingest(event));
    }

    #[test]
    fn a_snapshot_is_stored_once_the_peer_is_greeted_and_refused_before() {
        let snapshot = Snapshot {
            v: PROTOCOL_VERSION,
            projects: Vec::new(),
            workflows: Vec::new(),
            run_shape: Vec::new(),
        };
        assert_eq!(
            next(Phase::Unannounced, text(&snapshot)),
            refused(Violation::BeforeHello)
        );
        assert_eq!(
            next(Phase::Greeted, text(&snapshot)),
            Next::Replace(snapshot)
        );
    }

    #[test]
    fn what_is_not_a_message_ends_the_session_in_either_phase_and_a_pong_does_not() {
        for phase in [Phase::Unannounced, Phase::Greeted] {
            assert_eq!(next(phase, Received::Alive), Next::Wait);
            assert_eq!(next(phase, Received::Closed), Next::End(Ended::PeerGone));
            assert_eq!(
                next(phase, Received::Unsupported),
                refused(Violation::Binary)
            );
            assert_eq!(
                next(phase, Received::Text("{".to_owned())),
                refused(Violation::Frame(FrameError::Unrecognised))
            );
            let of_another_version = Hello { v: 2, ..hello() };
            assert_eq!(
                next(phase, text(&of_another_version)),
                refused(Violation::Frame(FrameError::Version(2)))
            );
        }
    }

    #[test]
    fn a_request_is_sent_once_on_a_connection_and_expired_whether_or_not_it_was_sent() {
        let envelope = SignedRequest {
            v: PROTOCOL_VERSION,
            request_id: "BwcHBwcHBwcHBwcHBwcHBw".to_owned(),
            instance_id: "6f0e4c5a-91f4-4f0c-9a55-3a8f6f0d2b11".to_owned(),
            issued_at: 1_700_000_000,
            expires_at: 1_700_000_600,
            payload: RequestPayload::CancelFeature(CancelFeature {
                feature_id: "f-1".to_owned(),
            }),
            signature: "c2lnbmF0dXJl".to_owned(),
        };
        let readable = QueuedEnvelope::Readable(Box::new(envelope.clone()));
        assert_eq!(
            outbound(Delivery::Deliver, &readable, false),
            Outbound::Send(&envelope)
        );
        assert_eq!(
            outbound(Delivery::Deliver, &readable, true),
            Outbound::Leave
        );
        assert_eq!(
            outbound(Delivery::Expire, &readable, false),
            Outbound::Expire
        );
        assert_eq!(
            outbound(Delivery::Expire, &readable, true),
            Outbound::Expire
        );
    }

    #[test]
    fn an_unreadable_request_is_never_sent_and_is_expired_like_any_other() {
        let unreadable = QueuedEnvelope::Unreadable;
        for sent_on_this_connection in [false, true] {
            assert_eq!(
                outbound(Delivery::Deliver, &unreadable, sent_on_this_connection),
                Outbound::Unreadable
            );
            assert_eq!(
                outbound(Delivery::Expire, &unreadable, sent_on_this_connection),
                Outbound::Expire
            );
        }
    }

    #[test]
    fn a_write_given_up_on_ends_the_session_for_what_it_was_waiting_beside() {
        assert_eq!(handed_over(Handover::Done), Ok(()));
        assert_eq!(handed_over(Handover::Failed), Err(Ended::PeerGone));
        for closing in [Closing::Displaced, Closing::ShuttingDown] {
            assert_eq!(
                handed_over(Handover::Asked(closing)),
                Err(Ended::Asked(closing))
            );
        }
        assert_eq!(handed_over(Handover::TokenRanOut), Err(Ended::TokenExpired));
        assert_eq!(handed_over(Handover::Overdue), Err(Ended::Silent));
    }

    #[test]
    fn a_token_has_no_lifetime_left_from_the_second_it_expires() {
        const EXPIRES: u64 = 1_700_000_900;
        assert_eq!(
            lifetime_left(EXPIRES, EXPIRES - 900),
            Some(Duration::from_secs(900))
        );
        assert_eq!(
            lifetime_left(EXPIRES, EXPIRES - 1),
            Some(Duration::from_secs(1))
        );
        assert_eq!(lifetime_left(EXPIRES, EXPIRES), None);
        assert_eq!(lifetime_left(EXPIRES, EXPIRES + 1), None);
        assert_eq!(lifetime_left(EXPIRES, u64::MAX), None);
        assert_eq!(lifetime_left(0, 0), None);
    }

    #[test]
    fn each_cause_the_hub_ends_a_session_for_has_its_own_close_code() {
        assert_eq!(close_code(Ended::Asked(Closing::ShuttingDown)), Some(1001));
        assert_eq!(close_code(Ended::TokenExpired), Some(4001));
        assert_eq!(close_code(Ended::Asked(Closing::Displaced)), Some(4002));
    }

    #[test]
    fn a_peer_that_is_gone_is_sent_no_close() {
        assert_eq!(close_code(Ended::PeerGone), None);
        assert_eq!(close_code(Ended::Silent), None);
        assert_eq!(
            close_code(Ended::Refused(Violation::BeforeHello)),
            Some(1008)
        );
        assert_eq!(close_code(Ended::StoreFailed), Some(1011));
        assert_eq!(close_code(Ended::Unencodable), Some(1011));
    }
}
