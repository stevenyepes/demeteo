//! One connection, from its `Hello` to its end (docs/HUB.md §5 *Events and
//! the cursor*).
//!
//! The desktop keeps no cursor of its own: the first `CursorAck` after
//! `Hello` is where it resumes, so a desktop whose database was reset sends
//! nothing below what the Hub already holds, and a Hub that lost rows is
//! refilled from where it says it is.
//!
//! It reads the desktop's own `run_events` and nothing else, whichever
//! transport ran the Feature a row belongs to.
//!
//! A request the Hub relays is handed to the bound [`HubRequestHandler`] on
//! a task of its own, and its `RequestResult` comes back to the loop to be
//! sent: a start can take as long as a launch and an attachment fetch, and
//! the loop that reads the Hub's pings cannot wait that out.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::ready;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use demeteo_hub_protocol::{CursorAck, RequestHeader, RequestResult};
use serde::Serialize;
use tokio::sync::{mpsc, watch, Mutex, Notify, OwnedMutexGuard};
use tokio::time::{sleep_until, timeout, Instant};

use crate::domain::hub_socket::connection::ConnectionState;
use crate::domain::hub_socket::{
    hello, hub_event, SocketEnd, LIVENESS_DEADLINE_SECS, MAX_UNACKED_EVENTS, PAGE_SIZE,
    SNAPSHOT_DEBOUNCE_SECS, STOP_CLOSE_DEADLINE_SECS,
};
use crate::ports::hub::{
    HubRequestHandler, HubSnapshotSource, HubSocketConnection, SocketGone, SocketReceived,
};
use crate::ports::run_events::{RunEvent, RunEventsPort};
use crate::shared::time::now_ms;

use super::snapshot_send::SnapshotSend;
use super::status::ConnectionCell;

/// RFC 6455 §7.4.1, for an endpoint that is going away.
const GOING_AWAY: u16 = 1001;
/// RFC 6455 §7.4.1: the desktop could not read its own run log. The Hub is
/// told to expect the connection back, which [`crate::domain::hub_socket::after`]
/// makes it.
const INTERNAL_ERROR: u16 = 1011;

/// Where request frames go. Bound after the socket's dependencies are
/// built, because the handler is built over the context that holds the
/// pairing; until then a request goes unanswered and is delivered again on
/// the next connection.
pub type RequestHandlerSlot = Arc<OnceLock<Arc<dyn HubRequestHandler>>>;

/// What one session reads and announces.
pub struct SessionContext {
    pub install_id: String,
    pub app_version: String,
    pub pinned_scopes: Vec<String>,
    /// Whether this session sends events and snapshots at all, decided by
    /// [`crate::domain::hub_socket::sends_events`] when the socket is
    /// started; [`Self::pinned_scopes`] alone decide only the `Hello`. Off
    /// until [`Self::with_relaying`] says otherwise.
    pub relaying: bool,
    pub run_events: Arc<dyn RunEventsPort>,
    /// Fired by [`super::wake::WakingRunEvents`] on every append to
    /// `run_events`; it has to be that decorator's, or an append wakes
    /// nothing and a live row waits for the next connection.
    pub wake: Arc<Notify>,
    pub snapshots: Arc<dyn HubSnapshotSource>,
    /// Fired by [`super::snapshot_wake`]'s decorators on every write to
    /// what [`Self::snapshots`] reads; the same pairing rule as
    /// [`Self::wake`].
    pub snapshot_wake: Arc<Notify>,
    /// How long after a [`Self::snapshot_wake`] the snapshot is sent; see
    /// [`SNAPSHOT_DEBOUNCE_SECS`].
    pub snapshot_debounce: Duration,
    /// How long the Hub may say nothing, of any kind, and how long one send
    /// may stay pending.
    pub liveness: Duration,
    /// How long the `1001` close of a stopped session may stay pending; see
    /// [`STOP_CLOSE_DEADLINE_SECS`].
    pub stop_close: Duration,
    pub requests: RequestHandlerSlot,
    /// Where the session, and the supervisor running it, record how the
    /// connection stands. A fresh, unshared cell until
    /// [`Self::with_connection`] hands it the pairing's.
    pub connection: ConnectionCell,
    answers: Answers,
}

