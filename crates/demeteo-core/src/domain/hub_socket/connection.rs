//! What the Settings › Hub tab is told of the socket. The supervisor and the
//! session write [`ConnectionFacts`]; [`view`] is the only reading of them
//! that crosses IPC.

use serde::Serialize;

use super::StopReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// Unpaired, or no socket started since the last unpair or app start.
    #[default]
    NotRunning,
    Connecting,
    Connected,
    /// Waiting out a backoff or a token refresh before the next dial.
    Reconnecting,
    /// The supervisor ended on a [`StopReason`] and will not dial again.
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    InSync,
    CatchingUp,
    /// Connected with `read` off or unpinned: only the `Hello` is sent.
    NotReporting,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConnectionFacts {
    pub state: ConnectionState,
    pub stop_reason: Option<StopReason>,
    /// Hello → first `CursorAck` of the current connection.
    pub latency_ms: Option<u64>,
    /// The last `CursorAck.offset` this process received. The desktop keeps
    /// no cursor of its own (docs/HUB.md §5), so this is "since the app
    /// started", not a persisted position.
    pub reported_through: Option<u64>,
    /// [`super::sends_events`] for the current session.
    pub relaying: bool,
    /// The last page read from `run_events` came back short, and no write
    /// has woken the session since.
    pub caught_up: bool,
    /// Events sent and not yet acked.
    pub in_flight: usize,
    pub last_heard_at_ms: Option<u64>,
}

/// The IPC payload of `hub_connection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HubConnection {
    pub state: ConnectionState,
    pub stop_reason: Option<StopReason>,
    pub latency_ms: Option<u64>,
    pub reported_through: Option<u64>,
    pub sync: SyncState,
    pub last_heard_at_ms: Option<u64>,
}

/// What is left once the desktop has stopped its own socket to start
/// another. That stop is not one to report, and if no socket replaces it
/// nothing is running: a [`StopReason::Stopped`] left standing would read
/// as stopped for good. The cursor and the last word heard are "since the
/// app started" and stay; the rest belonged to the session that ended.
pub fn after_local_restart(f: &mut ConnectionFacts) {
    *f = ConnectionFacts {
        reported_through: f.reported_through,
        last_heard_at_ms: f.last_heard_at_ms,
        ..ConnectionFacts::default()
    };
}

pub fn sync_state(f: &ConnectionFacts) -> SyncState {
    if f.state != ConnectionState::Connected {
        SyncState::Offline
    } else if !f.relaying {
        SyncState::NotReporting
    } else if f.caught_up && f.in_flight == 0 {
        SyncState::InSync
    } else {
        SyncState::CatchingUp
    }
}

/// [`ConnectionFacts`] as the tab shows them. Latency is withheld unless
/// connected, so a figure from a connection that has since ended is never
/// shown as current.
pub fn view(f: &ConnectionFacts) -> HubConnection {
    HubConnection {
        state: f.state,
        stop_reason: f.stop_reason,
        latency_ms: f
            .latency_ms
            .filter(|_| f.state == ConnectionState::Connected),
        reported_through: f.reported_through,
        sync: sync_state(f),
        last_heard_at_ms: f.last_heard_at_ms,
    }
}
