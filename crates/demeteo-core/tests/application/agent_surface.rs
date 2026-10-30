// Tests extracted from `crates/demeteo-core/src/application/agent_surface/reads.rs`
// (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::discovery::{create as open_discovery, NewDiscovery};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{GateDecisionId, StepId, TicketId};
use crate::domain::models::{Project, Ticket, TicketState};
use crate::ports::discovery::DiscoveryPatch;

/// A fully wired `AppContext` over a fresh temp-dir SQLite database — every
/// read in this module only ever touches repository ports and `run_view`,
/// which this gives real (rather than faked) implementations of, so nothing
/// here needs a hand-rolled execution-port double.
fn fixture(tag: &str) -> AppContext {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-agent-surface-{tag}"));
    build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    )
}

fn project(id: &str) -> Project {
    Project {
        id: ProjectId::from(id.to_string()),
        name: format!("project {id}"),
        compute_type: "local".to_string(),
        remote_host: None,
        status: "idle".to_string(),
        nodes: 0,
        spend: 0.0,
        tokens: 0,
        created_at: 0,
    }
}

fn feature(id: &str, project_id: &ProjectId) -> Feature {
    Feature {
        id: FeatureId::from(id.to_string()),
        project_id: project_id.clone(),
        workflow_id: None,
        workflow_version_id: None,
        title: format!("feature {id}"),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        duration: String::new(),
        tokens: 0,
        created_at: 0,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: Some("none".to_string()),
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
        launched_by: Default::default(),
    }
}

