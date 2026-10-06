// Tests extracted from `crates/demeteo-core/src/application/tickets/launch.rs`
// (mirrored-tests convention). `super` = that module.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::discovery::{create as open_discovery, NewDiscovery};
use crate::application::launch::tests::{harness, Harness, MirrorFailingOn, MirrorWrite, RunnerAt};
use crate::application::remote_runs::{
    find_mirror_for_feature, retry_remote_step, RemoteRewind, RewindOverrides,
};
use crate::application::tickets::{board, node_of, TicketView};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{DiscoveryId, FeatureId, MachineId, ProjectId, TicketId, WorkflowId};
use crate::domain::models::{EffortLevel, Feature, Project};
use crate::domain::ticket_graph::{derive_board, TicketLane};
use crate::ports::db::FeaturePatch;
use crate::ports::discovery::{DiscoveryPatch, TicketPort};
use crate::ports::step_executor::StepExecutor;

fn ticket(id: &str, seq: i64, title: &str) -> Ticket {
    Ticket {
        id: TicketId::from(id.to_string()),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        seq,
        title: title.to_string(),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        machine_id: None,
        attachments: Vec::new(),
        state: TicketState::Unstarted,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn refusal_for(subject_index: usize, tickets: &[Ticket]) -> Option<String> {
    let nodes: Vec<_> = tickets
        .iter()
        .map(|t| {
            let state = match t.state {
                TicketState::Started => Some("open"),
                _ => None,
            };
            node_of(t, state)
        })
        .collect();
    let board = derive_board(&nodes);
    start_refusal(
        &tickets[subject_index],
        &board.standings[subject_index],
        tickets,
    )
}

/// §7.1/§11: Demeteo shows what is startable and starts nothing itself, so the
/// refusal is the only thing standing between an unmet edge and a run cut from
/// a base branch that lacks the prerequisite's code.
#[test]
fn an_unmet_edge_refuses_the_start_and_names_the_blocker() {
    let mut dependent = ticket("t-2", 2, "the multiplexer");
    dependent.blocked_by = vec![TicketId::from("t-1".to_string())];
    let tickets = vec![ticket("t-1", 1, "the registry"), dependent];

    let refusal = refusal_for(1, &tickets).expect("an outstanding edge must refuse");
    assert!(refusal.contains("#2"), "{refusal}");
    assert!(refusal.contains("#1 \"the registry\""), "{refusal}");
    assert!(refusal.contains("force start"), "{refusal}");
}

/// §6.5's hatch is per ticket, so it clears every edge at once — including in
/// the case it exists for, a project with no forge where no edge will ever be
/// satisfied on its own.
#[test]
fn a_recorded_reason_clears_every_edge_at_once() {
    let mut dependent = ticket("t-3", 3, "conformance");
    dependent.blocked_by = vec![
        TicketId::from("t-1".to_string()),
        TicketId::from("t-2".to_string()),
    ];
    dependent.force_start_reason = Some("this project has no forge remote".to_string());
    let tickets = vec![
        ticket("t-1", 1, "the registry"),
        ticket("t-2", 2, "the keypair"),
        dependent,
    ];

    assert_eq!(refusal_for(2, &tickets), None);
}

#[test]
fn a_dropped_ticket_and_a_started_one_are_refused_for_their_own_reasons() {
    let mut dropped = ticket("t-1", 1, "the guide");
    dropped.state = TicketState::Dropped;
    let mut started = ticket("t-2", 2, "the registry");
    started.state = TicketState::Started;
    let tickets = vec![dropped, started];

    let d = refusal_for(0, &tickets).expect("a dropped ticket has nothing to start");
    assert!(d.contains("dropped"), "{d}");
    let s = refusal_for(1, &tickets).expect("a started ticket is already running");
    assert!(s.contains("already been started"), "{s}");
}

/// An unknown edge target is drift, not a plan (`BlockerReason::Unknown`), and
/// the refusal must not present it as a ticket the user could go and finish.
#[test]
fn an_edge_naming_nothing_refuses_and_says_so() {
    let mut dependent = ticket("t-2", 2, "the multiplexer");
    dependent.blocked_by = vec![TicketId::from("t-gone".to_string())];
    let tickets = vec![dependent];

    let refusal = refusal_for(0, &tickets).expect("a dangling edge blocks");
    assert!(
        refusal.contains("'t-gone' (not a ticket in this plan)"),
        "{refusal}"
    );
}

/// §7.2: the briefing rides in the launched prompt itself, not beside it.
#[test]
fn the_launch_description_carries_the_briefing_and_the_ticket_fields() {
    let mut t = ticket("t-1", 1, "the registry");
    t.description = "Key sessions by client id.".to_string();
    t.acceptance = vec!["two clients share one runner".to_string()];
    t.files = vec!["crates/demeteo-runner/src/session.rs".to_string()];
    t.test_command = Some("npm run checks:code".to_string());

    let body = launch_description(&t, "#0 \"the port\" landed.");
    assert!(body.starts_with("Key sessions by client id."), "{body}");
    assert!(
        body.contains("## Acceptance criteria\n- two clients share one runner"),
        "{body}"
    );
    assert!(
        body.contains("`crates/demeteo-runner/src/session.rs`"),
        "{body}"
    );
    assert!(body.contains("`npm run checks:code`"), "{body}");
    assert!(
        body.trim_end().ends_with("#0 \"the port\" landed."),
        "{body}"
    );
}

/// A ticket whose description was never filled in still needs a prompt body —
/// `FeatureLaunch` refuses an empty one.
#[test]
fn an_empty_description_falls_back_to_the_title() {
    let t = ticket("t-1", 1, "the registry");
    assert!(launch_description(&t, "none").starts_with("the registry"));
}

/// A project with nothing in it, the same shape
/// `tests/application/discovery/mod.rs`'s `fixture` uses — as much as
/// `start` reads off a Project.
fn ctx_with_project(tag: &str) -> (AppContext, ProjectId) {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-launch-{tag}"));
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    let project_id = ProjectId::from(format!("p-{tag}"));
    ctx.projects
        .add(Project {
            id: project_id.clone(),
            name: "name fixture".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .expect("the project is stored");
    (ctx, project_id)
}

fn opening(project_id: &ProjectId, title: &str) -> NewDiscovery {
    NewDiscovery {
        project_id: project_id.as_str().to_string(),
        title: title.to_string(),
        agent_kind: "claude-code".to_string(),
        model: None,
        effort: None,
        machine_id: None,
        staged_attachments: Vec::new(),
    }
}

/// A startable Ticket wired to `discovery_id`, with a workflow chosen so
/// `start` never refuses for the one precondition this fixture is not
/// exercising.
fn launchable_ticket(discovery_id: &DiscoveryId) -> Ticket {
    let mut t = ticket("t-1", 1, "the ticket");
    t.discovery_id = discovery_id.clone();
    t.workflow_id = Some(WorkflowId::from("wf-1".to_string()));
    t
}

/// A [`StepExecutor`] spy that captures the [`FeatureLaunch`] `start` builds
/// and answers with a fixed `Feature` — the same capture-and-panic-on-rest
/// shape `tests/application/discovery/mod.rs`'s `FakeBranches` uses, narrowed
/// to the one method `start` calls.
struct SpyExecutor {
    captured: Mutex<Option<FeatureLaunch>>,
}

impl SpyExecutor {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            captured: Mutex::new(None),
        })
    }
}

#[async_trait]
impl StepExecutor for SpyExecutor {
    async fn feature_start(&self, launch: FeatureLaunch) -> Result<Feature, String> {
        let feature = Feature {
            id: crate::domain::ids::FeatureId::from("f-1".to_string()),
            project_id: ProjectId::from(launch.project_id.clone()),
            workflow_id: None,
            workflow_version_id: None,
            title: launch.title.clone(),
            description: launch.description.clone(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: String::new(),
            tokens: 0,
            created_at: 0,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: None,
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        };
        *self.captured.lock().expect("lock is not poisoned") = Some(launch);
        Ok(feature)
    }

    async fn feature_pause(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_resume(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_cancel(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_get(&self, _: &str) -> Result<crate::domain::models::StepExecution, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_retry(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), crate::error::AppError> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_set_assignment(
        &self,
        _: &str,
        _: crate::domain::step_assignment::StepAssignment,
    ) -> Result<(), crate::error::AppError> {
        panic!("unexpected StepExecutor call")
    }
    async fn replay_from_step(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_list_for_run(
        &self,
        _: &str,
    ) -> Result<Vec<crate::domain::models::StepExecution>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_sync(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_drift(
        &self,
        _: &str,
        _: bool,
    ) -> Result<crate::domain::models::FeatureDrift, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_reconcile(
        &self,
        _: &str,
        _: crate::domain::upstream_feature::DivergenceReconcile,
    ) -> Result<Option<crate::ports::sync_session::SyncSessionView>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_divergence(
        &self,
        _: &str,
    ) -> Result<Option<crate::domain::models::FeatureDivergence>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_resolve_sync_conflicts(
        &self,
        _: &str,
        _: &crate::domain::sync_resolver::SyncResolverChoice,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_continue_sync(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_sync_resolver(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncResolverView, String> {
        panic!("unexpected StepExecutor call")
    }
}

/// Acceptance criterion 6 / the launch half of criterion 1: a Discovery's
/// declared `base_branch` becomes the launched run's origin, and
/// `diff_base_branch` is left `None` — `FeatureOrigin::Branch { base
/// }.base_branch(None)` already answers `Some(base)`, and
/// `domain::diff_base::resolve` falls through to it, so writing the same
/// fact twice would only give it a second, staler copy.
#[tokio::test]
async fn starting_a_ticket_cuts_from_the_discoverys_base_branch() {
    let (mut ctx, project_id) = ctx_with_project("base-branch");
    let discovery = open_discovery(&ctx, opening(&project_id, "integration work"))
        .expect("the discovery opens");
    ctx.discoveries
        .update(
            &discovery.id,
            &DiscoveryPatch {
                base_branch: Some(Some("integration".to_string())),
                ..Default::default()
            },
            now_ms(),
        )
        .expect("the base branch is stored");
    let t = launchable_ticket(&discovery.id);
    ctx.tickets
        .upsert_batch(std::slice::from_ref(&t))
        .expect("the ticket is stored");
    let spy = SpyExecutor::new();
    ctx.executor = spy.clone();

    start(&ctx, &t.id, None).await.expect("the ticket starts");

    let launch = spy
        .captured
        .lock()
        .expect("lock is not poisoned")
        .clone()
        .expect("feature_start was called");
    assert_eq!(
        launch.origin,
        FeatureOrigin::Branch {
            base: "integration".to_string()
        }
    );
    assert_eq!(launch.diff_base_branch, None);
}

/// The other half of criterion 1: a Discovery with no declared base still
/// launches its tickets from the project's default branch.
#[tokio::test]
async fn starting_a_ticket_with_no_base_branch_uses_the_default_branch() {
    let (mut ctx, project_id) = ctx_with_project("no-base-branch");
    let discovery =
        open_discovery(&ctx, opening(&project_id, "no base")).expect("the discovery opens");
    let t = launchable_ticket(&discovery.id);
    ctx.tickets
        .upsert_batch(std::slice::from_ref(&t))
        .expect("the ticket is stored");
    let spy = SpyExecutor::new();
    ctx.executor = spy.clone();

    start(&ctx, &t.id, None).await.expect("the ticket starts");

    let launch = spy
        .captured
        .lock()
        .expect("lock is not poisoned")
        .clone()
        .expect("feature_start was called");
    assert_eq!(launch.origin, FeatureOrigin::DefaultBranch);
}

// --- AC4: a detached ticket starts like a local one -----------------------

/// A startable ticket in a Discovery on the harness's local project, with the
/// harness's workflow and `stored` as its own placement choice.
fn stored_ticket(h: &Harness, stored: Option<&str>) -> Ticket {
    let discovery = open_discovery(&h.ctx, opening(&ProjectId::from("p-1"), "placed work"))
        .expect("the discovery opens");
    let mut t = launchable_ticket(&discovery.id);
    t.workflow_id = Some(WorkflowId::from("w-1".to_string()));
    t.machine_id = stored.map(MachineId::from);
    h.ctx
        .tickets
        .upsert_batch(std::slice::from_ref(&t))
        .expect("the ticket is stored");
    t
}

fn reloaded(h: &Harness, t: &Ticket) -> Ticket {
    h.ctx
        .tickets
        .get(&t.id)
        .expect("the ticket reads")
        .expect("the ticket exists")
}

fn feature_rows(h: &Harness) -> Vec<String> {
    h.ctx
        .features
        .get_all_for_project(&ProjectId::from("p-1"))
        .expect("features read")
        .into_iter()
        .map(|f| f.id.0)
        .collect()
}

/// Started, pointing at `feature`, with the one current attempt on it, which
/// records it was placed on `placed`.
fn assert_started_on(h: &Harness, t: &Ticket, feature: &Feature, placed: &str) {
    let after = reloaded(h, t);
    assert_eq!(after.state, TicketState::Started);
    assert_eq!(after.feature_id.as_ref(), Some(&feature.id));
    let attempts = h.ctx.tickets.list_attempts(&t.id).expect("attempts read");
    assert_eq!(attempts.len(), 1, "{attempts:?}");
    assert_eq!(attempts[0].feature_id, feature.id);
    assert_eq!(attempts[0].superseded_at, None);
    assert_eq!(attempts[0].machine_id, Some(MachineId::from(placed)));
}

/// Started on the runner: the Feature is the shadow row the mirror points at,
/// and the step executor never saw the launch.
fn assert_started_detached(h: &Harness, t: &Ticket, feature: &Feature) {
    assert_started_on(h, t, feature, "runner-1");
    assert_eq!(feature_rows(h), std::slice::from_ref(&feature.id.0));
    let mirrors = h.ctx.remote_run_mirror.list().expect("mirror reads");
    assert_eq!(mirrors.len(), 1);
    assert_eq!(
        mirrors[0].feature_id.as_deref(),
        Some(feature.id.0.as_str())
    );
    assert!(h.spy.launched().is_none(), "the executor is not called");
}

/// A refused start is as if it had never been asked for.
fn assert_untouched(h: &Harness, t: &Ticket) {
    let after = reloaded(h, t);
    assert_eq!(after.state, TicketState::Unstarted);
    assert_eq!(after.feature_id, None);
    assert!(h
        .ctx
        .tickets
        .list_attempts(&t.id)
        .expect("attempts read")
        .is_empty());
    assert_eq!(feature_rows(h), Vec::<String>::new(), "no Feature row");
    assert!(h.spy.launched().is_none(), "the executor is not called");
}

#[tokio::test]
async fn a_stored_detached_ticket_starts_on_the_shadow_feature() {
    let h = harness(RunnerAt::accepting());
    let t = stored_ticket(&h, Some("runner-1"));

    let feature = start(&h.ctx, &t.id, None)
        .await
        .expect("the ticket starts")
        .feature;

    assert_started_detached(&h, &t, &feature);
}

#[tokio::test]
async fn an_override_places_one_launch_and_is_not_saved() {
    let h = harness(RunnerAt::accepting());
    let t = stored_ticket(&h, Some("local"));

    let feature = start(&h.ctx, &t.id, Some(MachineId::from("runner-1")))
        .await
        .expect("the ticket starts")
        .feature;

    assert_started_detached(&h, &t, &feature);
    assert_eq!(reloaded(&h, &t).machine_id, Some(MachineId::from("local")));
}

#[tokio::test]
async fn a_local_override_beats_a_stored_runner() {
    let h = harness(RunnerAt::accepting());
    let t = stored_ticket(&h, Some("runner-1"));

    let feature = start(&h.ctx, &t.id, Some(MachineId::from("local")))
        .await
        .expect("the ticket starts")
        .feature;

    assert!(h.spy.launched().is_some(), "the executor ran it");
    assert_eq!(h.exec.calls(), Vec::<String>::new(), "no RPC");
    assert_started_on(&h, &t, &feature, "local");
    assert_eq!(
        reloaded(&h, &t).machine_id,
        Some(MachineId::from("runner-1"))
    );
}

#[tokio::test]
async fn a_start_on_an_unknown_machine_leaves_no_trace() {
    let h = harness(RunnerAt::accepting());
    let t = stored_ticket(&h, Some("runner-gone"));

    let error = start(&h.ctx, &t.id, None)
        .await
        .expect_err("an unknown machine is refused");

    assert!(error.to_string().contains("runner-gone"), "{error}");
    assert_untouched(&h, &t);
}

#[tokio::test]
async fn a_start_on_an_incompatible_runner_leaves_no_trace_and_can_be_retried() {
    let mut h = harness(RunnerAt::reporting("1.2.0-30"));
    let t = stored_ticket(&h, Some("runner-1"));

    let error = start(&h.ctx, &t.id, None)
        .await
        .expect_err("an incompatible runner is refused");

    assert_eq!(error.code(), "runner_incompatible", "{error}");
    assert_untouched(&h, &t);

    h.ctx.exec = Arc::new(RunnerAt::accepting());
    let feature = start(&h.ctx, &t.id, None)
        .await
        .expect("the retry starts")
        .feature;
    assert_started_detached(&h, &t, &feature);
}

/// A ticket on the runner that is blocked by an unstarted sibling, so only a
/// force start can launch it.
fn blocked_detached_ticket(h: &Harness) -> Ticket {
    let blocker = stored_ticket(h, None);
    let mut t = blocker.clone();
    t.id = TicketId::from("t-2".to_string());
    t.seq = 2;
    t.blocked_by = vec![blocker.id.clone()];
    t.machine_id = Some(MachineId::from("runner-1"));
    h.ctx
        .tickets
        .upsert_batch(std::slice::from_ref(&t))
        .expect("the dependent is stored");
    t
}

#[tokio::test]
async fn a_detached_force_start_records_its_reason_and_starts() {
    let h = harness(RunnerAt::accepting());
    let t = blocked_detached_ticket(&h);

    let feature = force_start(&h.ctx, &t.id, "no forge remote", None)
        .await
        .expect("the force start launches")
        .feature;

    assert_started_detached(&h, &t, &feature);
    assert_eq!(
        reloaded(&h, &t).force_start_reason.as_deref(),
        Some("no forge remote")
    );
}

#[tokio::test]
async fn a_refused_detached_force_start_keeps_its_reason() {
    let h = harness(RunnerAt::reporting("1.2.0-32"));
    let t = blocked_detached_ticket(&h);

    let error = force_start(&h.ctx, &t.id, "no forge remote", None)
        .await
        .expect_err("an incompatible runner is refused");

    assert_eq!(error.code(), "runner_incompatible", "{error}");
    assert_untouched(&h, &t);
    assert_eq!(
        reloaded(&h, &t).force_start_reason.as_deref(),
        Some("no forge remote")
    );
}

/// D8: the runner holds the run, so recording nothing would let the user
/// submit it a second time. The parked PAT goes back with it, since nothing
/// else reports it before a reconcile.
#[tokio::test]
async fn parked_credentials_still_start_the_ticket() {
    let h = harness(RunnerAt {
        accepts_credentials: false,
        ..RunnerAt::accepting()
    });
    let t = stored_ticket(&h, Some("runner-1"));

    let launched = start(&h.ctx, &t.id, None)
        .await
        .expect("an accepted run is a started ticket");

    assert!(h
        .exec
        .calls()
        .contains(&"rpc inject_credentials".to_string()));
    assert_started_detached(&h, &t, &launched.feature);
    let reason = launched
        .credentials_parked
        .expect("the parked PAT is reported");
    assert!(reason.contains("inject_credentials"), "{reason}");
    assert_eq!(launched.mirror_unrecorded, None);
}

#[tokio::test]
async fn a_clean_detached_start_reports_nothing_degraded() {
    let h = harness(RunnerAt::accepting());
    let t = stored_ticket(&h, Some("runner-1"));

    let launched = force_start(&h.ctx, &t.id, "no forge remote", None)
        .await
        .expect("the ticket starts");

    assert_started_detached(&h, &t, &launched.feature);
    assert_eq!(launched.credentials_parked, None);
    assert_eq!(launched.mirror_unrecorded, None);
}

/// D8: the run is on the runner even when the laptop could not mirror it, so
/// the attempt is recorded and a second start is refused rather than paying
/// for the same ticket twice.
#[tokio::test]
async fn a_mirror_failure_still_starts_the_ticket_once() {
    for write in [MirrorWrite::Submitted, MirrorWrite::Status] {
        let mut h = harness(RunnerAt::accepting());
        h.ctx.remote_run_mirror = MirrorFailingOn::wrap(h.ctx.remote_run_mirror.clone(), write);
        let t = stored_ticket(&h, Some("runner-1"));

        let launched = start(&h.ctx, &t.id, None)
            .await
            .unwrap_or_else(|error| panic!("{write:?}: an accepted run starts, got {error}"));
        let reason = launched
            .mirror_unrecorded
            .unwrap_or_else(|| panic!("{write:?}: the mirror failure is reported"));
        assert!(reason.contains(write.method()), "{reason}");
        let feature = launched.feature;

        assert_started_on(&h, &t, &feature, "runner-1");
        assert_eq!(feature_rows(&h), std::slice::from_ref(&feature.id.0));
        let again = start(&h.ctx, &t.id, None)
            .await
            .expect_err("a started ticket is not submitted twice");
        assert!(
            again.to_string().contains("already been started"),
            "{again}"
        );
        let submits = h
            .exec
            .calls()
            .into_iter()
            .filter(|call| call == "rpc submit_run")
            .count();
        assert_eq!(submits, 1, "{write:?}");
    }
}

/// The context's own ticket store, except that recording a start fails — the
/// local SQLite failure that can land after the run has already launched.
struct StartUnrecordable {
    inner: Arc<dyn TicketPort>,
}

impl StartUnrecordable {
    fn wrap(inner: Arc<dyn TicketPort>) -> Arc<dyn TicketPort> {
        Arc::new(Self { inner })
    }
}

impl TicketPort for StartUnrecordable {
    fn list_for_discovery(&self, discovery_id: &DiscoveryId) -> Result<Vec<Ticket>, String> {
        self.inner.list_for_discovery(discovery_id)
    }
    fn get(&self, id: &TicketId) -> Result<Option<Ticket>, String> {
        self.inner.get(id)
    }
    fn upsert_batch(&self, tickets: &[Ticket]) -> Result<(), String> {
        self.inner.upsert_batch(tickets)
    }
    fn update(&self, id: &TicketId, patch: &TicketPatch, now: i64) -> Result<(), String> {
        self.inner.update(id, patch, now)
    }
    fn delete(&self, id: &TicketId) -> Result<(), String> {
        self.inner.delete(id)
    }
    fn next_seq(&self, discovery_id: &DiscoveryId) -> Result<i64, String> {
        self.inner.next_seq(discovery_id)
    }
    fn for_feature(&self, feature_id: &FeatureId) -> Result<Vec<Ticket>, String> {
        self.inner.for_feature(feature_id)
    }
    fn record_start(
        &self,
        _: &TicketId,
        _: &FeatureId,
        _: &crate::domain::run_placement::RunPlacement,
        _: i64,
    ) -> Result<(), String> {
        Err("database is locked during record_start".to_string())
    }
    fn list_attempts(
        &self,
        ticket_id: &TicketId,
    ) -> Result<Vec<crate::domain::models::TicketFeatureAttempt>, String> {
        self.inner.list_attempts(ticket_id)
    }
}

/// The note an unrecorded start must carry: the run that exists, and why the
/// board will not show it.
fn assert_unrecorded_note(launched: &LaunchedRun) {
    let note = launched
        .ticket_unrecorded
        .as_deref()
        .expect("the unrecorded start is reported");
    assert!(note.contains(&launched.feature.id.0), "{note}");
    assert!(note.contains("database is locked"), "{note}");
}

/// The SubmitOutcome contract one layer up: once the runner holds a paid run,
/// a failed local write is a note on the launch, never an `Err` that invites
/// the same ticket to be submitted again.
#[tokio::test]
async fn an_unrecordable_detached_start_is_still_one_launched_run() {
    let mut h = harness(RunnerAt::accepting());
    h.ctx.tickets = StartUnrecordable::wrap(h.ctx.tickets.clone());
    let t = stored_ticket(&h, Some("runner-1"));

    let launched = start(&h.ctx, &t.id, None)
        .await
        .unwrap_or_else(|error| panic!("an accepted run is launched, got {error}"));

    assert_unrecorded_note(&launched);
    let submits = h
        .exec
        .calls()
        .into_iter()
        .filter(|call| call == "rpc submit_run")
        .count();
    assert_eq!(submits, 1);
    assert!(h.spy.launched().is_none(), "the executor is not called");
    assert_eq!(launched.credentials_parked, None);
    assert_eq!(launched.mirror_unrecorded, None);
}

#[tokio::test]
async fn an_unrecordable_local_start_is_still_one_launched_run() {
    let mut h = harness(RunnerAt::accepting());
    h.ctx.tickets = StartUnrecordable::wrap(h.ctx.tickets.clone());
    let t = stored_ticket(&h, Some("local"));

    let launched = start(&h.ctx, &t.id, None)
        .await
        .unwrap_or_else(|error| panic!("a started run is launched, got {error}"));

    assert_unrecorded_note(&launched);
    assert_eq!(h.spy.launch_count(), 1);
    assert_eq!(h.exec.calls(), Vec::<String>::new(), "no RPC");
}

fn board_view(h: &Harness, t: &Ticket) -> TicketView {
    board(&h.ctx, &t.discovery_id)
        .expect("the board reads")
        .tickets
        .into_iter()
        .find(|v| v.ticket.id == t.id)
        .expect("the ticket is on its board")
}

/// D9: the lane is the shadow Feature's, which reconcile hydrates — the board
/// never consults the mirror to place a detached ticket.
#[tokio::test]
async fn a_detached_ticket_lands_and_releases_its_dependent_through_the_shadow() {
    let h = harness(RunnerAt::accepting());
    let detached = stored_ticket(&h, Some("runner-1"));
    let mut dependent = detached.clone();
    dependent.id = TicketId::from("t-2".to_string());
    dependent.seq = 2;
    dependent.blocked_by = vec![detached.id.clone()];
    dependent.machine_id = None;
    h.ctx
        .tickets
        .upsert_batch(std::slice::from_ref(&dependent))
        .expect("the dependent is stored");

    let feature = start(&h.ctx, &detached.id, None)
        .await
        .expect("the ticket starts")
        .feature;

    assert_eq!(
        board_view(&h, &detached).standing.lane,
        TicketLane::InFlight
    );
    assert!(!board_view(&h, &dependent).standing.startable);

    h.ctx
        .features
        .update(
            &feature.id,
            &FeaturePatch {
                mr_state: Some(Some("merged".to_string())),
                ..Default::default()
            },
        )
        .expect("the shadow is hydrated");

    assert_eq!(board_view(&h, &detached).standing.lane, TicketLane::Landed);
    let released = board_view(&h, &dependent).standing;
    assert!(released.startable, "{released:?}");
    assert_eq!(released.lane, TicketLane::Ready);
}

#[tokio::test]
async fn a_detached_tickets_card_carries_its_mirror_row() {
    let h = harness(RunnerAt::accepting());
    let t = stored_ticket(&h, Some("runner-1"));
    start(&h.ctx, &t.id, None).await.expect("the ticket starts");
    let mirror = h
        .ctx
        .remote_run_mirror
        .list()
        .expect("mirror reads")
        .remove(0);
    h.ctx
        .remote_run_mirror
        .update_status(
            &mirror.machine_id,
            &mirror.run_id,
            "running",
            None,
            None,
            None,
            None,
            0,
            now_ms(),
        )
        .expect("the mirror advances");

    let remote = board_view(&h, &t)
        .feature
        .expect("a started ticket has a feature")
        .remote
        .expect("a detached attempt exposes its mirror row");

    assert_eq!(remote.machine_id, "runner-1");
    assert_eq!(remote.run_id, mirror.run_id);
    assert_eq!(remote.status, "running");
}

/// A Feature is retried in place, and a ticket's shadow Feature reaches its
/// run the way a UI-launched detached run does: through the mirror row keyed
/// by the Feature id. The retry records no new attempt on the ticket.
#[tokio::test]
async fn a_detached_tickets_shadow_is_retried_through_its_mirror_row() {
    let h = harness(RunnerAt {
        accepts_retry: true,
        ..RunnerAt::accepting()
    });
    let t = stored_ticket(&h, Some("runner-1"));
    let feature = start(&h.ctx, &t.id, None)
        .await
        .expect("the ticket starts")
        .feature;
    let accepted = h
        .ctx
        .remote_run_mirror
        .list()
        .expect("mirror reads")
        .remove(0);
    let before = h.exec.calls().len();

    let mirror = find_mirror_for_feature(&h.ctx, feature.id.0.clone())
        .expect("the mirror reads")
        .expect("the shadow Feature names its run");
    assert_eq!(mirror.machine_id, "runner-1");
    assert_eq!(mirror.run_id, accepted.run_id);

    retry_remote_step(
        &h.ctx,
        mirror.machine_id,
        mirror.run_id.clone(),
        "se-1".to_string(),
        RewindOverrides::default(),
        RemoteRewind::Retry,
    )
    .await
    .expect("the retry reaches the runner");

    let retried = h.exec.calls()[before..].to_vec();
    assert!(
        retried.contains(&format!("rpc retry_step {} se-1 retry", mirror.run_id)),
        "{retried:?}"
    );
    assert_started_detached(&h, &t, &feature);
}

/// Two surfaces starting one ticket at once — the board and an MCP
/// `start_ticket` — would both read it `Unstarted`, and the second would
/// orphan the first's run. The loser is refused before anything is asked of
/// the executor or the runner.
#[tokio::test]
async fn a_start_already_under_way_refuses_a_second_one_before_any_io() {
    let h = harness(RunnerAt::accepting());
    let t = stored_ticket(&h, Some("runner-1"));
    let in_flight = h
        .ctx
        .ticket_starts
        .try_claim(&t.id)
        .expect("nothing holds the ticket yet");

    for second in [
        start(&h.ctx, &t.id, None).await,
        start(&h.ctx, &t.id, Some(MachineId::from("local"))).await,
    ] {
        let error = second.expect_err("a second start is refused");
        assert_eq!(error.code(), "validation", "{error}");
        assert!(error.to_string().contains("#1"), "{error}");
        assert!(error.to_string().contains("under way"), "{error}");
    }

    assert_eq!(h.exec.calls(), Vec::<String>::new(), "no RPC");
    assert_untouched(&h, &t);
    drop(in_flight);
}

/// The claim lives exactly as long as one start: a refused start leaves the
/// ticket retryable, and a finished one leaves it claimable again.
#[tokio::test]
async fn a_start_releases_its_claim_whether_it_launches_or_is_refused() {
    let mut h = harness(RunnerAt::reporting("1.2.0-30"));
    let t = stored_ticket(&h, Some("runner-1"));

    start(&h.ctx, &t.id, None)
        .await
        .expect_err("an incompatible runner is refused");
    assert!(
        h.ctx.ticket_starts.try_claim(&t.id).is_some(),
        "a refused start released its claim"
    );

    h.ctx.exec = Arc::new(RunnerAt::accepting());
    let feature = start(&h.ctx, &t.id, None)
        .await
        .expect("the retry starts")
        .feature;
    assert_started_detached(&h, &t, &feature);
    assert!(
        h.ctx.ticket_starts.try_claim(&t.id).is_some(),
        "a launched start released its claim"
    );
}
