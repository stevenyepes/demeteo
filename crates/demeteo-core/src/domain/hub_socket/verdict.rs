use std::time::Duration;

use serde::Serialize;

use crate::domain::hub_pairing::{hub_error_code, token_error, token_refusal, HubPairingError};

/// The longest wait between two dials, whatever the attempt count.
pub const BACKOFF_CAP_SECS: u64 = 300;

/// Seconds the desktop goes on hearing nothing from the Hub, of any kind,
/// before it drops the connection and ends it as [`SocketEnd::Silent`].
/// Mirrors `PONG_DEADLINE_SECS` in `crates/demeteo-hub/src/socket/mod.rs`,
/// which records why it is that long.
pub const LIVENESS_DEADLINE_SECS: u64 = 90;

/// Seconds a stopped session waits for its `1001` close to go out. Not
/// [`LIVENESS_DEADLINE_SECS`]: a re-pair awaits the old session's end under
/// the refresh lock, so a Hub that has stopped reading the old connection
/// would hold the re-pair, and every refresh queued behind it, for the whole
/// liveness deadline.
pub const STOP_CLOSE_DEADLINE_SECS: u64 = 5;

/// Rows read from `run_events` per page.
pub const PAGE_SIZE: usize = 128;

/// Events sent beyond the last `CursorAck` before the session stops sending
/// and only reads, so a Hub that has stopped acking is not fed without end.
pub const MAX_UNACKED_EVENTS: usize = 256;

const _: () = assert!(PAGE_SIZE <= 256 && MAX_UNACKED_EVENTS <= 256);

/// How much life the local clock may still give a just-issued token when the
/// Hub closes `4001` on it, and the close still be read as that token
/// expiring. The Hub closes at the expiry it recorded at issue, to the
/// second, and the desktop counts its own from before it sent the refresh,
/// so a real expiry arrives with the local count at or below zero: what the
/// margin absorbs is whole-second rounding at both ends, delivery latency,
/// and a step correction of the local clock between issue and close. A
/// minute covers those, and is still a fifteenth of the Hub's 900-second
/// token, so a Hub that cuts sessions short is told apart from one that does
/// not.
pub const PREMATURE_EXPIRY_MARGIN_SECS: u64 = 60;

/// How one connection attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketEnd {
    /// The socket was open and closed, with the close code if one was sent.
    Closed(Option<u16>),
    /// No frame for [`LIVENESS_DEADLINE_SECS`].
    Silent,
    /// No HTTP answer to the upgrade: DNS, TCP or TLS failed.
    Unreachable,
    /// The upgrade was answered with something other than `101`.
    /// `fresh_token` is whether the access token was refreshed for this
    /// attempt rather than carried over from an earlier one.
    Refused { status: u16, fresh_token: bool },
}

/// What the supervisor does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconnect {
    AfterBackoff,
    /// Refresh the access token, then dial with no backoff.
    RefreshNow,
    /// Wait the backoff, then refresh: the token the Hub just closed is not
    /// dialled again. The session that ended in it does not count as
    /// connected (see [`resets_backoff`]), so each of these waits longer.
    RefreshAfterBackoff,
    Stop(StopReason),
}

/// Why the supervisor stopped. Each one holds until a user action or the next
/// app start, since redialling would meet the same answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Closed `4002`. Never reconnect after being displaced: the redial would
    /// displace its successor, which would reconnect in its turn, and two
    /// clients of one instance would displace each other forever (docs/HUB.md
    /// §5 *Close codes*).
    Displaced,
    /// Closed `1008`: the Hub would refuse the same frame again.
    ProtocolViolation,
    /// A `401` to an access token issued for this very attempt.
    Unauthorized,
    Refused,
    /// The refresh token was refused (`invalid_grant`): only pairing again
    /// gets a new one.
    NeedsRepair,
    /// A refresh was sent and no answer read. The Hub may have rotated the
    /// token, and presenting the old one again would be a reuse that revokes
    /// the whole family, so it is never retried.
    RefreshUncertain,
    ClockSkew,
    /// Asked to stop, by an unpair or app shutdown. No [`SocketEnd`] or
    /// [`RefreshEnd`] maps here.
    Stopped,
}

/// The AC4 table. Every row is asserted in
/// `src-tauri/tests/infrastructure/hub.rs`.
///
/// A refusal status the Hub's upgrade handler does not send is judged by its
/// class: a `5xx` or `429` is the server or a proxy in front of it being
/// unavailable and is waited out; any other status — a `403`, a `404` from a
/// Hub without the route, a redirect — would be answered the same way again.
pub fn after(end: SocketEnd) -> Reconnect {
    match end {
        SocketEnd::Closed(Some(4001)) => Reconnect::RefreshNow,
        // Never redial: see `StopReason::Displaced`.
        SocketEnd::Closed(Some(4002)) => Reconnect::Stop(StopReason::Displaced),
        SocketEnd::Closed(Some(1008)) => Reconnect::Stop(StopReason::ProtocolViolation),
        SocketEnd::Closed(_) | SocketEnd::Silent | SocketEnd::Unreachable => {
            Reconnect::AfterBackoff
        }
        SocketEnd::Refused {
            status: 401,
            fresh_token: false,
        } => Reconnect::RefreshNow,
        SocketEnd::Refused {
            status: 401,
            fresh_token: true,
        } => Reconnect::Stop(StopReason::Unauthorized),
        SocketEnd::Refused {
            status: 429 | 500..=599,
            ..
        } => Reconnect::AfterBackoff,
        SocketEnd::Refused { .. } => Reconnect::Stop(StopReason::Refused),
    }
}