/// Results of requests handled off the session, held by the context rather
/// than one connection: the request's id is spent once it is handled, so a
/// result that missed the connection it arrived on is sent on the next one,
/// where a redelivery would only be refused `replayed`.
///
/// That covers reconnects of one socket and nothing longer. Every
/// `HubPairing::start_socket` builds a fresh context, so a result still queued
/// is dropped by an app restart and by `hub_pair`. A redelivery over the same
/// device key is then answered `replayed`, and the Hub keeps that as the
/// outcome of a request whose run did start. After `hub_disconnect` the next
/// pairing is a new instance, so nothing is redelivered and the request stays
/// unanswered on the old one. Not held for the process instead: after a
/// re-pair it would hand one Hub's results to another.
struct Answers {
    tx: mpsc::UnboundedSender<RequestResult>,
    rx: Arc<Mutex<mpsc::UnboundedReceiver<RequestResult>>>,
}

impl Answers {
    fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }
}

impl SessionContext {
    pub fn new(
        install_id: String,
        app_version: String,
        pinned_scopes: Vec<String>,
        run_events: Arc<dyn RunEventsPort>,
        wake: Arc<Notify>,
        snapshots: Arc<dyn HubSnapshotSource>,
        snapshot_wake: Arc<Notify>,
    ) -> Self {
        Self {
            install_id,
            app_version,
            pinned_scopes,
            relaying: false,
            run_events,
            wake,
            snapshots,
            snapshot_wake,
            snapshot_debounce: Duration::from_secs(SNAPSHOT_DEBOUNCE_SECS),
            liveness: Duration::from_secs(LIVENESS_DEADLINE_SECS),
            stop_close: Duration::from_secs(STOP_CLOSE_DEADLINE_SECS),
            requests: RequestHandlerSlot::default(),
            connection: ConnectionCell::default(),
            answers: Answers::new(),
        }
    }

    pub fn with_relaying(mut self, relaying: bool) -> Self {
        self.relaying = relaying;
        self
    }

    pub fn with_requests(mut self, requests: RequestHandlerSlot) -> Self {
        self.requests = requests;
        self
    }

    pub fn with_connection(mut self, connection: ConnectionCell) -> Self {
        self.connection = connection;
        self
    }

    pub fn with_liveness(mut self, liveness: Duration) -> Self {
        self.liveness = liveness;
        self
    }

    pub fn with_snapshot_debounce(mut self, snapshot_debounce: Duration) -> Self {
        self.snapshot_debounce = snapshot_debounce;
        self
    }

