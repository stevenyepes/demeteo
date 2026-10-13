use crate::domain::cache_release::releasable_after_mr_poll;
use crate::domain::ids::FeatureId;
use crate::domain::models::{Feature, Notification, NotificationKind};
use crate::ports::db::{FeaturePatch, FeatureRepository, NotificationRepository};
use crate::ports::discovery::{DiscoveryPort, TicketPort};
use crate::ports::mr_publisher::MrPublisher;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::worktree_ops::FeatureCachePort;
use std::sync::Arc;

/// Background MR-state monitor — polls `MrPublisher::fetch_mr_state`
/// for every feature with `mr_state = 'open'`, then on a transition
/// to `merged` updates the feature row, persists a `Notification`,
/// and emits a live `DomainEvent::MrMerged` for the bell + toast.
///
/// 2 minutes is the sweet spot between "merge reflected quickly"
/// and "don't hammer the provider API" — a fresh merge shows up in
/// the UI within ~2 minutes of the user clicking "Merge" on
/// GitHub/GitLab. Per-feature polling is fine while N is small; a
/// future `GET /repos/:o/:r/pulls` batch upgrade plugs in at the
/// single call site below without changing any other layer.
const MR_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(120);

/// Everything one poll reads or writes.
pub struct MrMonitorPorts {
    pub features: Arc<dyn FeatureRepository>,
    pub mr_publisher: Arc<dyn MrPublisher>,
    pub notifications: Arc<dyn NotificationRepository>,
    pub notif: Arc<dyn NotificationPort>,
    pub tickets: Arc<dyn TicketPort>,
    pub discoveries: Arc<dyn DiscoveryPort>,
    pub cache: Arc<dyn FeatureCachePort>,
}

pub fn start_mr_monitor(ports: MrMonitorPorts, runtime: &tokio::runtime::Handle) {
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(MR_POLL_INTERVAL);
        // Skip the immediate first tick so app launch doesn't fire
        // a burst of API calls before the user has interacted.
        interval.tick().await;
        loop {
            interval.tick().await;
            eprintln!("[MrMonitor] tick — polling open MRs");
            if let Err(e) = check_mr_states(&ports).await {
                eprintln!("[MrMonitor] poll error: {}", e);
            }
        }
    });
}

async fn check_mr_states(ports: &MrMonitorPorts) -> Result<(), String> {
    let mr_publisher = &*ports.mr_publisher;
    let open = ports.features.list_with_open_mr()?;
    eprintln!("[MrMonitor] found {} feature(s) with open MR", open.len());
    for feature in &open {
        let url = match &feature.mr_url {
            Some(u) if !u.is_empty() => u.clone(),
            _ => continue,
        };

        let new_state = match mr_publisher
            .fetch_mr_state(&feature.project_id.0, &url)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[MrMonitor] fetch_mr_state failed for feature {}: {}",
                    feature.id.0, e
                );
                continue;
            }
        };

        apply_polled_state(ports, feature, &new_state).await?;
    }
    Ok(())
}

