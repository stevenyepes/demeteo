//! Telling the Hub that this machine is unpairing: `POST /unpair`, made by
//! [`HubPairing::disconnect`](crate::adapters::hub::HubPairing::disconnect)
//! before it deletes anything (`docs/HUB.md` §4 *Revocation*).
//!
//! **Best effort, and said so.** The local delete happens whatever the Hub
//! answers: a disconnect that waited on an unreachable Hub, or refused to
//! forget a pairing the Hub would not let go of, would leave the user with
//! no way to unpair at all. What the Hub was told is reported beside the
//! status instead, so a user whose Hub was not told knows to remove the
//! instance there.

use serde::{Deserialize, Serialize};

use super::HubStatus;

/// What became of telling the Hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum HubUnpairNotice {
    /// The Hub removed this instance: its tokens are retired and it has left
    /// the fleet.
    Removed,
    /// Nothing was paired, so there was nobody to tell.
    NotPaired,
    /// The Hub was not told, or did not accept it. `reason` is in words and
    /// names no token or URL. The instance may still be listed on the Hub,
    /// where it can be removed by hand.
    NotTold { reason: String },
}

/// What a disconnect answers: the status as storage holds it afterwards, and
/// what the Hub was told. Flattened, so a caller that reads only a
/// [`HubStatus`] — a desktop older than this runner — still reads one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubDisconnected {
    #[serde(flatten)]
    pub status: HubStatus,
    /// `None` from a runner that predates `POST /unpair`, which told the Hub
    /// nothing.
    #[serde(default)]
    pub hub_notice: Option<HubUnpairNotice>,
}

/// Why `POST /unpair` did not answer `204`. No variant carries a token, a
/// URL or a body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnpairFailure {
    #[error("the Hub answered HTTP {status}")]
    Refused { status: u16 },
    #[error("{0}")]
    Unavailable(String),
}

/// The URL of the route, which is also the `htu` of the proof that calls it.
pub fn unpair_url(origin: &str) -> String {
    format!("{origin}/unpair")
}

/// Whether a refused unpair earns one access-token refresh and a second try:
/// only a `401` to a token this process held from before, which the Hub may
/// have expired early. A `401` to a token refreshed for this very call means
/// the pairing itself is gone on the Hub, and a refresh would be refused too.
pub fn retry_after_refresh(failure: &UnpairFailure, token_was_held: bool) -> bool {
    token_was_held && *failure == UnpairFailure::Refused { status: 401 }
}

/// The notice an unpair's last answer gives.
pub fn unpair_notice(answer: Result<(), UnpairFailure>) -> HubUnpairNotice {
    let reason = match answer {
        Ok(()) => return HubUnpairNotice::Removed,
        Err(UnpairFailure::Refused { status: 401 }) => {
            "the Hub no longer accepts this pairing; it may already have removed this instance"
                .to_owned()
        }
        Err(UnpairFailure::Refused { status }) => format!("the Hub answered HTTP {status}"),
        Err(UnpairFailure::Unavailable(reason)) => reason,
    };
    HubUnpairNotice::NotTold { reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::hub_pairing::HubPairingState;

    #[test]
    fn only_a_204_is_removed_and_every_failure_says_why_in_words() {
        assert_eq!(unpair_notice(Ok(())), HubUnpairNotice::Removed);
        let not_told = |answer| match unpair_notice(answer) {
            HubUnpairNotice::NotTold { reason } => reason,
            other => panic!("expected not_told, got {other:?}"),
        };
        assert!(not_told(Err(UnpairFailure::Refused { status: 401 })).contains("already"));
        assert_eq!(
            not_told(Err(UnpairFailure::Refused { status: 503 })),
            "the Hub answered HTTP 503"
        );
        assert_eq!(
            not_told(Err(UnpairFailure::Unavailable("timed out".into()))),
            "timed out"
        );
    }

    #[test]
    fn only_a_401_to_a_held_token_is_retried() {
        let unauthorized = UnpairFailure::Refused { status: 401 };
        assert!(retry_after_refresh(&unauthorized, true));
        assert!(!retry_after_refresh(&unauthorized, false));
        assert!(!retry_after_refresh(
            &UnpairFailure::Refused { status: 500 },
            true
        ));
        assert!(!retry_after_refresh(
            &UnpairFailure::Unavailable("x".into()),
            true
        ));
    }

    /// An older desktop parses a runner's answer as a bare `HubStatus`, and a
    /// newer desktop parses an older runner's bare status as one with no
    /// notice.
    #[test]
    fn a_disconnected_answer_reads_as_a_status_and_a_status_as_one_with_no_notice() {
        let status = HubStatus {
            state: HubPairingState::Unpaired,
            hub_url: None,
            instance_id: None,
            fingerprint: None,
            rp_id: None,
            scopes: Vec::new(),
            paired_at: None,
            keyring_backend: "sealed".into(),
            keyring_available: true,
        };
        let disconnected = HubDisconnected {
            status: status.clone(),
            hub_notice: Some(HubUnpairNotice::NotTold {
                reason: "offline".into(),
            }),
        };
        let wire = serde_json::to_value(&disconnected).unwrap();
        assert_eq!(wire["hub_notice"]["outcome"], "not_told");
        assert_eq!(wire["hub_notice"]["reason"], "offline");
        assert_eq!(
            serde_json::from_value::<HubStatus>(wire).unwrap(),
            status.clone()
        );

        let old = serde_json::to_value(&status).unwrap();
        let read: HubDisconnected = serde_json::from_value(old).unwrap();
        assert_eq!(read.hub_notice, None);
        assert_eq!(read.status, status);
    }
}
