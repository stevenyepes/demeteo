//! Appends the `feature_status` rows a feature's log is missing, so every
//! run that has ended reads as ended on a paired Hub.
//!
//! The rule is [`status_backfill`]; why the rows were missing is recorded on
//! [`crate::domain::hub_socket`]'s `run_end`. The paths that dropped them now
//! emit, so on a healthy instance this finds nothing — it exists to repair
//! logs written before they did, and runs at every start because it is cheap
//! and idempotent: a row it appends is the latest one the next pass reads.
//!
//! Rows go straight to the log rather than through the `NotificationPort`.
//! This repairs a record; nothing changed state, and the UI already shows
//! each feature's row as it is.

use crate::adapters::run_event_log::run_event_record;
use crate::domain::hub_socket::status_backfill;
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::notification::DomainEvent;
use crate::ports::run_events::RunEventsPort;

/// How many rows were appended.
pub fn backfill_run_ends(
    projects: &dyn ProjectRepository,
    features: &dyn FeatureRepository,
    run_events: &dyn RunEventsPort,
    now: i64,
) -> Result<usize, String> {
    let mut appended = 0;
    for project in projects.get_projects()? {
        for feature in features.get_all_for_project(&project.id)? {
            let log = run_events.list_since(feature.id.as_str(), 0)?;
            let logged = log
                .iter()
                .rev()
                .filter(|row| row.kind == "feature_status")
                .find_map(|row| logged_status(row.payload_json.as_deref()));
            let Some(status) = status_backfill(&feature.status, logged.as_deref(), !log.is_empty())
            else {
                continue;
            };
            let Some(record) = run_event_record(&DomainEvent::FeatureStatusChanged {
                feature_id: feature.id.clone(),
                status: status.to_string(),
            }) else {
                continue;
            };
            run_events.append(
                &record.feature_id,
                record.kind,
                Some(&record.payload.to_string()),
                now,
            )?;
            appended += 1;
        }
    }
    Ok(appended)
}

fn logged_status(payload: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload?).ok()?;
    value.get("status")?.as_str().map(str::to_owned)
}

#[cfg(test)]
#[path = "../../tests/application/run_end_backfill.rs"]
mod tests;
