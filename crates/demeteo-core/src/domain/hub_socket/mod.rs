//! The desktop's half of the Hub socket — the policy half (docs/HUB.md §5).
//!
//! See [`crate::domain`] for why nothing here is `async`;
//! `src-tauri/tests/hub_layout.rs` fails the build if anything here becomes
//! one. What crosses the socket is decided here and only here: [`hub_event`]
//! is the allowlist a `run_events` row has to pass, [`hello`] the scopes
//! an instance announces, and [`snapshot`] what an instance reports of its
//! Projects. What follows a connection's end is decided here
//! too: [`after`], [`after_close`], [`resets_backoff`], [`refresh_refusal`],
//! [`after_refresh`] and [`backoff`]. What the Settings tab is told of the
//! socket is [`connection::view`].

use demeteo_hub_protocol::{Hello, Scope, PROTOCOL_VERSION};

pub mod connection;
mod projection;
mod snapshot;
mod verdict;

pub use projection::{hub_event, MAX_FEATURE_ID_BYTES};
pub use snapshot::{
    snapshot, FeatureInput, ProjectInput, SnapshotInput, MAX_PINNED_VERSIONS,
    MAX_REPORTED_FEATURES, MAX_SNAPSHOT_BYTES, MAX_TITLE_CHARS, SNAPSHOT_DEBOUNCE_SECS,
};
pub use verdict::{
    after, after_close, after_refresh, backoff, refresh_refusal, resets_backoff, Reconnect,
    RefreshEnd, SocketEnd, StopReason, BACKOFF_CAP_SECS, LIVENESS_DEADLINE_SECS,
    MAX_UNACKED_EVENTS, PAGE_SIZE, PREMATURE_EXPIRY_MARGIN_SECS, STOP_CLOSE_DEADLINE_SECS,
};

/// The `Hello` that opens every connection. A pinned scope that does not
/// parse as a [`Scope`] is dropped rather than sent: the Hub's parser refuses
/// an unknown name, so one stale pin would otherwise refuse the whole frame.
pub fn hello(
    install_id: &str,
    app_version: &str,
    os: &str,
    arch: &str,
    pinned_scopes: &[String],
) -> Hello {
    Hello {
        v: PROTOCOL_VERSION,
        install_id: install_id.to_owned(),
        app_version: app_version.to_owned(),
        os: os.to_owned(),
        arch: arch.to_owned(),
        scopes: pinned_scopes.iter().filter_map(|s| scope(s)).collect(),
    }
}

/// Whether the run log and the snapshot are relayed at all: only while
/// `read` is both pinned and left on in the local policy. Without it the
/// Hub may not see a Feature's progress, so the session sends its `Hello`
/// and nothing else of its own. [`hello`] still announces `read` while it
/// is pinned: the toggle is this desktop's withholding, not a narrower
/// pairing.
pub fn sends_events(pinned_scopes: &[String], read_enabled: bool) -> bool {
    read_enabled && pinned_scopes.iter().any(|s| scope(s) == Some(Scope::Read))
}

fn scope(name: &str) -> Option<Scope> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}