fn step(id: &str, feature_id: &FeatureId, index: u32) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from(id.to_string()),
        feature_id: feature_id.clone(),
        step_id: StepId::from(format!("s-{id}")),
        step_index: index,
        step_kind: "agent".to_string(),
        status: "completed".to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn ticket(id: &str, discovery_id: &DiscoveryId, seq: i64, state: TicketState) -> Ticket {
    Ticket {
        id: TicketId::from(id.to_string()),
        discovery_id: discovery_id.clone(),
        seq,
        title: format!("ticket {seq}"),
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
        state,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// Same shape `tests/application/tickets/launch.rs`'s `ctx_with_project`
/// uses: a project with nothing else in it, as much as opening a Discovery
/// reads off one.
fn ctx_with_project(tag: &str) -> (AppContext, ProjectId) {
    let ctx = fixture(tag);
    let project_id = ProjectId::from(format!("p-{tag}"));
    ctx.projects
        .add(project(project_id.as_str()))
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

#[tokio::test]
async fn listing_projects_on_an_empty_store_is_empty() {
    let ctx = fixture("projects-empty");
    assert!(list_projects(&ctx).unwrap().is_empty());
}

#[tokio::test]
async fn listing_projects_returns_every_inserted_project() {
    let ctx = fixture("projects-two");
    ctx.projects.add(project("p-1")).unwrap();
    ctx.projects.add(project("p-2")).unwrap();

    let mut ids: Vec<String> = list_projects(&ctx)
        .unwrap()
        .into_iter()
        .map(|p| p.id.as_str().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["p-1".to_string(), "p-2".to_string()]);
}

#[tokio::test]
async fn listing_features_scoped_to_a_project_excludes_the_others() {
    let ctx = fixture("features-scoped");
    let p1 = ProjectId::from("p-1".to_string());
    let p2 = ProjectId::from("p-2".to_string());
    ctx.projects.add(project(p1.as_str())).unwrap();
    ctx.projects.add(project(p2.as_str())).unwrap();
    ctx.features.add(feature("f-1", &p1)).unwrap();
    ctx.features.add(feature("f-2", &p2)).unwrap();

    let scoped = list_features(&ctx, Some(&p1)).unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].id.as_str(), "f-1");
}

#[tokio::test]
async fn listing_features_with_no_project_unions_every_project_and_skips_empty_ones() {
    let ctx = fixture("features-union");
    let p1 = ProjectId::from("p-1".to_string());
    let p2 = ProjectId::from("p-2".to_string());
    let p3 = ProjectId::from("p-3".to_string());
    ctx.projects.add(project(p1.as_str())).unwrap();
    ctx.projects.add(project(p2.as_str())).unwrap();
    ctx.projects.add(project(p3.as_str())).unwrap();
    ctx.features.add(feature("f-1", &p1)).unwrap();
    ctx.features.add(feature("f-2", &p2)).unwrap();
    // p3 has no active features and must contribute nothing.

    let mut ids: Vec<String> = list_features(&ctx, None)
        .unwrap()
        .into_iter()
        .map(|f| f.id.as_str().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["f-1".to_string(), "f-2".to_string()]);
}

#[tokio::test]
async fn getting_a_missing_feature_is_none_not_an_error() {
    let ctx = fixture("feature-missing");
    let missing = FeatureId::from("f-none".to_string());
    assert!(get_feature(&ctx, &missing).unwrap().is_none());
}

#[tokio::test]
async fn getting_a_feature_carries_every_one_of_its_steps() {
    let ctx = fixture("feature-with-steps");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-1".to_string());
    ctx.features.add(feature("f-1", &project_id)).unwrap();
    ctx.features
        .step_create(step("se-1", &feature_id, 0))
        .unwrap();
    ctx.features
        .step_create(step("se-2", &feature_id, 1))
        .unwrap();

    let found = get_feature(&ctx, &feature_id)
        .unwrap()
        .expect("the feature exists");
    assert_eq!(found.feature.id, feature_id);
    let mut step_ids: Vec<String> = found
        .steps
        .iter()
        .map(|s| s.id.as_str().to_string())
        .collect();
    step_ids.sort();
    assert_eq!(step_ids, vec!["se-1".to_string(), "se-2".to_string()]);
}

#[tokio::test]
async fn a_step_with_no_attempts_lists_none() {
    let ctx = fixture("attempts-empty");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-1".to_string());
    ctx.features.add(feature("f-1", &project_id)).unwrap();
    let step_execution_id = StepExecutionId::from("se-1".to_string());
    ctx.features
        .step_create(step("se-1", &feature_id, 0))
        .unwrap();

    assert!(list_step_attempts(&ctx, &step_execution_id)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_step_with_two_attempts_lists_both() {
    let ctx = fixture("attempts-two");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-1".to_string());
    ctx.features.add(feature("f-1", &project_id)).unwrap();
    let step_execution_id = StepExecutionId::from("se-1".to_string());
    ctx.features
        .step_create(step("se-1", &feature_id, 0))
        .unwrap();

    let attempt_no_1 = ctx
        .features
        .attempt_open(&step_execution_id, 0, None)
        .unwrap();
    ctx.features
        .attempt_close(
            &step_execution_id,
            attempt_no_1,
            "completed",
            0.0,
            0,
            0,
            None,
            None,
            None,
            1,
        )
        .unwrap();
    let attempt_no_2 = ctx
        .features
        .attempt_open(&step_execution_id, 2, None)
        .unwrap();
    ctx.features
        .attempt_close(
            &step_execution_id,
            attempt_no_2,
            "completed",
            0.0,
            0,
            0,
            None,
            None,
            None,
            3,
        )
        .unwrap();

    let attempts = list_step_attempts(&ctx, &step_execution_id).unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].attempt_no, 1);
    assert_eq!(attempts[1].attempt_no, 2);
}

#[tokio::test]
async fn no_active_feature_awaiting_a_gate_lists_no_pending_gates() {
    let ctx = fixture("gates-none");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    ctx.features.add(feature("f-1", &project_id)).unwrap();

    assert!(list_pending_gates(&ctx, None).unwrap().is_empty());
}

#[tokio::test]
async fn one_feature_with_a_pending_gate_is_named_and_the_other_active_feature_is_not() {
    let ctx = fixture("gates-one");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let gated_feature_id = FeatureId::from("f-gated".to_string());
    ctx.features.add(feature("f-gated", &project_id)).unwrap();
    ctx.features.add(feature("f-clean", &project_id)).unwrap();
    let step_execution_id = StepExecutionId::from("se-gate".to_string());
    ctx.features
        .step_create(step("se-gate", &gated_feature_id, 0))
        .unwrap();
    ctx.gates
        .create(GateDecision {
            id: GateDecisionId::from("gd-1".to_string()),
            step_execution_id: step_execution_id.clone(),
            decision: None,
            feedback: None,
            created_at: 0,
            auto_approved: false,
        })
        .unwrap();

    let pending = list_pending_gates(&ctx, Some(&project_id)).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].project_id, project_id);
    assert_eq!(pending[0].feature_id, gated_feature_id);
    assert_eq!(pending[0].feature_title, "feature f-gated");
    assert_eq!(pending[0].decision.step_execution_id, step_execution_id);
}

