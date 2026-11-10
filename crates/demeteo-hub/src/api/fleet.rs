//! The tenant's instances: `GET /v1/fleet` lists them with what waits on a
//! human, `GET /v1/instances/{instance_id}` shows one.
//!
//! What either withholds without `read`, and why a missing scope is not an
//! error here, is the [module's read-scope rule](super).

use super::run_labels::RunLabels;
use super::run_state::{by_feature, fold, FeatureStatus, PendingGate, TicketProgress};
use super::scope::read_verdict;
use super::snapshot_view::{harness_catalog, run_shape, workflows};
use super::{ApiContext, PostgresApi, SnapshotRead};
use crate::attachments::postgres::PostgresAttachments;
use crate::auth::handlers::{store_failed, unix_now, ApiError, CurrentSession};
use crate::pairing::authorize::scope_named;
use crate::routes::AppState;
use crate::store::instances::{Instance, InstanceKind};
use crate::store::{InstanceId, UserId};
use axum::extract::{Path, State};
use axum::Json;
use demeteo_hub_protocol::{RunEvent, RunEventKind, Scope, Snapshot};
use serde::Serialize;
use time::OffsetDateTime;
use uuid::Uuid;

pub use super::snapshot_view::{
    GraphEdgeView, GraphNodeView, GraphPosition, HarnessCatalogView, ModelChoiceView, RunShapeView,
    WorkflowGraphView, WorkflowView,
};

#[derive(Debug, Serialize)]
pub struct FleetView {
    pub instances: Vec<FleetInstanceView>,
}

/// Every `Option` is answered as `null`, never left out: a field the Hub
/// withholds stays distinguishable from one a client does not know.
#[derive(Debug, Serialize)]
pub struct FleetInstanceView {
    pub id: String,
    /// `None` only for a stored kind this build does not know.
    pub kind: Option<InstanceKind>,
    pub label: Option<String>,
    pub present: bool,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
    pub scopes: Vec<Scope>,
    pub read_granted: bool,
    pub last_seen: Option<u64>,
    /// `None` without `read`.
    pub live_runs: Option<u32>,
    /// `None` without `read`.
    pub needs_you: Option<Vec<NeedsYou>>,
    /// The harness kinds the instance registered. `None` without `read`,
    /// before a snapshot, and from a desktop that does not report them.
    pub harnesses: Option<Vec<String>>,
    /// `None` without `read` and before a readable snapshot.
    pub project_count: Option<u32>,
}

/// A Gate of a running Feature waiting on a decision. `since` is when it
/// began waiting, in Unix seconds. The three labels are
/// [`RunLabels`]' and `None` when the snapshot gives none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NeedsYou {
    pub instance_id: String,
    pub feature_id: String,
    pub step_id: Option<String>,
    pub step_execution_id: String,
    pub since: u64,
    pub title: Option<String>,
    pub project_name: Option<String>,
    pub step_title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InstanceDetailView {
    pub id: String,
    /// As [`FleetInstanceView::kind`].
    pub kind: Option<InstanceKind>,
    pub present: bool,
    pub connected_at: Option<u64>,
    pub last_seen: Option<u64>,
    pub install_id: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
    pub scopes: Vec<Scope>,
    pub read_granted: bool,
    /// `None` without `read`; empty until the instance sends a snapshot, and
    /// while the one it sent is unreadable.
    pub projects: Option<Vec<ProjectView>>,
    /// Graph-only, by the [`snapshot_view`](super::snapshot_view) rule.
    /// `None` and empty as `projects` is.
    pub workflows: Option<Vec<WorkflowView>>,
    /// `None` and empty as `projects` is.
    pub run_shape: Option<Vec<RunShapeView>>,
    /// `None` without `read`.
    pub live_runs: Option<Vec<RunSummary>>,
    /// As [`FleetInstanceView::harnesses`].
    pub harnesses: Option<Vec<String>>,
    /// The harnesses the instance found installed and the models each
    /// lists. `None` without `read`, before a snapshot, and from an instance
    /// that predates the field or has not finished a probe; `Some([])` when it
    /// probed and found none installed.
    pub harness_catalog: Option<Vec<HarnessCatalogView>>,
}

/// A project as the snapshot names it. Built field by field: the snapshot is
/// the desktop's own description, and only these four are the Hub's to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub remote_url: Option<String>,
    /// As [`ProjectSnapshot::provider_kind`](crate::protocol::ProjectSnapshot::provider_kind):
    /// what lets the browser tell, before it dispatches, a Project a runner
    /// cannot start from one it can (`docs/HUB.md` §6.4).
    pub provider_kind: Option<String>,
}

