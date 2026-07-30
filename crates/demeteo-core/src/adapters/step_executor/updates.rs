use std::time::Instant;

use crate::domain::ids::FeatureId;
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
    let total_cost = features
        .steps_for_feature(f_id)
        .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
        .unwrap_or(0.0);
    let total_tokens = features
        .steps_for_feature(f_id)
        .map(|list| list.iter().map(|s| s.tokens.unwrap_or(0)).sum::<i64>())
        .unwrap_or(0);
    let total_dur = format!("{}s", start_time.elapsed().as_secs());
    let _ = features.update(
        f_id,
        &FeaturePatch {
            status: Some(status.to_string()),
            total_cost: Some(Some(total_cost)),
            tokens: Some(Some(total_tokens)),
            duration: Some(Some(total_dur)),
            ..Default::default()
        },
    );
    let _ = notif.emit(&DomainEvent::FeatureStatusChanged {
        feature_id: f_id.clone(),
        status: status.into(),
    });
}