/// A run on a remote machine writes its gate decisions on the runner, so the
/// desktop holds only the mirrored step row: no `gate_decisions` row exists.
async fn feature_with_mirrored_gate(tag: &str, status: &str) -> (AppContext, ProjectId) {
    let ctx = fixture(tag);
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-detached".to_string());
    ctx.features
        .add(feature("f-detached", &project_id))
        .unwrap();
    let mut gate = step("se-gate", &feature_id, 1);
    gate.step_kind = "gate".to_string();
    gate.status = status.to_string();
    ctx.features.step_create(gate).unwrap();
    (ctx, project_id)
}

#[tokio::test]
async fn a_gate_mirrored_from_a_runner_is_listed_without_a_local_decision_row() {
    let (ctx, project_id) = feature_with_mirrored_gate("gates-mirrored", "awaiting_gate").await;

    let pending = list_pending_gates(&ctx, Some(&project_id)).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].feature_id.as_str(), "f-detached");
    assert_eq!(pending[0].decision.step_execution_id.as_str(), "se-gate");
    assert_eq!(pending[0].decision.decision, None);
}

#[tokio::test]
async fn a_mirrored_gate_that_has_been_decided_is_not_listed() {
    let (ctx, _) = feature_with_mirrored_gate("gates-mirrored-done", "completed").await;

    assert!(list_pending_gates(&ctx, None).unwrap().is_empty());
}

/// A merge or a removal settles a feature without deciding its gate, so the
/// row outlives the run; no decision on it could reach anything.
#[tokio::test]
async fn a_pending_gate_row_of_a_run_that_has_ended_is_not_listed() {
    for ended in ["completed", "awaiting_mr", "failed", "cancelled"] {
        let (ctx, _) =
            feature_with_mirrored_gate(&format!("gates-ended-{ended}"), "awaiting_gate").await;
        ctx.features
            .update(
                &FeatureId::from("f-detached".to_string()),
                &crate::ports::db::FeaturePatch {
                    status: Some(ended.to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        ctx.gates
            .create(GateDecision {
                id: GateDecisionId::from("gd-1".to_string()),
                step_execution_id: StepExecutionId::from("se-gate".to_string()),
                decision: None,
                feedback: None,
                created_at: 0,
                auto_approved: false,
            })
            .unwrap();

        assert!(
            list_pending_gates(&ctx, None).unwrap().is_empty(),
            "{ended}"
        );
    }
}

#[tokio::test]
async fn a_local_gate_with_a_pending_row_is_listed_once_under_its_own_decision_id() {
    let (ctx, _) = feature_with_mirrored_gate("gates-local-once", "awaiting_gate").await;
    ctx.gates
        .create(GateDecision {
            id: GateDecisionId::from("gd-1".to_string()),
            step_execution_id: StepExecutionId::from("se-gate".to_string()),
            decision: None,
            feedback: None,
            created_at: 0,
            auto_approved: false,
        })
        .unwrap();

    let pending = list_pending_gates(&ctx, None).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].decision.id.as_str(), "gd-1");
}

#[tokio::test]
async fn a_gate_decided_locally_before_its_step_row_settles_is_not_listed() {
    let (ctx, _) = feature_with_mirrored_gate("gates-decided-race", "awaiting_gate").await;
    ctx.gates
        .create(GateDecision {
            id: GateDecisionId::from("gd-1".to_string()),
            step_execution_id: StepExecutionId::from("se-gate".to_string()),
            decision: Some("approve".to_string()),
            feedback: None,
            created_at: 0,
            auto_approved: false,
        })
        .unwrap();

    assert!(list_pending_gates(&ctx, None).unwrap().is_empty());
}

#[tokio::test]
async fn a_discovery_boards_ticket_count_matches_its_mixed_state_fixture() {
    let (ctx, project_id) = ctx_with_project("board");
    let discovery =
        open_discovery(&ctx, opening(&project_id, "board fixture")).expect("the discovery opens");
    ctx.tickets
        .upsert_batch(&[
            ticket("t-1", &discovery.id, 1, TicketState::Unstarted),
            ticket("t-2", &discovery.id, 2, TicketState::Started),
            ticket("t-3", &discovery.id, 3, TicketState::Dropped),
        ])
        .unwrap();

    let board = get_discovery_board(&ctx, &discovery.id).unwrap();
    assert_eq!(board.tickets.len(), 3);
}

#[tokio::test]
async fn run_events_since_zero_returns_everything_ascending() {
    let ctx = fixture("events-all");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-1".to_string());
    ctx.features.add(feature("f-1", &project_id)).unwrap();

    ctx.run_events
        .append(feature_id.as_str(), "kind-a", None, 0)
        .unwrap();
    ctx.run_events
        .append(feature_id.as_str(), "kind-b", None, 1)
        .unwrap();

    let events = run_events_since(&ctx, &feature_id, 0).unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0].offset < events[1].offset);
    assert_eq!(events[0].kind, "kind-a");
    assert_eq!(events[1].kind, "kind-b");
}