/// One Feature of one instance, from its mirrored events and the labels of
/// its instance's snapshot. Times are Unix seconds; `started_at` is `None`
/// while no `feature_started` was mirrored. `step_id` is the Step the run
/// last entered ([`RunState::current_step`](super::run_state::RunState)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunSummary {
    pub instance_id: String,
    pub feature_id: String,
    pub status: FeatureStatus,
    pub started_at: Option<u64>,
    pub updated_at: u64,
    /// Always `null`: the Hub stores nothing about cost (`docs/HUB.md` §5).
    pub cost_cents: (),
    pub title: Option<String>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub step_id: Option<String>,
    pub step_title: Option<String>,
    pub tickets: TicketProgress,
}

impl RunSummary {
    /// `None` for a Feature with no event, which has nothing to summarize.
    pub fn of(
        instance: InstanceId,
        feature_id: &str,
        events: &[RunEvent],
        labels: &RunLabels,
    ) -> Option<Self> {
        let last = events.last()?;
        let started_at = events
            .iter()
            .find(|event| event.kind == RunEventKind::FeatureStarted)
            .map(|event| event.occurred_at);
        let state = fold(events);
        Some(Self {
            instance_id: instance.0.to_string(),
            feature_id: feature_id.to_owned(),
            status: state.status,
            started_at,
            updated_at: last.occurred_at,
            cost_cents: (),
            title: labels.title(feature_id),
            project_id: labels.project_id(feature_id),
            project_name: labels.project_name(feature_id),
            step_title: labels.step_title(feature_id, state.current_step.as_deref()),
            step_id: state.current_step,
            tickets: state.tickets,
        })
    }
}

impl NeedsYou {
    fn of(instance: InstanceId, gate: PendingGate, labels: &RunLabels) -> Self {
        Self {
            instance_id: instance.0.to_string(),
            title: labels.title(&gate.feature_id),
            project_name: labels.project_name(&gate.feature_id),
            step_title: labels.step_title(&gate.feature_id, gate.step_id.as_deref()),
            feature_id: gate.feature_id,
            step_id: gate.step_id,
            step_execution_id: gate.step_execution_id,
            since: gate.since,
        }
    }
}

/// `_now` is taken as every use-case takes it, though nothing shown here
/// depends on the clock yet.
pub async fn fleet(api: &ApiContext<'_>, user: UserId, _now: u64) -> Result<FleetView, ApiError> {
    let rows = api.store.instances(user).await.map_err(store_failed)?;
    let mut instances = Vec::with_capacity(rows.len());
    for row in rows {
        let scopes = granted(&row);
        let read_granted = read_verdict(&scopes).is_ok();
        let (live_runs, needs_you, harnesses, project_count) = if read_granted {
            let snapshot = snapshot_of(api, user, row.id).await?;
            let labels = RunLabels::of(snapshot.as_ref());
            let runs = runs_of(api, user, row.id, &labels).await?;
            (
                Some(count(runs.live.len())),
                Some(runs.needs_you),
                snapshot.as_ref().and_then(|s| s.harnesses.clone()),
                snapshot.as_ref().map(|s| count(s.projects.len())),
            )
        } else {
            (None, None, None, None)
        };
        instances.push(FleetInstanceView {
            id: row.id.0.to_string(),
            kind: InstanceKind::parse(&row.kind),
            label: non_empty(&row.name),
            present: present(api, &row),
            os: non_empty(&row.os),
            arch: non_empty(&row.arch),
            version: non_empty(&row.version),
            scopes,
            read_granted,
            last_seen: row.last_seen.and_then(unix_seconds),
            live_runs,
            needs_you,
            harnesses,
            project_count,
        });
    }
    Ok(FleetView { instances })
}

