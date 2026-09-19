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

/// A fully wired `AppContext` over a fresh temp-dir SQLite database — every
/// read in this module only ever touches repository ports and `run_view`,
/// which this gives real (rather than faked) implementations of, so nothing
/// here needs a hand-rolled execution-port double.
fn fixture(tag: &str) -> AppContext {
    let dir = std::env::temp_dir().join(format!(
        "demeteo-agent-surface-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos()
    ));
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
        })
        .unwrap();

    let pending = list_pending_gates(&ctx, Some(&project_id)).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].project_id, project_id);
    assert_eq!(pending[0].feature_id, gated_feature_id);
    assert_eq!(pending[0].feature_title, "feature f-gated");
    assert_eq!(pending[0].decision.step_execution_id, step_execution_id);
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