#[tokio::test]
async fn get_failure_verdict_matches_run_view_explain_step_failure_directly() {
    let ctx = fixture("failure-verdict");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-1".to_string());
    ctx.features.add(feature("f-1", &project_id)).unwrap();
    let step_execution_id = StepExecutionId::from("se-1".to_string());
    ctx.features
        .step_create(step("se-1", &feature_id, 0))
        .unwrap();

    let attempt_no = ctx
        .features
        .attempt_open(&step_execution_id, 0, None)
        .unwrap();
    ctx.features
        .attempt_close(
            &step_execution_id,
            attempt_no,
            "failed",
            0.0,
            0,
            0,
            Some("verdict"),
            Some("fp-a"),
            Some("verdict.redirect"),
            1,
        )
        .unwrap();

    let verdict = get_failure_verdict(&ctx, &step_execution_id).unwrap();
    let expected = ctx
        .run_view
        .explain_step_failure(&step_execution_id)
        .unwrap();
    assert_eq!(verdict.verdict, expected.verdict);
    assert_eq!(verdict.log_tail, expected.log_tail);
}

#[tokio::test]
async fn run_events_since_a_later_offset_excludes_events_at_or_below_it() {
    let ctx = fixture("events-since");
    let project_id = ProjectId::from("p-1".to_string());
    ctx.projects.add(project(project_id.as_str())).unwrap();
    let feature_id = FeatureId::from("f-1".to_string());
    ctx.features.add(feature("f-1", &project_id)).unwrap();

    let first_offset = ctx
        .run_events
        .append(feature_id.as_str(), "kind-a", None, 0)
        .unwrap();
    ctx.run_events
        .append(feature_id.as_str(), "kind-b", None, 1)
        .unwrap();

    let events = run_events_since(&ctx, &feature_id, first_offset).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "kind-b");
}

#[tokio::test]
async fn listing_discoveries_scopes_to_a_project_or_unions_them_all() {
    let (ctx, first) = ctx_with_project("discoveries");
    let second = ProjectId::from("p-other".to_string());
    ctx.projects.add(project(second.as_str())).unwrap();
    let a = open_discovery(&ctx, opening(&first, "first plan")).expect("a opens");
    let b = open_discovery(&ctx, opening(&second, "second plan")).expect("b opens");

    let scoped = list_discoveries(&ctx, Some(&second)).unwrap();
    assert_eq!(
        scoped.iter().map(|d| &d.id).collect::<Vec<_>>(),
        [&b.id],
        "only the named project's"
    );

    let mut all: Vec<_> = list_discoveries(&ctx, None)
        .unwrap()
        .into_iter()
        .map(|d| (d.id.0, d.project_id.0, d.title))
        .collect();
    all.sort();
    let mut expected = vec![
        (a.id.0, first.0, "first plan".to_string()),
        (b.id.0, second.0, "second plan".to_string()),
    ];
    expected.sort();
    assert_eq!(all, expected);
}

#[tokio::test]
async fn a_listed_discovery_and_its_board_name_the_base_branch() {
    let (ctx, project_id) = ctx_with_project("base-branch");
    let discovery =
        open_discovery(&ctx, opening(&project_id, "integration")).expect("the discovery opens");

    assert_eq!(list_discoveries(&ctx, None).unwrap()[0].base_branch, None);
    assert_eq!(
        get_discovery_board(&ctx, &discovery.id)
            .unwrap()
            .base_branch,
        None,
        "no base is the project's default branch"
    );

    ctx.discoveries
        .update(
            &discovery.id,
            &DiscoveryPatch {
                base_branch: Some(Some("feat/integration".to_string())),
                ..Default::default()
            },
            1,
        )
        .expect("the base branch is stored");

    let named = Some("feat/integration".to_string());
    assert_eq!(list_discoveries(&ctx, None).unwrap()[0].base_branch, named);
    assert_eq!(
        get_discovery_board(&ctx, &discovery.id)
            .unwrap()
            .base_branch,
        named
    );
}