/// An `instance_id` that is no UUID answers as one that is not the tenant's:
/// both are `404 not_found`, by the [tenant rule](super).
pub async fn instance_detail(
    api: &ApiContext<'_>,
    user: UserId,
    instance_id: &str,
    _now: u64,
) -> Result<InstanceDetailView, ApiError> {
    let id = parsed_instance(instance_id).ok_or(ApiError::NOT_FOUND)?;
    let row = api
        .store
        .instance(user, id)
        .await
        .map_err(store_failed)?
        .ok_or(ApiError::NOT_FOUND)?;
    let scopes = granted(&row);
    let read_granted = read_verdict(&scopes).is_ok();
    let (reported, live_runs) = if read_granted {
        let snapshot = snapshot_of(api, user, id).await?;
        let labels = RunLabels::of(snapshot.as_ref());
        let live = runs_of(api, user, id, &labels).await?.live;
        (Some(reported_of(snapshot)), Some(live))
    } else {
        (None, None)
    };
    let (projects, workflows, run_shape, harnesses, harness_catalog) = match reported {
        Some(reported) => (
            Some(reported.projects),
            Some(reported.workflows),
            Some(reported.run_shape),
            reported.harnesses,
            reported.harness_catalog,
        ),
        None => (None, None, None, None, None),
    };
    Ok(InstanceDetailView {
        id: row.id.0.to_string(),
        kind: InstanceKind::parse(&row.kind),
        present: present(api, &row),
        connected_at: row.connected_at.and_then(unix_seconds),
        last_seen: row.last_seen.and_then(unix_seconds),
        install_id: row.install_id.clone(),
        os: non_empty(&row.os),
        arch: non_empty(&row.arch),
        version: non_empty(&row.version),
        scopes,
        read_granted,
        projects,
        workflows,
        run_shape,
        live_runs,
        harnesses,
        harness_catalog,
    })
}

pub async fn get_fleet(
    State(state): State<AppState>,
    current: CurrentSession,
) -> Result<Json<FleetView>, ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    fleet(&api, current.user, unix_now()).await.map(Json)
}

/// The id is a `String` taken after the session, so a request without one is
/// `401` before its path is read at all.
pub async fn get_instance(
    State(state): State<AppState>,
    current: CurrentSession,
    Path(instance_id): Path<String>,
) -> Result<Json<InstanceDetailView>, ApiError> {
    let store = PostgresApi::new(state.db().clone());
    let attachments = PostgresAttachments::new(state.db().clone());
    let api = ApiContext::of(&state, &store, &attachments);
    instance_detail(&api, current.user, &instance_id, unix_now())
        .await
        .map(Json)
}

struct Runs {
    live: Vec<RunSummary>,
    needs_you: Vec<NeedsYou>,
}

async fn runs_of(
    api: &ApiContext<'_>,
    user: UserId,
    id: InstanceId,
    labels: &RunLabels,
) -> Result<Runs, ApiError> {
    let mirror = api
        .store
        .mirror(user, id, None)
        .await
        .map_err(store_failed)?;
    let mut runs = Runs {
        live: Vec::new(),
        needs_you: Vec::new(),
    };
    for (feature_id, events) in by_feature(&mirror) {
        let state = fold(&events);
        if state.status != FeatureStatus::Running {
            continue;
        }
        runs.needs_you.extend(
            state
                .pending_gates
                .into_iter()
                .map(|gate| NeedsYou::of(id, gate, labels)),
        );
        runs.live
            .extend(RunSummary::of(id, &feature_id, &events, labels));
    }
    Ok(runs)
}

#[derive(Default)]
struct Reported {
    projects: Vec<ProjectView>,
    workflows: Vec<WorkflowView>,
    run_shape: Vec<RunShapeView>,
    harnesses: Option<Vec<String>>,
    harness_catalog: Option<Vec<HarnessCatalogView>>,
}

/// The instance's snapshot, or `None` before one and while the one it sent
/// is unreadable: both show nothing it reported.
pub(super) async fn snapshot_of(
    api: &ApiContext<'_>,
    user: UserId,
    id: InstanceId,
) -> Result<Option<Snapshot>, ApiError> {
    let snapshot = api.store.snapshot(user, id).await.map_err(store_failed)?;
    Ok(match snapshot {
        Some(SnapshotRead::Readable(snapshot)) => Some(*snapshot),
        Some(SnapshotRead::Unreadable) => {
            tracing::warn!(instance = %id.0, "a stored snapshot is unreadable");
            None
        }
        None => None,
    })
}

fn reported_of(snapshot: Option<Snapshot>) -> Reported {
    let Some(snapshot) = snapshot else {
        return Reported::default();
    };
    Reported {
        projects: snapshot
            .projects
            .into_iter()
            .map(|project| ProjectView {
                id: project.id,
                name: project.name,
                remote_url: project.remote_url,
                provider_kind: project.provider_kind,
            })
            .collect(),
        workflows: workflows(snapshot.workflows),
        run_shape: run_shape(snapshot.run_shape),
        harnesses: snapshot.harnesses,
        harness_catalog: snapshot.harness_catalog.map(harness_catalog),
    }
}

