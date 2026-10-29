use demeteo_hub_protocol::{RunEvent, RunEventKind, RunEventPayload, PROTOCOL_VERSION};
use serde_json::Value;

use crate::ports::run_events;

/// The Hub's bound on `feature_id`, mirrored because this crate must not
/// depend on `demeteo-hub`. Why it exists, and why an id over it is the
/// peer's violation rather than a store failure, is recorded beside the
/// Hub's own constant and its use in `crates/demeteo-hub/src/socket/frame.rs`.
pub const MAX_FEATURE_ID_BYTES: usize = 256;

/// The allowlisted `run_events` kinds, by what decides their protocol kind
/// and which payload id they carry onto the wire.
enum Allowed {
    Feature,
    Step,
    Gate(RunEventKind),
}

/// Projects one `run_events` row onto the protocol's allowlist, or `None`
/// when the row must not cross. The `match` below is the table; every row of
/// it, and every kind and status it drops, is asserted in
/// `src-tauri/tests/infrastructure/hub.rs`.
///
/// The outbound payload is built from the one id the kind carries and
/// nothing else: `payload_json` is read for `status` and that id, never
/// forwarded. A row the Hub's frame parser would refuse — an offset or
/// timestamp it cannot hold, an oversized feature id, a U+0000 — is dropped
/// here, since no resend of it could ever succeed.
pub fn hub_event(row: &run_events::RunEvent) -> Option<RunEvent> {
    let offset = u64::try_from(row.offset).ok().filter(|&o| o > 0)?;
    let occurred_at = u64::try_from(row.created_at).ok()? / 1000;
    if row.run_id.len() > MAX_FEATURE_ID_BYTES || row.run_id.contains('\0') {
        return None;
    }
    let allowed = match row.kind.as_str() {
        "feature_status" => Allowed::Feature,
        "step_progress" => Allowed::Step,
        "gate_required" => Allowed::Gate(RunEventKind::GatePending),
        "gate_decided" | "gate_auto_approved" => Allowed::Gate(RunEventKind::GateDecided),
        _ => return None,
    };
    let fields: Value = serde_json::from_str(row.payload_json.as_deref()?).ok()?;
    let status = || text(&fields, "status");
    let id = |field| -> Option<String> {
        let id = text(&fields, field)?;
        (!id.contains('\0')).then(|| id.to_owned())
    };
    let (kind, payload) = match allowed {
        Allowed::Feature => (feature_kind(status()?)?, RunEventPayload::default()),
        Allowed::Step => (
            step_kind(status()?)?,
            RunEventPayload {
                step_id: Some(id("step_id")?),
                ..RunEventPayload::default()
            },
        ),
        Allowed::Gate(kind) => (
            kind,
            RunEventPayload {
                step_execution_id: Some(id("step_execution_id")?),
                // Rows written before `gate_required` carried it have none;
                // the Hub still lists such a gate, only without a node.
                step_id: (kind == RunEventKind::GatePending)
                    .then(|| id("step_id"))
                    .flatten(),
                ..RunEventPayload::default()
            },
        ),
    };
    Some(RunEvent {
        v: PROTOCOL_VERSION,
        feature_id: row.run_id.clone(),
        offset,
        kind,
        occurred_at,
        payload,
    })
}

fn text<'a>(payload: &'a Value, field: &str) -> Option<&'a str> {
    payload.get(field)?.as_str()
}

fn feature_kind(status: &str) -> Option<RunEventKind> {
    match status {
        "running" => Some(RunEventKind::FeatureStarted),
        // The run ended well and only the PR is left to open (see
        // `auto_publish_pr`): the Hub has no kind for "finished, unpublished",
        // and dropping the row would leave the run `running` there for good.
        "completed" | "awaiting_mr" => Some(RunEventKind::FeatureCompleted),
        "failed" => Some(RunEventKind::FeatureFailed),
        "cancelled" => Some(RunEventKind::FeatureCancelled),
        _ => None,
    }
}

fn step_kind(status: &str) -> Option<RunEventKind> {
    match status {
        "running" => Some(RunEventKind::StepStarted),
        "completed" => Some(RunEventKind::StepCompleted),
        "failed" => Some(RunEventKind::StepFailed),
        _ => None,
    }
}
