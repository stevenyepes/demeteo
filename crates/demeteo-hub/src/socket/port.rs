//! What a session asks of the connection it runs over, and of the rows it
//! writes.
//!
//! The session is written against [`Transport`] and not against a WebSocket
//! so that a test can stand where the peer does without a listener or a
//! client library: the Hub has no WebSocket client among its dependencies,
//! and adding one is a decision this module was shaped to make unnecessary.

use super::connections::ConnectionId;
use super::requests::OutcomeChange;
use crate::store::requests::{NewRequest, QueuedRequest, RequestId, RequestState};
use crate::store::{InstanceId, StoreError};
use async_trait::async_trait;
use demeteo_hub_protocol::{Hello, RunEvent, Scope, Snapshot};
use uuid::Uuid;

/// One thing the peer did, as far as a session can tell them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    Text(String),
    /// A frame that carries nothing for the session and still shows the peer
    /// is there: a pong, and any other control frame the transport surfaces.
    Alive,
    /// A binary frame. Its bytes are not carried: the protocol has no use for
    /// one, so there is nothing a session could do with them but refuse.
    Unsupported,
    /// The peer closed, or the connection failed. A session cannot act on the
    /// difference, so a transport does not report it.
    Closed,
}

/// A send that found the connection already gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the peer is gone")]
pub struct PeerGone;

#[async_trait]
pub trait Transport: Send {
    /// The next thing the peer did. An implementation must lose nothing when
    /// the returned future is dropped unresolved: a session waits on this
    /// beside its timers and its wake-ups, and whichever loses is dropped.
    async fn received(&mut self) -> Received;

    /// Waited for under the ends a read is, so a session may drop the future
    /// unresolved, with the frame part-written. It sends nothing more after
    /// that but the [`close`](Self::close), so an implementation owes no
    /// frame left whole and no resumption of one.
    async fn send_text(&mut self, text: String) -> Result<(), PeerGone>;

    async fn send_ping(&mut self) -> Result<(), PeerGone>;

    /// Says why, and ends the connection. It takes the transport so that
    /// nothing can be sent after the close, and reports no failure: a peer
    /// already gone is the state a close asks for.
    async fn close(self, code: u16);
}

/// The rows of one instance of one tenant. Both are fixed when a store is
/// made, from the admission and from nothing a peer sent afterwards, so no
/// method takes either: a frame has no argument through which to name
/// another instance's rows.
///
/// A store decides nothing. Where a write depends on a verdict, the caller
/// hands over the verdict.
#[async_trait]
pub trait SocketStore: Send + Sync {
    /// The instance the store is bound to, for what has to name it beside
    /// its rows: an envelope is signed for this one because the row lands
    /// here.
    fn instance(&self) -> InstanceId;

    /// Writes what `hello` reported onto the instance's row and makes
    /// `connection` the one it is present through, as of `now` (Unix
    /// seconds). Returns the instance's event cursor: the highest event
    /// offset stored for it, `0` when none is.
    async fn greeted(
        &self,
        hello: &Hello,
        connection: ConnectionId,
        now: u64,
    ) -> Result<u64, StoreError>;

    /// Stores `event` in the mirror under its Feature and offset, or drops it
    /// when a row is already there, and writes the instance's cursor, as one
    /// step: the cursor never names an event the mirror lacks. The cursor
    /// written and returned is `decide(cursor before, event.offset)`, asked
    /// for a dropped event as for a stored one. What an ack may name is
    /// [`cursor::advanced`](super::cursor::advanced)'s to say, and a store
    /// compares no two offsets.
    ///
    /// The row holds `event` serialised. A store is never handed the frame,
    /// so it has no text of the peer's to keep.
    async fn ingested(
        &self,
        event: &RunEvent,
        decide: fn(u64, u64) -> u64,
    ) -> Result<u64, StoreError>;

    /// Makes `snapshot` the one the instance has, as of `now` (Unix seconds),
    /// in place of whatever it had: the row is replaced whole and nothing of
    /// an earlier snapshot is merged into it. It is filed under
    /// [`SNAPSHOT_KIND`](super::SNAPSHOT_KIND), and serialised from the parsed
    /// value as an event is.
    async fn snapshot_replaced(&self, snapshot: &Snapshot, now: u64) -> Result<(), StoreError>;

