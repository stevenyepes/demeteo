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

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::ready;
use std::sync::Arc;
use std::time::Duration;

use demeteo_hub_protocol::{CursorAck, RequestHeader};
use serde::Serialize;
use tokio::sync::{watch, Notify};
use tokio::time::{sleep_until, timeout, Instant};

use crate::domain::hub_socket::{
    hello, hub_event, sends_events, SocketEnd, LIVENESS_DEADLINE_SECS, MAX_UNACKED_EVENTS,
    PAGE_SIZE, STOP_CLOSE_DEADLINE_SECS,
};
use crate::ports::hub::{HubSocketConnection, SocketGone, SocketReceived};
use crate::ports::run_events::{RunEvent, RunEventsPort};

/// RFC 6455 §7.4.1, for an endpoint that is going away.
const GOING_AWAY: u16 = 1001;
/// RFC 6455 §7.4.1: the desktop could not read its own run log. The Hub is
/// told to expect the connection back, which [`crate::domain::hub_socket::after`]
/// makes it.
const INTERNAL_ERROR: u16 = 1011;

/// What one session reads and announces.
pub struct SessionContext {
    pub install_id: String,
    pub app_version: String,
    pub pinned_scopes: Vec<String>,
    pub run_events: Arc<dyn RunEventsPort>,
    /// Fired by [`super::wake::WakingRunEvents`] on every append to
    /// `run_events`; it has to be that decorator's, or an append wakes
    /// nothing and a live row waits for the next connection.
    pub wake: Arc<Notify>,
    /// How long the Hub may say nothing, of any kind, and how long one send
    /// may stay pending.
    pub liveness: Duration,
    /// How long the `1001` close of a stopped session may stay pending; see
    /// [`STOP_CLOSE_DEADLINE_SECS`].
    pub stop_close: Duration,
}

impl SessionContext {
    pub fn new(
        install_id: String,
        app_version: String,
        pinned_scopes: Vec<String>,
        run_events: Arc<dyn RunEventsPort>,
        wake: Arc<Notify>,
    ) -> Self {
        Self {
            install_id,
            app_version,
            pinned_scopes,
            run_events,
            wake,
            liveness: Duration::from_secs(LIVENESS_DEADLINE_SECS),
            stop_close: Duration::from_secs(STOP_CLOSE_DEADLINE_SECS),
        }
    }

    pub fn with_liveness(mut self, liveness: Duration) -> Self {
        self.liveness = liveness;
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
        let cursor = loop {
            if let Some(offset) = self.heard(stop).await? {
                break offset;
            }
        };
        self.acked = true;
        // The Hub stores events from an instance that did not report `read`
        // (docs/HUB.md §7): withholding them is this side's decision alone.
        if !sends_events(&ctx.pinned_scopes) {
            loop {
                self.heard(stop).await?;
            }
        }
        let mut outbox = Outbox::after(cursor);
        loop {
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
                () = ctx.wake.notified(), if outbox.caught_up => outbox.caught_up = false,
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
    /// `CursorAck`.
    async fn heard(&mut self, stop: &mut watch::Receiver<bool>) -> Result<Option<u64>, Exit> {
        tokio::select! {
            biased;
            () = stopped(stop) => Err(Exit::Stop),
            received = self.conn.received() => self.took(received),
            () = sleep_until(self.heard_by) => Err(Exit::Ended(SocketEnd::Silent)),
        }
    }

    /// Any frame proves the Hub is there. One that is not a `CursorAck` is
    /// not acted on, and a `SignedRequest` goes unanswered, with no
    /// `RequestResult` and the socket left open: the Hub keeps the first
    /// outcome it records (docs/HUB.md §5 *Queued requests*), so a `refused`
    /// sent while request verification is not built (§2) would stand
    /// after it is. Unanswered, the request is delivered again on the next
    /// connection.
    fn took(&mut self, received: SocketReceived) -> Result<Option<u64>, Exit> {
        self.heard_by = Instant::now() + self.liveness;
        match received {
            SocketReceived::Text(text) => {
                if let Ok(ack) = serde_json::from_str::<CursorAck>(&text) {
                    return Ok(Some(ack.offset));
                }
                if let Ok(request) = serde_json::from_str::<RequestHeader>(&text) {
                    tracing::debug!(
                        request_id = %request.request_id,
                        "a Hub request left unanswered"
                    );
                }
                Ok(None)
            }
            SocketReceived::Alive => Ok(None),
            SocketReceived::Closed(code) => Err(Exit::Ended(SocketEnd::Closed(code))),
        }
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

fn encoded(frame: &impl Serialize) -> Option<String> {
    serde_json::to_string(frame)
        .map_err(|error| tracing::warn!(%error, "a frame for the Hub did not serialise"))
        .ok()
}
