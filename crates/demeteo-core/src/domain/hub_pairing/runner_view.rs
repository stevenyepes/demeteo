//! What a `demeteo-runner`'s `hub_pair_state` and `hub_status` control
//! methods answer. In core rather than the runner crate because both ends
//! read the same spelling: the runner writes it, and a desktop managing that
//! runner's pairing over SSH parses it back.

use serde::{Deserialize, Serialize};

use super::device::DeviceTicket;
use super::{HubMachine, HubStatus};
use crate::domain::hub_socket::connection::HubConnection;

/// `hub_pair_state` on the wire. A success reads `idle`; `hub_status` is what
/// says paired.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairStateView {
    Idle,
    Pending { ticket: DeviceTicket },
    Failed { error: String },
}

impl PairStateView {
    /// Whether the runner's stored pairing may have changed by the time it
    /// answered this, so a desktop's record of it is worth asking again.
    /// `Idle` is also what a finished device-code flow reads as, including
    /// one that completed just before a cancel landed — the cancel awaits
    /// the poll task rather than undoing what it stored.
    pub fn may_have_paired(&self) -> bool {
        matches!(self, Self::Idle)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HubStatusView {
    pub status: HubStatus,
    /// Why the sealed store cannot hold the Hub credentials, when it cannot.
    pub store_unavailable: Option<String>,
    pub connection: HubConnection,
    pub machine: HubMachine,
}

/// Whether a control-socket `error` says the runner predates the method it
/// was asked: a desktop manages runners older than itself, and one from
/// before Hub pairing, or before its activity log, is a version to name, not
/// a failure to report.
pub fn runner_lacks_method(error: &str) -> bool {
    error.starts_with("unknown method")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::hub_pairing::HubPairingState;
    use crate::domain::hub_socket::connection::{ConnectionState, SyncState};
    use crate::domain::hub_socket::StopReason;

    fn round_trip<T: Serialize + for<'de> Deserialize<'de>>(value: &T) -> T {
        let text = serde_json::to_string(value).expect("serializes");
        serde_json::from_str(&text).expect("parses back")
    }

    #[test]
    fn every_pair_state_parses_back_as_itself() {
        let ticket = DeviceTicket {
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://hub.example/device".into(),
            verification_uri_complete: "https://hub.example/device?user_code=ABCD-EFGH".into(),
            fingerprint: "SHA256:aaaa bbbb cccc dddd".into(),
            expires_at: 1_700_000_000,
        };
        for view in [
            PairStateView::Idle,
            PairStateView::Pending { ticket },
            PairStateView::Failed {
                error: "expired".into(),
            },
        ] {
            assert_eq!(round_trip(&view), view);
        }
    }

    #[test]
    fn the_pair_state_tag_is_the_wire_spelling() {
        let value =
            serde_json::to_value(PairStateView::Failed { error: "x".into() }).expect("serializes");
        assert_eq!(value, serde_json::json!({"state": "failed", "error": "x"}));
    }

    #[test]
    fn only_the_dispatchers_own_refusal_reads_as_an_older_runner() {
        assert!(runner_lacks_method("unknown method: hub_request_log"));
        assert!(!runner_lacks_method(
            "Hub pairing is not available on this runner: it has no HTTP client for the Hub"
        ));
        assert!(!runner_lacks_method("invalid params: unknown method field"));
    }

    #[test]
    fn a_status_view_parses_back_as_itself() {
        let view = HubStatusView {
            status: HubStatus {
                state: HubPairingState::Incomplete,
                hub_url: Some("https://hub.example".into()),
                instance_id: Some("inst-1".into()),
                fingerprint: Some("SHA256:aaaa bbbb cccc dddd".into()),
                rp_id: Some("hub.example".into()),
                scopes: vec!["read".into(), "spend".into()],
                paired_at: Some(1_700_000_000),
                keyring_backend: "sealed".into(),
                keyring_available: false,
            },
            store_unavailable: Some("no sealed key".into()),
            connection: HubConnection {
                state: ConnectionState::Stopped,
                stop_reason: Some(StopReason::NeedsRepair),
                latency_ms: Some(12),
                reported_through: Some(40),
                sync: SyncState::Offline,
                last_heard_at_ms: Some(1_700_000_000_000),
            },
            machine: HubMachine {
                instance_name: "runner-1".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
                app_version: "1.2.3".into(),
            },
        };
        assert_eq!(round_trip(&view), view);
    }
}