#[tokio::test]
async fn a_started_tickets_card_names_the_branch_its_run_cut() {
    let (ctx, project_id) = ctx_with_project("head-branch");
    let discovery =
        open_discovery(&ctx, opening(&project_id, "branches")).expect("the discovery opens");
    let mut cut = feature("f-cut", &project_id);
    cut.resolved_branch = Some("demeteo/f-cut".to_string());
    ctx.features.add(cut).unwrap();
    ctx.features.add(feature("f-uncut", &project_id)).unwrap();
    let mut with_branch = ticket("t-1", &discovery.id, 1, TicketState::Started);
    with_branch.feature_id = Some(FeatureId::from("f-cut".to_string()));
    let mut before_cut = ticket("t-2", &discovery.id, 2, TicketState::Started);
    before_cut.feature_id = Some(FeatureId::from("f-uncut".to_string()));
    ctx.tickets
        .upsert_batch(&[with_branch, before_cut])
        .unwrap();

    let board = get_discovery_board(&ctx, &discovery.id).unwrap();
    let branches: Vec<Option<String>> = board
        .tickets
        .iter()
        .map(|t| t.feature.as_ref().expect("a started ticket").branch.clone())
        .collect();

    assert_eq!(branches, [Some("demeteo/f-cut".to_string()), None]);
}

/// A `read` grant spans every project, so what a machine row says beyond its
/// id and name must not ride out on this list.
#[tokio::test]
async fn listing_machines_names_placements_and_none_of_the_connection_details() {
    let ctx = fixture("machines");
    for existing in ctx.machines.get_machines().unwrap() {
        ctx.machines.delete(&existing.id).unwrap();
    }
    let mut remote = crate::application::launch::tests::machine("runner-1", "key");
    remote.host = "runner.internal.example".to_string();
    remote.username = "deploy".to_string();
    remote.key_path = Some("/home/me/.ssh/id_runner".to_string());
    remote.notify_webhook_url = Some("https://hooks.example/T0/secret".to_string());
    ctx.machines.add(remote).unwrap();
    ctx.machines
        .add(crate::application::launch::tests::machine("desk", "local"))
        .unwrap();

    let mut machines = list_machines(&ctx).unwrap();
    machines.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));

    assert_eq!(
        machines.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["desk", "runner-1"]
    );
    assert_eq!(machines[0].placement, RunPlacement::Local);
    assert_eq!(
        machines[1].placement,
        RunPlacement::Detached {
            machine_id: MachineId::from("runner-1")
        }
    );
    let wire = serde_json::to_string(&machines).unwrap();
    for hidden in [
        "runner.internal.example",
        "deploy",
        "id_runner",
        "hooks.example",
    ] {
        assert!(!wire.contains(hidden), "{hidden} left on the wire: {wire}");
    }
}

// ── refresh_run ─────────────────────────────────────────────────────────────

/// A runner that answers only the RPCs it was scripted with and fails every
/// other call, recording each one.
struct ScriptedRunner {
    rpc: std::collections::HashMap<&'static str, Result<serde_json::Value, String>>,
    calls: std::sync::Mutex<Vec<String>>,
}