/// [`after`], told what the local clock says of the token the connection
/// was dialled with: whether it was refreshed for this attempt, and the
/// seconds it has left, `None` when no expiry is held for it.
///
/// A `4001` on a fresh token with more than [`PREMATURE_EXPIRY_MARGIN_SECS`]
/// left is not that token expiring — clock disagreement, or a Hub bug — and
/// refreshing at once would rotate the refresh token in a loop as fast as
/// the Hub closes. `fresh_token` alone proves nothing: a connection opened
/// with a fresh token can last that token's whole life and then close
/// `4001`. A held token's `4001` is the AC4 row whatever its count, since a
/// refresh is what tells the two apart.
pub fn after_close(end: SocketEnd, fresh_token: bool, secs_left: Option<u64>) -> Reconnect {
    match (end, secs_left) {
        (SocketEnd::Closed(Some(4001)), Some(left))
            if fresh_token && left > PREMATURE_EXPIRY_MARGIN_SECS =>
        {
            Reconnect::RefreshAfterBackoff
        }
        _ => after(end),
    }
}

/// Whether a session that ended in `verdict` starts the backoff over. Only
/// an acked one does, and not one judged [`Reconnect::RefreshAfterBackoff`]:
/// its ack proves the Hub admitted the token, not that the connection holds,
/// and counting it would keep every one of those waits at `backoff(0)`.
pub fn resets_backoff(acked: bool, verdict: Reconnect) -> bool {
    acked && verdict != Reconnect::RefreshAfterBackoff
}

/// How a `refresh_token` grant failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshEnd {
    /// The request never left: the connection was not made.
    Unsent,
    /// The request was sent and no readable answer came back. A `5xx` other
    /// than `503` with no Hub error code is one: a gateway may have
    /// forwarded the request and lost the Hub's answer.
    Unanswered,
    /// A `503` with no Hub error code, taken to be a proxy or a Hub that is
    /// down or redeploying turning the request away before anything read its
    /// token. That is an assumption about the deployment — that a proxy in
    /// front of the Hub answers `503` only for a request it did not forward
    /// — not something the answer proves. A proxy that answers `503` on an
    /// upstream reset after forwarding, as Envoy does, breaks it: the Hub may
    /// have rotated the token, the retry presents a spent refresh token and
    /// revokes the family, and the answer is `invalid_grant`, so
    /// [`StopReason::NeedsRepair`]. The user pairs again, which is no worse
    /// than stopping here.
    Unavailable,
    /// The Hub answered with a refusal, as [`token_refusal`] reads it.
    Refused(HubPairingError),
}

/// The [`RefreshEnd`] of a non-2xx answer to a refresh. A body carrying a
/// Hub error code is read as [`token_refusal`] reads it, whatever the
/// status; only a body without one is judged by its status.
pub fn refresh_refusal(status: u16, body: &[u8]) -> RefreshEnd {
    match (hub_error_code(body), status) {
        (Some(code), _) => RefreshEnd::Refused(token_error(&code)),
        (None, 503) => RefreshEnd::Unavailable,
        (None, 500..=599) => RefreshEnd::Unanswered,
        (None, _) => RefreshEnd::Refused(token_refusal(body)),
    }
}

/// What a failed refresh means for the supervisor.
///
/// A refusal arrives as [`refresh_refusal`] reads it, so `invalid_dpop_proof`
/// is [`HubPairingError::ClockSkew`] by the time it is here. What nothing
/// can have rotated is waited out: a request never sent, and the one `503`
/// the refresh grant sends, `too_many_requests`, a full proof cache. So is a
/// bare `503`, which is only assumed not to have reached the Hub (see
/// [`RefreshEnd::Unavailable`]). Everything else stops, because retrying a
/// refresh token the Hub may already have rotated revokes the family.
pub fn after_refresh(end: RefreshEnd) -> Reconnect {
    match end {
        RefreshEnd::Unsent | RefreshEnd::Unavailable => Reconnect::AfterBackoff,
        RefreshEnd::Unanswered => Reconnect::Stop(StopReason::RefreshUncertain),
        RefreshEnd::Refused(HubPairingError::ClockSkew) => Reconnect::Stop(StopReason::ClockSkew),
        RefreshEnd::Refused(HubPairingError::HubRefused { error }) => match error.as_str() {
            "invalid_grant" => Reconnect::Stop(StopReason::NeedsRepair),
            "too_many_requests" => Reconnect::AfterBackoff,
            _ => Reconnect::Stop(StopReason::Refused),
        },
        RefreshEnd::Refused(_) => Reconnect::Stop(StopReason::Refused),
    }
}

/// `min(2^attempt, BACKOFF_CAP_SECS)` seconds, scaled by `jitter` clamped to
/// `[0.5, 1.0]`. A NaN jitter waits the full time: `f64::clamp` passes NaN
/// on, and `Duration::from_secs_f64` panics on it.
pub fn backoff(attempt: u32, jitter: f64) -> Duration {
    let secs = 1u64
        .checked_shl(attempt)
        .unwrap_or(u64::MAX)
        .min(BACKOFF_CAP_SECS);
    let jitter = if jitter.is_nan() {
        1.0
    } else {
        jitter.clamp(0.5, 1.0)
    };
    Duration::from_secs_f64(secs as f64 * jitter)
}