/// Everything a poll does once the forge has answered for `feature`: record
/// the state, release the dependency cache a settled PR no longer needs, and
/// recompute the Discoveries the Feature gates.
///
/// Shared with an on-demand refresh
/// ([`crate::application::tickets::refresh`]) because a settle is not only a
/// column write. A refresh that recorded `merged` by itself would take the
/// Feature out of `list_with_open_mr` for good, and the cache release and the
/// dependents' notice this loop owes it would never happen.
pub async fn apply_polled_state(
    ports: &MrMonitorPorts,
    feature: &Feature,
    new_state: &str,
) -> Result<(), String> {
    let features = &*ports.features;
    let notifications = &*ports.notifications;
    let notif = &*ports.notif;
    eprintln!("[MrMonitor] feature {} state = {}", feature.id.0, new_state);
    let recorded = match new_state {
        "open" => false,
        "merged" => {
            record_mr_state(feature, new_state, features, notifications, notif)?;
            true
        }
        _ => record_mr_state(feature, new_state, features, notifications, notif).is_ok(),
    };

    if recorded && releasable_after_mr_poll(&feature.status, new_state) {
        let settled = Feature {
            mr_state: Some(new_state.to_string()),
            ..feature.clone()
        };
        if let Err(e) = ports.cache.release(&settled).await {
            eprintln!(
                "[MrMonitor] dependency cache release failed for feature {}: {}",
                feature.id.0, e
            );
        }
    }

    // Placed here rather than inside `record_merged` because `closed`
    // never reaches that function and releases dependents just as
    // `merged` does (`docs/PRD_DISCOVERY.md` §6.4), and because the
    // recompute must not inherit that function's notification-keyed
    // guard — see `release_dependents` for which hazard that guard
    // forces a choice between. A failure here is logged, not
    // propagated: the poll owes the remaining features their turn.
    if new_state == "merged" || new_state == "closed" {
        if let Err(e) = crate::application::tickets::release::release_dependents(
            feature,
            &*ports.tickets,
            &*ports.discoveries,
            features,
            notifications,
            notif,
        ) {
            eprintln!(
                "[MrMonitor] ticket recompute failed for feature {}: {}",
                feature.id.0, e
            );
        }
    }
    Ok(())
}

/// Record a polled PR state on `feature`'s row: a merge completes the feature
/// and notifies once, any other state only updates the badge. The one writer
/// of a settled PR, for the monitor and for a runner told of a merge its own
/// monitor could not see.
pub fn record_mr_state(
    feature: &Feature,
    new_state: &str,
    features: &dyn FeatureRepository,
    notifications: &dyn NotificationRepository,
    notif: &dyn NotificationPort,
) -> Result<(), String> {
    if new_state == "merged" {
        return record_merged(feature, new_state, features, notifications, notif);
    }
    features.update(
        &feature.id,
        &FeaturePatch {
            mr_state: Some(Some(new_state.to_string())),
            ..Default::default()
        },
    )
}

fn record_merged(
    feature: &Feature,
    new_state: &str,
    features: &dyn FeatureRepository,
    notifications: &dyn NotificationRepository,
    notif: &dyn NotificationPort,
) -> Result<(), String> {
    if notifications
        .list(Some(&feature.project_id), u32::MAX)?
        .iter()
        .any(|notification| {
            notification.feature_id == feature.id.0
                && notification.kind == NotificationKind::MrMerged
        })
    {
        return Ok(());
    }

    // 1. Update the feature row. `status = completed` is the
    //    user-visible terminal state; `mr_state = merged` is the
    //    driver.
    features.update(
        &feature.id,
        &FeaturePatch {
            status: Some("completed".to_string()),
            mr_state: Some(Some(new_state.to_string())),
            ..Default::default()
        },
    )?;

    // 2. Persist a notification row.
    let url = feature.mr_url.clone().unwrap_or_default();
    let notification = Notification {
        id: format!("notif-{}", crate::paths::now_ms()),
        project_id: feature.project_id.0.clone(),
        feature_id: feature.id.0.clone(),
        kind: NotificationKind::MrMerged,
        message: format!("MR for '{}' was merged", feature.title),
        feature_url: Some(format!(
            "/projects/{}/features/{}",
            feature.project_id.0, feature.id.0
        )),
        read: false,
        created_at: crate::paths::now_ms(),
    };
    notifications.add(notification)?;

    // 3. Push the live event to the bell + toast.
    let _ = notif.emit(&DomainEvent::MrMerged {
        feature_id: FeatureId::from(feature.id.0.clone()),
        project_id: feature.project_id.0.clone(),
        feature_title: feature.title.clone(),
        mr_url: url,
    });

    Ok(())
}

#[cfg(test)]
#[path = "../../tests/infrastructure/mr_monitor.rs"]
mod tests;