/// The row's `connection_id` alone outlives a restart that dropped every
/// socket; see [`Connections::is_live`](crate::socket::connections::Connections::is_live).
fn present(api: &ApiContext<'_>, row: &Instance) -> bool {
    row.connection_id.is_some() && api.connections.is_live(row.id)
}

/// A stored name that is no [`Scope`] is left out, which can only withhold
/// more. [`granted_scopes`](crate::store::instances::granted_scopes) refuses
/// one instead, because a short list there would pass for a narrower grant
/// than the one written; here a narrower one is the safe reading.
pub(super) fn granted(row: &Instance) -> Vec<Scope> {
    row.granted_scopes
        .iter()
        .filter_map(|name| scope_named(name))
        .collect()
}

pub(super) fn parsed_instance(text: &str) -> Option<InstanceId> {
    Uuid::try_parse(text).ok().map(InstanceId)
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

fn unix_seconds(at: OffsetDateTime) -> Option<u64> {
    u64::try_from(at.unix_timestamp()).ok()
}

fn count(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use demeteo_hub_protocol::{Effort, HarnessCatalogEntry, ModelChoice, ProjectSnapshot};

    fn project(provider_kind: Option<&str>) -> ProjectSnapshot {
        ProjectSnapshot {
            id: "p".to_owned(),
            name: "Widgets".to_owned(),
            remote_url: Some("https://gitlab.example.com/acme/widgets".to_owned()),
            provider_kind: provider_kind.map(str::to_owned),
        }
    }

    fn snapshot(projects: Vec<ProjectSnapshot>) -> Snapshot {
        Snapshot {
            v: 1,
            projects,
            workflows: Vec::new(),
            run_shape: Vec::new(),
            features: Vec::new(),
            harnesses: None,
            harness_catalog: None,
        }
    }

    #[test]
    fn a_harness_catalog_reaches_the_view_field_by_field() {
        let snapshot = Snapshot {
            harness_catalog: Some(vec![
                HarnessCatalogEntry {
                    kind: "codex".to_owned(),
                    models: vec![ModelChoice {
                        value: "gpt-5.5".to_owned(),
                        name: "GPT-5.5".to_owned(),
                    }],
                    effort_levels: Some(vec![Effort::Low, Effort::High]),
                    source: Some("bundled".to_owned()),
                    probed_at: 1_700_000_000,
                },
                HarnessCatalogEntry {
                    kind: "opencode".to_owned(),
                    models: Vec::new(),
                    effort_levels: None,
                    source: None,
                    probed_at: 1_700_000_001,
                },
            ]),
            ..snapshot(Vec::new())
        };
        let catalog = reported_of(Some(snapshot)).harness_catalog;
        assert_eq!(
            catalog,
            Some(vec![
                HarnessCatalogView {
                    kind: "codex".to_owned(),
                    models: vec![ModelChoiceView {
                        value: "gpt-5.5".to_owned(),
                        name: "GPT-5.5".to_owned(),
                    }],
                    effort_levels: Some(vec![Effort::Low, Effort::High]),
                    source: Some("bundled".to_owned()),
                    probed_at: 1_700_000_000,
                },
                HarnessCatalogView {
                    kind: "opencode".to_owned(),
                    models: Vec::new(),
                    effort_levels: None,
                    source: None,
                    probed_at: 1_700_000_001,
                },
            ])
        );
    }

    #[test]
    fn an_empty_catalog_stays_distinct_from_none() {
        let probed_empty = Snapshot {
            harness_catalog: Some(Vec::new()),
            ..snapshot(Vec::new())
        };
        assert_eq!(
            reported_of(Some(probed_empty)).harness_catalog,
            Some(Vec::new())
        );
        assert_eq!(
            reported_of(Some(snapshot(Vec::new()))).harness_catalog,
            None
        );
        assert_eq!(reported_of(None).harness_catalog, None);
    }

    #[test]
    fn a_projects_provider_kind_reaches_the_view_as_the_desktop_reported_it() {
        let snapshot = snapshot(vec![project(Some("gitlab")), project(None)]);
        let kinds: Vec<Option<String>> = reported_of(Some(snapshot))
            .projects
            .into_iter()
            .map(|view| view.provider_kind)
            .collect();
        assert_eq!(kinds, [Some("gitlab".to_owned()), None]);
    }
}
