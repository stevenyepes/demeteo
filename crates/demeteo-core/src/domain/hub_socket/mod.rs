//! The desktop's half of the Hub socket — the policy half (docs/HUB.md §5).
//!
//! See [`crate::domain`] for why nothing here is `async`;
//! `src-tauri/tests/hub_layout.rs` fails the build if anything here becomes
//! one. What crosses the socket is decided here and only here: [`hub_event`]
//! is the allowlist a `run_events` row has to pass, [`failure_reason`] the
//! closed cause a failure carries with it, [`hello`] the scopes
//! an instance announces, and [`snapshot`] what an instance reports of its
//! Projects, with [`outbound_catalog`] deciding what of its harness probe
//! crosses and [`probe_due`] when that probe runs. Which `feature_status`
//! row the log still owes the Hub before a run reads as ended there is
//! [`status_backfill`] and [`removal_logs_cancel`].
//! What follows a connection's end is decided here
//! too: [`after`], [`after_close`], [`resets_backoff`], [`refresh_refusal`],
//! [`after_refresh`] and [`backoff`]. What the Settings tab is told of the
//! socket is [`connection::view`]. Which Feature id a run is reported
//! under, and whether this instance reports it at all, is
//! [`reported_feature_id`].

use crate::domain::hub_request::HubLocalPolicy;
use demeteo_hub_protocol::{Hello, Scope, PROTOCOL_VERSION};

pub mod connection;
mod failure;
mod harness_catalog;
mod projection;
mod reporting;
mod run_end;
mod snapshot;
mod verdict;

pub use failure::{
    classified, failure_reason, may_carry_failure, FAILURE_CONTEXT_WINDOW, OVER_WALL_CLOCK_CAP,
    WALL_CLOCK_CAP_PHRASE,
};
pub use harness_catalog::{
    merged, outbound_catalog, probe_due, CatalogEntry, HarnessProbe, HarnessProbeOutcome,
    ProbeTrigger, HARNESS_PROBE_CONNECT_FLOOR_SECS, HARNESS_PROBE_INTERVAL_SECS,
    HARNESS_PROBE_TIMEOUT_SECS, MAX_CATALOG_CHARS, MAX_CATALOG_MODELS,
};
pub use projection::{hub_event, MAX_FEATURE_ID_BYTES};
pub use reporting::{
    observed_runner_hub_state, reported_feature_id, runner_reports_itself, EventOwner,
    RunnerHubState,
};
pub use run_end::{removal_logs_cancel, run_has_ended, status_backfill};
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

/// The scopes a `Hello` announces: the pinned ones, less `spend` and `gates`
/// while their local toggle is off. The Hub declines to queue a request for
/// a scope this did not announce (`409 scope_not_granted`, docs/HUB.md §7),
/// so its web app can say a scope is off before a dispatch is sent rather
/// than after the instance refuses it. That is a courtesy to the person
/// dispatching, never the enforcement: [`crate::domain::hub_request::precheck`]
/// still judges every request against the toggles.
///
/// `read` is announced whenever it is pinned, toggle or not: with it off the
/// socket relays nothing ([`sends_events`]), which is this instance
/// withholding, not a narrower pairing.
pub fn announced_scopes(pinned_scopes: &[String], policy: &HubLocalPolicy) -> Vec<String> {
    pinned_scopes
        .iter()
        .filter(|name| match scope(name) {
            Some(Scope::Spend) => policy.spend_enabled,
            Some(Scope::Gates) => policy.gates_enabled,
            Some(Scope::Read) | None => true,
        })
        .cloned()
        .collect()
}

/// Whether the run log and the snapshot are relayed at all: only while
/// `read` is both pinned and left on in the local policy. Without it the
/// Hub may not see a Feature's progress, so the session sends its `Hello`
/// and nothing else of its own. [`announced_scopes`] still announces `read`
/// while it is pinned.
pub fn sends_events(pinned_scopes: &[String], read_enabled: bool) -> bool {
    read_enabled && pinned_scopes.iter().any(|s| scope(s) == Some(Scope::Read))
}

fn scope(name: &str) -> Option<Scope> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}
