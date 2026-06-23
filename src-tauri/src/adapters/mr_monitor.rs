use crate::domain::ids::FeatureId;
use crate::domain::models::{Notification, NotificationKind};
use crate::ports::db::{FeaturePatch, FeatureRepository, NotificationRepository};
use crate::ports::mr_publisher::MrPublisher;
use crate::ports::notification::{DomainEvent, NotificationPort};
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

pub fn start_mr_monitor(
    features: Arc<dyn FeatureRepository>,
    mr_publisher: Arc<dyn MrPublisher>,
    notifications: Arc<dyn NotificationRepository>,
    notif: Arc<dyn NotificationPort>,
) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(MR_POLL_INTERVAL);
        // Skip the immediate first tick so app launch doesn't fire
        // a burst of API calls before the user has interacted.
        interval.tick().await;
        loop {
            interval.tick().await;
            eprintln!("[MrMonitor] tick — polling open MRs");
            if let Err(e) =
                check_mr_states(&*features, &*mr_publisher, &*notifications, &*notif).await
            {
                eprintln!("[MrMonitor] poll error: {}", e);
            }
        }
    });
}

async fn check_mr_states(
    features: &dyn FeatureRepository,
    mr_publisher: &dyn MrPublisher,
    notifications: &dyn NotificationRepository,
    notif: &dyn NotificationPort,
) -> Result<(), String> {
    let open = features.list_with_open_mr()?;
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

        if new_state == "merged" {
            eprintln!(
                "[MrMonitor] feature {} transitioned to merged — recording",
                feature.id.0
            );
            record_merged(feature, &new_state, features, notifications, notif)?;
        } else {
            eprintln!("[MrMonitor] feature {} state = {}", feature.id.0, new_state);
        }
        if new_state != "open" && new_state != "merged" {
            // `closed` / `draft` aren't notifications, just keep
            // the column in sync so the UI badge reflects reality.
            let _ = features.update(
                &feature.id,
                &FeaturePatch {
                    mr_state: Some(Some(new_state)),
                    ..Default::default()
                },
            );
        }
    }
    Ok(())
}

fn record_merged(
    feature: &crate::domain::models::Feature,
    new_state: &str,
    features: &dyn FeatureRepository,
    notifications: &dyn NotificationRepository,
    notif: &dyn NotificationPort,
) -> Result<(), String> {
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