impl ScriptedRunner {
    fn new(rpc: Vec<(&'static str, Result<serde_json::Value, String>)>) -> Arc<Self> {
        Arc::new(Self {
            rpc: rpc.into_iter().collect(),
            calls: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::ports::execution::ExecutionPort for ScriptedRunner {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        Err("unscripted test_connection".to_string())
    }
    async fn read_file(&self, _machine_id: &str, _path: &str) -> Result<String, String> {
        Err("unscripted read_file".to_string())
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("unscripted write_file".to_string())
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        Err("unscripted write_file_bytes".to_string())
    }
    async fn get_metadata(
        &self,
        _: &str,
        _: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        Err("unscripted get_metadata".to_string())
    }
    async fn list_dir(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
        Err("unscripted list_dir".to_string())
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("unscripted setup_worktree".to_string())
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        Err("unscripted resolve_home".to_string())
    }
    async fn resolve_platform(&self, _: &str) -> Result<crate::domain::models::Platform, String> {
        Err("unscripted resolve_platform".to_string())
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        Err("unscripted resolve_user".to_string())
    }
    async fn control_rpc(
        &self,
        _machine_id: &str,
        method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.calls.lock().unwrap().push(method.to_string());
        self.rpc
            .get(method)
            .cloned()
            .unwrap_or_else(|| Err(format!("unscripted rpc {method}")))
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("unscripted spawn_interactive".to_string())
    }
}

/// A shadow feature `f-1` (running, no steps) with a mirror row on `m-1`.
fn detached_fixture(tag: &str, runner: Arc<ScriptedRunner>) -> AppContext {
    let (mut ctx, project_id) = ctx_with_project(tag);
    ctx.features.add(feature("f-1", &project_id)).unwrap();
    ctx.remote_run_mirror
        .upsert_submitted(
            "m-1",
            "r-1",
            Some(project_id.as_str()),
            Some("f-1"),
            "detached",
            0,
        )
        .unwrap();
    ctx.exec = runner;
    ctx
}

fn runner_answering_completed(tag: &str) -> Arc<ScriptedRunner> {
    let mut runner_feature = feature("f-1", &ProjectId::from(format!("p-{tag}")));
    runner_feature.status = "completed".to_string();
    let runner_step = step("se-1", &runner_feature.id, 0);
    ScriptedRunner::new(vec![
        (
            "get_status",
            Ok(serde_json::json!({ "status": "completed", "feature_id": "f-1" })),
        ),
        (
            "get_feature",
            Ok(serde_json::to_value(runner_feature).unwrap()),
        ),
        (
            "list_steps",
            Ok(serde_json::to_value(vec![runner_step]).unwrap()),
        ),
    ])
}

fn f1() -> FeatureId {
    FeatureId::from("f-1".to_string())
}

#[tokio::test]
async fn refresh_run_of_a_missing_feature_is_an_error() {
    let ctx = fixture("refresh-missing");
    let error = refresh_run(&ctx, &FeatureId::from("f-none".to_string()))
        .await
        .unwrap_err();
    assert_eq!(error, "feature not found");
}

#[tokio::test]
async fn refresh_run_of_a_local_feature_returns_it_unchanged_without_asking_a_runner() {
    let (mut ctx, project_id) = ctx_with_project("refresh-local");
    let runner = ScriptedRunner::new(Vec::new());
    ctx.exec = runner.clone();
    ctx.features.add(feature("f-1", &project_id)).unwrap();

    let refreshed = refresh_run(&ctx, &f1()).await.unwrap();

    assert!(!refreshed.refreshed);
    assert!(refreshed
        .note
        .as_deref()
        .unwrap()
        .contains("not a detached run"));
    assert_eq!(refreshed.run.feature.status, "running");
    assert!(runner.calls().is_empty(), "{:?}", runner.calls());
}

#[tokio::test]
async fn refresh_run_pulls_the_runners_state_into_the_shadow_and_returns_it() {
    let runner = runner_answering_completed("refresh-detached");
    let ctx = detached_fixture("refresh-detached", runner.clone());
    assert!(get_feature(&ctx, &f1()).unwrap().unwrap().steps.is_empty());

    let refreshed = refresh_run(&ctx, &f1()).await.unwrap();

    assert!(refreshed.refreshed);
    assert!(refreshed.note.is_none());
    assert_eq!(refreshed.run.feature.status, "completed");
    assert_eq!(refreshed.run.steps.len(), 1);
    let stored = get_feature(&ctx, &f1()).unwrap().unwrap();
    assert_eq!(stored.feature.status, "completed");
    assert_eq!(runner.calls(), ["get_status", "get_feature", "list_steps"]);
}

#[tokio::test]
async fn refresh_run_reports_an_unreachable_runner_and_leaves_the_shadow_alone() {
    let runner = ScriptedRunner::new(vec![(
        "get_status",
        Err("ssh: connection refused".to_string()),
    )]);
    let ctx = detached_fixture("refresh-unreachable", runner);

    let error = refresh_run(&ctx, &f1()).await.unwrap_err();

    assert!(
        error.contains("m-1") && error.contains("could not be reached"),
        "{error}"
    );
    let stored = get_feature(&ctx, &f1()).unwrap().unwrap();
    assert_eq!(stored.feature.status, "running");
}
