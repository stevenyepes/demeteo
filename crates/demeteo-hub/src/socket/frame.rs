//! Which inbound message a text frame is. The rule, and the tagged frame it
//! was chosen over, are on [the module root](super).

use super::MAX_FEATURE_ID_BYTES;
use demeteo_hub_protocol::{Hello, RequestResult, RunEvent, Snapshot, PROTOCOL_VERSION};
use serde_json::Value;

/// The four messages an instance sends. `CursorAck` and `SignedRequest` travel
/// the other way and are deliberately not here: an instance that sends one is
/// refused like any other frame the Hub has no use for.
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    Hello(Hello),
    RunEvent(RunEvent),
    Snapshot(Snapshot),
    RequestResult(RequestResult),
}

/// Why a frame was refused. It carries no text of the frame and no parser
/// message: `serde_json` quotes the field it stopped at, which the peer
/// chose, and this is what a log line is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("the frame is none of the inbound messages")]
    Unrecognised,
    #[error("the frame parses as more than one inbound message")]
    Ambiguous,
    #[error("the frame is of protocol version {0}, not {PROTOCOL_VERSION}")]
    Version(u32),
    #[error("event offset {0} is above what the mirror can key")]
    OffsetTooLarge(u64),
    #[error("a string in the frame holds U+0000")]
    Nul,
    #[error("a feature id of {0} bytes is over {MAX_FEATURE_ID_BYTES}")]
    FeatureIdTooLong(usize),
}

/// A frame of another protocol version is refused whole, even though it
/// parsed: the protocol crate leaves the mismatch policy to its consumers,
/// and a message whose fields happen to line up across versions need not mean
/// what this version's does.
///
/// Three frames are refused here although only the store would otherwise
/// refuse them: an offset above `i64::MAX` (`BIGINT` is signed), a U+0000 in
/// any string (Postgres stores it in neither `TEXT` nor `JSONB`), and a
/// `feature_id` over [`MAX_FEATURE_ID_BYTES`] (it is part of the mirror's
/// primary key). At the store each would surface as a failure, which tells
/// the peer to come back and resend; no resend of that frame can succeed, so
/// it is the peer's violation and is named as one.
///
/// The U+0000 test reads the decoded strings and not the frame text, where
/// the six characters `\u0000` may as well be an escaped backslash followed
/// by `u0000`. Inbound types carry no maps, so their string values are every
/// string the store is handed.
pub fn parse(text: &str) -> Result<Inbound, FrameError> {
    let frame = only(candidates(text))?;
    let version = match &frame {
        Inbound::Hello(hello) => hello.v,
        Inbound::RunEvent(event) => event.v,
        Inbound::Snapshot(snapshot) => snapshot.v,
        Inbound::RequestResult(result) => result.v,
    };
    if version != PROTOCOL_VERSION {
        return Err(FrameError::Version(version));
    }
    if let Inbound::RunEvent(event) = &frame {
        if i64::try_from(event.offset).is_err() {
            return Err(FrameError::OffsetTooLarge(event.offset));
        }
        if event.feature_id.len() > MAX_FEATURE_ID_BYTES {
            return Err(FrameError::FeatureIdTooLong(event.feature_id.len()));
        }
    }
    match serde_json::from_str(text) {
        Ok(value) if !holds_nul(&value) => Ok(frame),
        Ok(_) => Err(FrameError::Nul),
        Err(_) => Err(FrameError::Unrecognised),
    }
}

fn holds_nul(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(holds_nul),
        Value::Object(fields) => fields.values().any(holds_nul),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// Every type is tried, including after one has parsed: stopping at the first
/// would make the answer depend on the order of this list the day two of the
/// shapes overlap.
fn candidates(text: &str) -> impl Iterator<Item = Inbound> {
    [
        serde_json::from_str(text).ok().map(Inbound::Hello),
        serde_json::from_str(text).ok().map(Inbound::RunEvent),
        serde_json::from_str(text).ok().map(Inbound::Snapshot),
        serde_json::from_str(text).ok().map(Inbound::RequestResult),
    ]
    .into_iter()
    .flatten()
}

fn only(mut parsed: impl Iterator<Item = Inbound>) -> Result<Inbound, FrameError> {
    match (parsed.next(), parsed.next()) {
        (Some(frame), None) => Ok(frame),
        (None, _) => Err(FrameError::Unrecognised),
        (Some(_), Some(_)) => Err(FrameError::Ambiguous),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use demeteo_hub_protocol::RequestOutcome;

    fn result(request_id: &str) -> Inbound {
        Inbound::RequestResult(RequestResult {
            v: PROTOCOL_VERSION,
            request_id: request_id.to_owned(),
            outcome: RequestOutcome::Accepted,
        })
    }

    /// No text reaches the third arm through [`parse`] while the four types
    /// stay disjoint, so the arm is driven with the candidates themselves.
    #[test]
    fn two_types_parsing_one_frame_is_an_error_and_not_the_first_of_them() {
        assert_eq!(only([result("a")].into_iter()), Ok(result("a")));
        assert_eq!(only([].into_iter()), Err(FrameError::Unrecognised));
        assert_eq!(
            only([result("a"), result("b")].into_iter()),
            Err(FrameError::Ambiguous)
        );
    }
}
