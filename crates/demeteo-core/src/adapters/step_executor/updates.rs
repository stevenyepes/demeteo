use std::time::Instant;

use crate::domain::ids::FeatureId;
use crate::domain::run_control::driverless_cancel_ends;
use crate::ports::db::{FeaturePatch, FeatureRepository};
use crate::ports::notification::{DomainEvent, NotificationPort};

/// Mark a feature as completed / failed / cancelled, summing step costs and tokens for total_cost/tokens.
#[allow(dead_code)]
pub(crate) fn finish_feature(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    f_id: &FeatureId,
    status: &str,
    start_time: Instant,
) {
    let total_dur = format!("{}s", start_time.elapsed().as_secs());
    let _ = features.update(
        f_id,
        &FeaturePatch {
            status: Some(status.to_string()),
            duration: Some(Some(total_dur)),
            ..spend_rollup(features, f_id)
        },
    );
    let _ = notif.emit(&DomainEvent::FeatureStatusChanged {
        feature_id: f_id.clone(),
        status: status.into(),
    });
}

/// The feature's cost and tokens, summed over its steps.
fn spend_rollup(features: &dyn FeatureRepository, f_id: &FeatureId) -> FeaturePatch {
    let steps = features.steps_for_feature(f_id).unwrap_or_default();
    FeaturePatch {
        total_cost: Some(Some(steps.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum())),
        tokens: Some(Some(steps.iter().map(|s| s.tokens.unwrap_or(0)).sum())),
        ..Default::default()
    }
}

/// Move the feature to `awaiting_gate` because the run stopped to ask a
/// human — a `gate` step, or a non-gate step parked on a synthetic gate.
///
/// The step row alone is not enough. `FeatureDetail` synthesizes its headline
/// from the steps, so it reads "gate needs you" either way; the project list's
/// "Needs you" band and the rail badge read `features.status` (through
/// `feature_status_rollup`), and a feature left at `running` sits in "Active"
/// with no gate anywhere in the chrome. Both parks must call this or the two
/// surfaces disagree exactly while a human is being waited on.
///
/// Guarded on `running` so it never clobbers a terminal state, and undone by
/// [`ensure_feature_running`], which already accepts `awaiting_gate` as a
/// source.
pub(crate) fn park_feature(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    f_id: &FeatureId,
) {
    let cur = features.get(f_id).ok().flatten().map(|f| f.status);
    if cur.as_deref() != Some("running") {
        return;
    }
    set_feature_status(features, notif, f_id, "awaiting_gate");
}

/// Move the feature to `running` when a resume left it parked at a gate
/// status while a non-gate step is being driven.
///
/// The fresh-start bootstrap tail flips the feature to `running`, but the
/// resume paths (gate decision, watchdog, app restart) go through
/// `start_execution_with_ctx`, which never does — so a resumed run keeps
/// reading `awaiting_gate`/`gated` and the UI mislabels a running feature
/// as parked at a gate. Idempotent, and only nudges the transient in-flight
/// statuses: it never clobbers a terminal state (the run loop wouldn't be
/// executing for one).
pub(crate) fn ensure_feature_running(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    f_id: &FeatureId,
) {
    let cur = features.get(f_id).ok().flatten().map(|f| f.status);
    if !matches!(
        cur.as_deref(),
        Some("awaiting_gate") | Some("gated") | Some("bootstrapping")
    ) {
        return;
    }
    set_feature_status(features, notif, f_id, "running");
}

/// End a feature no driver is running as `cancelled`, if
/// [`driverless_cancel_ends`] says nothing else will. A free function over the
/// two ports it writes, so the rule is reachable without an executor.
pub(crate) fn cancel_without_driver(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    f_id: &FeatureId,
) {
    let cur = features.get(f_id).ok().flatten().map(|f| f.status);
    if !cur.as_deref().is_some_and(driverless_cancel_ends) {
        return;
    }
    let _ = features.update(
        f_id,
        &FeaturePatch {
            status: Some("cancelled".to_string()),
            ..spend_rollup(features, f_id)
        },
    );
    let _ = notif.emit(&DomainEvent::FeatureStatusChanged {
        feature_id: f_id.clone(),
        status: "cancelled".to_string(),
    });
}

fn set_feature_status(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    f_id: &FeatureId,
    status: &str,
) {
    let _ = features.update(
        f_id,
        &FeaturePatch {
            status: Some(status.to_string()),
            ..Default::default()
        },
    );
    let _ = notif.emit(&DomainEvent::FeatureStatusChanged {
        feature_id: f_id.clone(),
        status: status.to_string(),
    });
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/updates.rs"]
mod updates_tests;
