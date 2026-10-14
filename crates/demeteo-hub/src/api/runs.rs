//! The tenant's Features: `GET /v1/runs` lists them across the fleet,
//! `GET /v1/instances/{instance_id}/runs/{feature_id}` shows one.
//!
//! Both are the instance's run mirror and nothing else, so both fall under the
//! [module's read-scope rule](super): an instance named without `read` is
//! refused by name, and one the unfiltered list would read is passed over.

use super::fleet::{granted, parsed_instance, RunSummary};
use super::run_state::{by_feature, fold, FeatureStatus, PendingGate, StepState, TicketProgress};
use super::scope::read_verdict;
use super::{ApiContext, PostgresApi};
use crate::auth::handlers::{store_failed, unix_now, ApiError, CurrentSession};
use crate::routes::AppState;
use crate::store::instances::Instance;
use crate::store::UserId;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::Json;
use demeteo_hub_protocol::{RunEvent, RunEventKind};
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};

/// The query of `GET /v1/runs`, as text. [`runs`] parses each value, so a
/// request without a session is `401` whatever its query holds.
#[derive(Debug, Default, Deserialize)]
pub struct RunsFilter {
    pub instance: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RunsView {
    pub runs: Vec<RunSummary>,
}

#[derive(Debug, Serialize)]
pub struct RunDetailView {
    pub instance_id: String,
    pub feature_id: String,
    pub status: FeatureStatus,
    /// Always `null`: no mirrored event names the Workflow version a Feature
    /// runs (`docs/HUB.md` §5), and the field is held for the one that will.
    pub workflow: (),
    /// Ordered by `step_id`.
    pub steps: Vec<StepView>,
    pub tickets: TicketProgress,
    pub pending_gates: Vec<PendingGate>,
    /// The readable events, in offset order. An unreadable row is left out
    /// for the reason [`by_feature`] gives.
    pub events: Vec<EventView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepView {
    pub step_id: String,
    pub state: StepState,
}

/// A mirrored event, field by field. `occurred_at` is Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventView {
    pub offset: u64,
    pub kind: RunEventKind,
    pub occurred_at: u64,
    pub step_id: Option<String>,
    pub step_execution_id: Option<String>,
    pub ticket_id: Option<String>,
}

/// Every Feature of the instances `filter` names, in the order the store
/// lists instances and then by `feature_id`. Without `instance` that is
/// each instance reporting `read`; with it, that one instance, which must be
/// the tenant's and report `read`. A Feature with no readable event is not
/// listed, as [`RunSummary::of`] has nothing to say of it.
pub async fn runs(
    api: &ApiContext<'_>,
    user: UserId,
    filter: RunsFilter,
    _now: u64,
) -> Result<RunsView, ApiError> {
    let instances = match filter.instance.as_deref() {
        Some(instance_id) => {
            let row = owned(api, user, instance_id).await?;
            read_verdict(&granted(&row))?;
            vec![row]
        }
        None => {
            let mut rows = api.store.instances(user).await.map_err(store_failed)?;
            rows.retain(|row| read_verdict(&granted(row)).is_ok());
            rows
        }
    };
    let status = match filter.status.as_deref() {
        Some(name) => Some(status_named(name).ok_or(ApiError::INVALID_REQUEST)?),
        None => None,
    };

    let mut runs = Vec::new();
    for row in instances {
        let mirror = api
            .store
            .mirror(user, row.id, None)
            .await
            .map_err(store_failed)?;
        runs.extend(
            by_feature(&mirror)
                .iter()
                .filter_map(|(feature_id, events)| RunSummary::of(row.id, feature_id, events))
                .filter(|run| status.is_none_or(|status| run.status == status)),
        );
    }
    Ok(RunsView { runs })
}

/// A Feature the instance mirrored no readable event of is `404 not_found`,
/// but only once `read` is granted: before it, the answer would tell which
/// Features exist.
pub async fn run_detail(
    api: &ApiContext<'_>,
    user: UserId,
    instance_id: &str,
    feature_id: &str,
    _now: u64,
) -> Result<RunDetailView, ApiError> {
    let row = owned(api, user, instance_id).await?;
    read_verdict(&granted(&row))?;
    let mirror = api
        .store
        .mirror(user, row.id, Some(feature_id))
        .await
        .map_err(store_failed)?;
    let events = by_feature(&mirror)
        .remove(feature_id)
        .ok_or(ApiError::NOT_FOUND)?;
    Ok(detail(&row, feature_id, &events))
}

pub async fn get_runs(
    State(state): State<AppState>,
    current: CurrentSession,
    query: Result<Query<RunsFilter>, QueryRejection>,
) -> Result<Json<RunsView>, ApiError> {
    let Query(filter) = query.map_err(|_| ApiError::INVALID_REQUEST)?;
    let store = PostgresApi::new(state.db().clone());
    let api = ApiContext::of(&state, &store);
    runs(&api, current.user, filter, unix_now()).await.map(Json)
}

/// The ids are taken as text after the session, so a request without one is
/// `401` before its path is read at all.
pub async fn get_run(
    State(state): State<AppState>,
    current: CurrentSession,
    Path((instance_id, feature_id)): Path<(String, String)>,
) -> Result<Json<RunDetailView>, ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let api = ApiContext::of(&state, &store);
    run_detail(&api, current.user, &instance_id, &feature_id, unix_now())
        .await
        .map(Json)
}

/// The tenant's instance named by `instance_id`, which is `404 not_found`
/// alike when it is another tenant's, unknown, or no UUID at all.
pub(super) async fn owned(
    api: &ApiContext<'_>,
    user: UserId,
    instance_id: &str,
) -> Result<Instance, ApiError> {
    let id = parsed_instance(instance_id).ok_or(ApiError::NOT_FOUND)?;
    api.store
        .instance(user, id)
        .await
        .map_err(store_failed)?
        .ok_or(ApiError::NOT_FOUND)
}

/// Spelled as the status is answered, so a filter is the value a client read.
fn status_named(name: &str) -> Option<FeatureStatus> {
    let text: StrDeserializer<'_, ValueError> = name.into_deserializer();
    FeatureStatus::deserialize(text).ok()
}

fn detail(row: &Instance, feature_id: &str, events: &[RunEvent]) -> RunDetailView {
    let state = fold(events);
    RunDetailView {
        instance_id: row.id.0.to_string(),
        feature_id: feature_id.to_owned(),
        status: state.status,
        workflow: (),
        steps: state
            .steps
            .into_iter()
            .map(|(step_id, state)| StepView { step_id, state })
            .collect(),
        tickets: state.tickets,
        pending_gates: state.pending_gates,
        events: events
            .iter()
            .map(|event| EventView {
                offset: event.offset,
                kind: event.kind,
                occurred_at: event.occurred_at,
                step_id: event.payload.step_id.clone(),
                step_execution_id: event.payload.step_execution_id.clone(),
                ticket_id: event.payload.ticket_id.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_filter_is_spelled_as_the_status_is_answered() {
        for status in [
            FeatureStatus::Running,
            FeatureStatus::Completed,
            FeatureStatus::Failed,
            FeatureStatus::Cancelled,
        ] {
            let answered = serde_json::to_value(status).unwrap();
            let name = answered.as_str().unwrap();
            assert_eq!(status_named(name), Some(status), "{name}");
        }
        for not_a_status in ["", "Running", "RUNNING", "pending", " running"] {
            assert_eq!(status_named(not_a_status), None, "{not_a_status:?}");
        }
    }
}