    pub fn with_stop_close(mut self, stop_close: Duration) -> Self {
        self.stop_close = stop_close;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// For [`crate::domain::hub_socket::after`] to judge.
    Ended(SocketEnd),
    /// The stop signal was raised, or its sender dropped: nobody is left to
    /// stop the socket, so it stops.
    Stopped,
}

/// Runs one connection until it ends. The `bool` is whether the Hub's first
/// `CursorAck` arrived, which is what resets the supervisor's backoff.
pub async fn run_session(
    conn: Box<dyn HubSocketConnection>,
    ctx: &SessionContext,
    stop: &mut watch::Receiver<bool>,
) -> (SessionEnd, bool) {
    let mut session = Session {
        conn,
        liveness: ctx.liveness,
        heard_by: Instant::now() + ctx.liveness,
        acked: false,
        requests: ctx.requests.clone(),
        connection: ctx.connection.clone(),
        answered: ctx.answers.tx.clone(),
        answers: ctx.answers.rx.clone().lock_owned().await,
    };
    let exit = match session.run(ctx, stop).await {
        Err(exit) => exit,
        Ok(never) => match never {},
    };
    let acked = session.acked;
    let (code, end, deadline) = match exit {
        Exit::Ended(end) => return (SessionEnd::Ended(end), acked),
        Exit::Stop => (GOING_AWAY, SessionEnd::Stopped, ctx.stop_close),
        Exit::Failed => (
            INTERNAL_ERROR,
            SessionEnd::Ended(SocketEnd::Closed(Some(INTERNAL_ERROR))),
            ctx.liveness,
        ),
    };
    if timeout(deadline, session.conn.close(code)).await.is_err() {
        tracing::warn!(code, "the Hub socket did not close in time");
    }
    (end, acked)
}

enum Exit {
    /// The connection is over; nothing is sent on it again.
    Ended(SocketEnd),
    Stop,
    /// The run log could not be read.
    Failed,
}

struct Session {
    conn: Box<dyn HubSocketConnection>,
    liveness: Duration,
    /// When the Hub counts as silent unless something arrives first.
    heard_by: Instant,
    acked: bool,
    requests: RequestHandlerSlot,
    connection: ConnectionCell,
    answered: mpsc::UnboundedSender<RequestResult>,
    answers: OwnedMutexGuard<mpsc::UnboundedReceiver<RequestResult>>,
}

impl Session {
    async fn run(
        &mut self,
        ctx: &SessionContext,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Infallible, Exit> {
        let hello = hello(
            &ctx.install_id,
            &ctx.app_version,
            std::env::consts::OS,
            std::env::consts::ARCH,
            &ctx.pinned_scopes,
        );
        self.send(encoded(&hello).ok_or(Exit::Failed)?, stop)
            .await?;
        let hello_sent = Instant::now();
        let cursor = loop {
            if let Some(offset) = self.heard(stop).await? {
                break offset;
            }
        };
        self.acked = true;
        let latency = hello_sent.elapsed().as_millis();
        self.connection.update(|facts| {
            facts.state = ConnectionState::Connected;
            facts.stop_reason = None;
            facts.latency_ms = Some(u64::try_from(latency).unwrap_or(u64::MAX));
            facts.relaying = ctx.relaying;
            facts.caught_up = false;
            facts.in_flight = 0;
        });
        // The Hub stores events from an instance that did not report `read`
        // (docs/HUB.md §7): withholding them is this side's decision alone.
        if !ctx.relaying {
            loop {
                self.heard(stop).await?;
            }
        }
        // After the first ack, not after `Hello`: the ack proves the Hub took
        // the `Hello`, so nothing is built for a connection it refuses.
        let mut snapshots = SnapshotSend::default();
        if let Some(frame) = snapshots.frame(&ctx.snapshots).await {
            self.send(frame, stop).await?;
        }
        let mut outbox = Outbox::after(cursor);
        loop {
            self.connection.update(|facts| {
                facts.caught_up = outbox.caught_up;
                facts.in_flight = outbox.in_flight.len();
            });
            if outbox.pending.is_empty() && !outbox.caught_up {
                outbox.read(&ctx.run_events).await?;
                continue;
            }
            let sendable =
                !outbox.pending.is_empty() && outbox.in_flight.len() < MAX_UNACKED_EVENTS;
            // In this order and not at random: a frame already waiting is
            // read before the next send, so acks keep the window open and
            // count as the Hub being heard.
            tokio::select! {
                biased;
                () = stopped(stop) => return Err(Exit::Stop),
                received = self.conn.received() => {
                    if let Some(offset) = self.took(received)? {
                        outbox.acked(offset);
                    }
                }
                () = sleep_until(self.heard_by) => return Err(Exit::Ended(SocketEnd::Silent)),
                Some(result) = self.answers.recv() => self.answer(result, stop).await?,
                () = ctx.wake.notified(), if outbox.caught_up => outbox.caught_up = false,
                () = ctx.snapshot_wake.notified() => snapshots.woken(ctx.snapshot_debounce),
                () = snapshots.due() => {
                    if let Some(frame) = snapshots.frame(&ctx.snapshots).await {
                        self.send(frame, stop).await?;
                    }
                }
                () = ready(()), if sendable => {
                    if let Some((offset, text)) = outbox.pending.pop_front() {
                        self.send(text, stop).await?;
                        outbox.in_flight.push_back(offset);
                    }
                }
            }
        }
    }

    /// The next frame from the Hub: the offset it acks, if it was a
    /// `CursorAck`. A result ready meanwhile is sent.
    async fn heard(&mut self, stop: &mut watch::Receiver<bool>) -> Result<Option<u64>, Exit> {
        loop {
            tokio::select! {
                biased;
                () = stopped(stop) => return Err(Exit::Stop),
                received = self.conn.received() => return self.took(received),
                () = sleep_until(self.heard_by) => return Err(Exit::Ended(SocketEnd::Silent)),
                Some(result) = self.answers.recv() => self.answer(result, stop).await?,
            }
        }
    }

    /// Any frame proves the Hub is there. A request goes to the bound
    /// handler, which decides whether it is answered at all. With none
    /// bound it goes unanswered, with the socket left open: the Hub keeps
    /// the first outcome it records (docs/HUB.md §5 *Queued requests*), so
    /// only a desktop that has judged the request may answer it, and an
    /// unanswered one is delivered again on the next connection.
    fn took(&mut self, received: SocketReceived) -> Result<Option<u64>, Exit> {
        self.heard_by = Instant::now() + self.liveness;
        let heard_at = now_ms();
        self.connection
            .update(|facts| facts.last_heard_at_ms = Some(heard_at));
        match received {
            SocketReceived::Text(text) => {
                if let Ok(ack) = serde_json::from_str::<CursorAck>(&text) {
                    self.connection
                        .update(|facts| facts.reported_through = Some(ack.offset));
                    return Ok(Some(ack.offset));
                }
                if let Ok(request) = serde_json::from_str::<RequestHeader>(&text) {
                    match self.requests.get() {
                        Some(handler) => self.handle(Arc::clone(handler), text),
                        None => tracing::debug!(
                            request_id = %request.request_id,
                            "a Hub request left unanswered: no handler is bound"
                        ),
                    }
                }
                Ok(None)
            }
            SocketReceived::Alive => Ok(None),
            SocketReceived::Closed(code) => Err(Exit::Ended(SocketEnd::Closed(code))),
        }
    }

    /// Never awaited here: the result comes back through
    /// [`Session::answers`]. One request at a time is the handler's to hold,
    /// not a session's: a session lasts one connection, and a lock here would
    /// be a fresh one on every re-pair
    /// ([`crate::application::hub_requests::HubRequests`]).
    ///
    /// The task is not tracked, so it outlives `hub_disconnect`: a request
    /// already in hand runs to completion, a start still launches, and its
    /// result is dropped with the context. It is not aborted because a start
    /// stopped between spending its id and launching its run would leave a
    /// spent id with no run and no answer.
    fn handle(&self, handler: Arc<dyn HubRequestHandler>, text: String) {
        let answered = self.answered.clone();
        tokio::spawn(async move {
            if let Some(result) = handler.handle(&text).await {
                let _ = answered.send(result);
            }
        });
    }

    /// A result whose send fails is queued again for the next connection.
    async fn answer(
        &mut self,
        result: RequestResult,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<(), Exit> {
        let Some(text) = encoded(&result) else {
            return Ok(());
        };
        if let Err(exit) = self.send(text, stop).await {
            let _ = self.answered.send(result);
            return Err(exit);
        }
        Ok(())
    }

    /// A send still pending after [`Session::liveness`] is a Hub that reads
    /// nothing, which is as gone as one that sends nothing.
    async fn send(&mut self, text: String, stop: &mut watch::Receiver<bool>) -> Result<(), Exit> {
        tokio::select! {
            biased;
            () = stopped(stop) => Err(Exit::Stop),
            sent = timeout(self.liveness, self.conn.send_text(text)) => match sent {
                Ok(Ok(())) => Ok(()),
                Ok(Err(SocketGone)) => Err(Exit::Ended(SocketEnd::Closed(None))),
                Err(_) => Err(Exit::Ended(SocketEnd::Silent)),
            },
        }
    }
}

/// What is read from `run_events` and not yet acked.
struct Outbox {
    /// The last row read, sent or not. The next page starts after it: a page
    /// that ends in rows the allowlist drops would otherwise be read again,
    /// forever.
    read_through: i64,
    pending: VecDeque<(u64, String)>,
    /// Offsets sent and not yet acked, in the order sent, which is
    /// ascending.
    in_flight: VecDeque<u64>,
    /// The last page came back short. Cleared by the wake.
    caught_up: bool,
}

impl Outbox {
    fn after(cursor: u64) -> Self {
        Self {
            read_through: i64::try_from(cursor).unwrap_or(i64::MAX),
            pending: VecDeque::new(),
            in_flight: VecDeque::new(),
            caught_up: false,
        }
    }

    /// One page, read off the runtime: every repository shares one SQLite
    /// lock, which `list_after` takes and releases inside the blocking task.
    async fn read(&mut self, run_events: &Arc<dyn RunEventsPort>) -> Result<(), Exit> {
        let run_events = Arc::clone(run_events);
        let after = self.read_through;
        let page = tokio::task::spawn_blocking(move || run_events.list_after(after, PAGE_SIZE))
            .await
            .map_err(|e| e.to_string())
            .and_then(|page| page);
        let rows: Vec<RunEvent> = page.map_err(|error| {
            tracing::warn!(%error, "the run log could not be read for the Hub");
            Exit::Failed
        })?;
        self.caught_up = rows.len() < PAGE_SIZE;
        if let Some(last) = rows.last() {
            self.read_through = last.offset;
        }
        for event in rows.iter().filter_map(hub_event) {
            if let Some(text) = encoded(&event) {
                self.pending.push_back((event.offset, text));
            }
        }
        Ok(())
    }

    fn acked(&mut self, offset: u64) {
        while self.in_flight.front().is_some_and(|&sent| sent <= offset) {
            self.in_flight.pop_front();
        }
    }
}

/// Resolves once `stop` reads `true`, or its sender is gone.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|&stop| stop).await;
}

pub(super) fn encoded(frame: &impl Serialize) -> Option<String> {
    serde_json::to_string(frame)
        .map_err(|error| tracing::warn!(%error, "a frame for the Hub did not serialise"))
        .ok()
}