    /// The scopes the instance's row holds: what its last `hello` reported,
    /// and until it has sent one, what its pairing approved. A report, as
    /// the [module root](super) says of everything a socket carries.
    async fn granted_scopes(&self) -> Result<Vec<Scope>, StoreError>;

    /// Adds `request` to the end of the instance's queue, as `queued`. The
    /// envelope is stored complete, signature included, and nothing is
    /// derived from it here: its columns are the ones `request` names.
    async fn request_inserted(&self, request: NewRequest) -> Result<(), StoreError>;

    /// [`request_inserted`](Self::request_inserted), and the binding of
    /// every id of `attachments` to the request, as one step: each must be a
    /// row of this instance that is bound to no request and whose
    /// `expires_at` is after `now` (Unix seconds), and each takes the
    /// request's `id` and `expires_at`. If any is not, nothing is written,
    /// neither the request nor a binding, and the answer is
    /// [`StoreError::AttachmentUnavailable`]. `attachments` must name no id
    /// twice; one that does is refused the same way.
    ///
    /// Which ids a dispatch may name is the caller's verdict, reached on a
    /// read of the rows before it signs; this is where that read is held to
    /// what the rows are by the time of the write.
    ///
    /// The default is a store that holds no attachment rows, so it has none
    /// of these to bind. `PostgresStore` and `MemoryStore` hold them and
    /// override it.
    async fn request_inserted_binding(
        &self,
        request: NewRequest,
        attachments: &[Uuid],
        now: u64,
    ) -> Result<(), StoreError> {
        let _ = now;
        if !attachments.is_empty() {
            return Err(StoreError::AttachmentUnavailable);
        }
        self.request_inserted(request).await
    }

    /// The instance's requests that are still `queued`, oldest first, the
    /// overdue among them: whether one may be sent is
    /// [`delivery_verdict`](super::requests::delivery_verdict)'s to say and
    /// no clock is read here.
    ///
    /// A row whose envelope does not parse is in the list as
    /// [`Unreadable`](crate::store::requests::QueuedEnvelope::Unreadable)
    /// and does not fail it: the rows behind it are still read, and its own
    /// expiry can still be judged.
    async fn queued_requests(&self) -> Result<Vec<QueuedRequest>, StoreError>;

    /// Counts one more delivery of the request, made at `now` (Unix
    /// seconds), whatever its status. Called once its frame has been handed
    /// to the transport and never before.
    async fn request_sent(&self, id: RequestId, now: u64) -> Result<(), StoreError>;

    /// Marks the request `expired` if it is still `queued`. One that was
    /// answered since the queue was read keeps its outcome, and that is not
    /// a failure.
    async fn request_expired(&self, id: RequestId) -> Result<(), StoreError>;

    /// Shows `decide` the instance's request `id` as it stands and writes
    /// what it answers: for [`OutcomeChange::Record`] the status `answered`,
    /// that outcome and `now` (Unix seconds) together, with every attachment
    /// bound to the request deleted in the same write, and for
    /// [`OutcomeChange::Keep`] nothing. `false` when the instance has no
    /// request under `id`; `decide` is then not asked and nothing is written.
    ///
    /// `id` is a value a peer sent. It is looked for among this instance's
    /// requests and nowhere else, so one that is another instance's request
    /// is `false` exactly as one that is nobody's is.
    ///
    /// The reading and the write are one step: of two results for one
    /// request, the second is shown what the first recorded.
    async fn outcome_recorded(
        &self,
        id: RequestId,
        decide: &(dyn Fn(RequestState) -> OutcomeChange + Sync),
        now: u64,
    ) -> Result<bool, StoreError>;

    /// Marks the instance absent as of `now` if `connection` is still the
    /// one it is present through. A connection that was displaced changes
    /// nothing, and that is not a failure.
    async fn departed(&self, connection: ConnectionId, now: u64) -> Result<(), StoreError>;
}
