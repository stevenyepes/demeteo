// Tests extracted from `crates/demeteo-core/src/application/tickets/launch.rs`
// (mirrored-tests convention). `super` = that module.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::discovery::{create as open_discovery, NewDiscovery};
use crate::application::tickets::node_of;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{DiscoveryId, ProjectId, TicketId, WorkflowId};
use crate::domain::models::{EffortLevel, Project};
use crate::domain::ticket_graph::derive_board;
use crate::ports::discovery::DiscoveryPatch;
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
    let dir = std::env::temp_dir().join(format!(
        "demeteo-launch-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
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

    start(&ctx, &t.id).await.expect("the ticket starts");

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

    start(&ctx, &t.id).await.expect("the ticket starts");

    let launch = spy
        .captured
        .lock()
        .expect("lock is not poisoned")
        .clone()
        .expect("feature_start was called");
    assert_eq!(launch.origin, FeatureOrigin::DefaultBranch);
}
