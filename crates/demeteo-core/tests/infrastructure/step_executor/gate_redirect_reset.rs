// Regression tests for gate redirect handling.
// Extracted from `steps/gate.rs` (kept out of the source file per the
// crate's mirrored-tests convention); `super` resolves to that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, WorkflowId};
use crate::domain::models::Feature;
use crate::ports::db::{FeatureRepository, GateRepository, ProjectRepository};
use rusqlite::Connection;

/// Construct an in-memory `SqliteAdapter` that implements every
/// port the helper touches (`FeatureRepository`,
/// `GateRepository`, and the rest of the trait surface that
/// `SqliteAdapter::new` requires). We pass it three times as
/// `&dyn` of the three relevant ports — the helper is generic
/// over `&dyn FeatureRepository` / `&dyn GateRepository`.
#[allow(clippy::type_complexity)]
fn make_adapter() -> (
    std::sync::Arc<SqliteAdapter>,
    std::sync::Arc<dyn ProjectRepository>,
    std::sync::Arc<dyn FeatureRepository>,
    std::sync::Arc<dyn GateRepository>,
) {
    let conn = Connection::open_in_memory().unwrap();
    let adapter = std::sync::Arc::new(SqliteAdapter::new(conn).unwrap());
    let projects: std::sync::Arc<dyn ProjectRepository> = adapter.clone();
    let features: std::sync::Arc<dyn FeatureRepository> = adapter.clone();
    let gates: std::sync::Arc<dyn GateRepository> = adapter.clone();
    (adapter, projects, features, gates)
}

/// Insert the parent `Project` and `Feature` rows that the
/// `step_executions` foreign key requires. Returns the
/// `FeatureId` used so callers can reuse it in the
/// `StepExecution::feature_id` field.
fn seed_parent_rows(
    projects: &dyn ProjectRepository,
    features: &dyn FeatureRepository,
) -> FeatureId {
    let now = crate::paths::now_ms();
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-1".to_string()),
            name: "test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();
    features
        .add(Feature {
            effort: None,
            id: FeatureId::from("f-1".to_string()),
            project_id: ProjectId::from("p-1".to_string()),
            workflow_id: Some(WorkflowId::from("w-1".to_string())),
            title: "test feature".to_string(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            created_at: now,
            commit_artifacts: None,
            loop_iterations: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
        })
        .unwrap();
    FeatureId::from("f-1".to_string())
}

/// Stand-in `StepExecution` builder. The helper only reads `id`
/// and `step_id`, but the repo refuses garbage values for the
/// other fields, so we fill in plausible ones.
fn make_step_exec(id: &str, step_id: &str, index: u32, status: &str) -> StepExecution {
    let now = crate::paths::now_ms();
    StepExecution {
        last_failure_fingerprint: None,
        id: StepExecutionId::from(id.to_string()),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: crate::domain::ids::StepId::from(step_id.to_string()),
        step_index: index,
        step_kind: "agent".to_string(),
        status: status.to_string(),
        cost_usd: Some(0.42),
        tokens: Some(1234),
        wall_clock_secs: Some(7),
        artifact_path: Some("artifacts/spec.md".to_string()),
        artifact_paths: vec!["artifacts/spec.md".to_string()],
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        created_at: now,
        updated_at: now,
    }
}

fn make_gate_exec(id: &str, index: u32) -> StepExecution {
    let now = crate::paths::now_ms();
    StepExecution {
        last_failure_fingerprint: None,
        id: StepExecutionId::from(id.to_string()),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: crate::domain::ids::StepId::from("s-gate".to_string()),
        step_index: index,
        step_kind: "gate".to_string(),
        status: "awaiting_gate".to_string(),
        cost_usd: Some(0.0),
        tokens: Some(0),
        wall_clock_secs: Some(0),
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        created_at: now,
        updated_at: now,
    }
}

/// Captures the events the helper emits so the assertions can
/// check the UI-facing contract: each DB mutation must be paired
/// with a `StepProgress` event whose status matches the new DB
/// value. `FakeNotif` in the e2e suite silently drops events;
/// this variant records them for inspection.
struct CapturingNotif {
    events: std::sync::Mutex<Vec<DomainEvent>>,
}
impl CapturingNotif {
    fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn snapshot(&self) -> Vec<DomainEvent> {
        self.events.lock().unwrap().clone()
    }
}
impl NotificationPort for CapturingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

#[test]
fn reset_marks_target_step_pending_and_clears_artifacts() {
    // Mirror the real bug: spec is `completed` with artifacts
    // attached, the gate is mid-decision. The helper must
    // rewind spec to `pending` and drop the artifacts so the
    // re-run starts from a clean slate.
    let (_adapter, projects, features, gates) = make_adapter();
    let f_id = seed_parent_rows(&*projects, &*features);
    let notif = CapturingNotif::new();

    let spec = make_step_exec("se-spec", "s-spec", 1, "completed");
    let gate = make_gate_exec("se-gate", 2);
    let step_execs = vec![
        make_step_exec("se-research", "s-research", 0, "completed"),
        spec.clone(),
        gate.clone(),
    ];

    // Persist the spec + gate so the reset reads / writes hit
    // real rows. The research row is included so the step index
    // math lines up.
    features.step_create(step_execs[0].clone()).unwrap();
    features.step_create(spec.clone()).unwrap();
    features.step_create(gate.clone()).unwrap();

    // Pre-condition: spec carries the artifact from the
    // previous run, gate has an open decision.
    assert_eq!(
        features.step_get(&spec.id).unwrap().unwrap().status,
        "completed"
    );
    assert_eq!(
        features.step_get(&spec.id).unwrap().unwrap().artifact_paths,
        vec!["artifacts/spec.md".to_string()]
    );

    reset_for_redirect(&*features, &*gates, &notif, &f_id, &step_execs, 1, &gate.id);

    // Post-condition: spec is pending with cleared counters
    // and dropped artifacts. The driver will now see the spec
    // as "not yet done" and re-run it instead of skipping past
    // it.
    let spec_after = features.step_get(&spec.id).unwrap().unwrap();
    assert_eq!(spec_after.status, "pending");
    assert_eq!(spec_after.artifact_path, None);
    assert!(spec_after.artifact_paths.is_empty());
    assert_eq!(spec_after.cost_usd, Some(0.0));
    assert_eq!(spec_after.tokens, Some(0));
    assert_eq!(spec_after.wall_clock_secs, Some(0));
}

#[test]
fn reset_clears_gate_decision_row() {
    // The second half of the fix: the gate's own decision row
    // must be deleted, not just updated to `None`. After the
    // reset, the next visit to the gate must find no recorded
    // decision so the reconciliation falls through to the
    // in-process waiter (or the startup watchdog on a fresh
    // launch) and re-prompts the user.
    let (_adapter, projects, features, gates) = make_adapter();
    let f_id = seed_parent_rows(&*projects, &*features);
    let notif = CapturingNotif::new();

    let gate = make_gate_exec("se-gate", 2);
    features.step_create(gate.clone()).unwrap();
    gates
        .create(GateDecision {
            id: GateDecisionId::from("gd-se-gate".to_string()),
            step_execution_id: gate.id.clone(),
            decision: Some("redirect".to_string()),
            feedback: Some("revise the spec to use cargo before mise".to_string()),
            created_at: crate::paths::now_ms(),
        })
        .unwrap();

    // Sanity check: the decision is in place.
    assert_eq!(
        gates
            .latest_for_step(&gate.id)
            .unwrap()
            .unwrap()
            .decision
            .as_deref(),
        Some("redirect")
    );

    let step_execs = vec![
        make_step_exec("se-spec", "s-spec", 1, "completed"),
        gate.clone(),
    ];
    reset_for_redirect(&*features, &*gates, &notif, &f_id, &step_execs, 1, &gate.id);

    // The decision row is gone; `latest_for_step` returns None
    // and the gate's reconciliation will treat this as
    // "no decision yet, await user".
    assert!(gates.latest_for_step(&gate.id).unwrap().is_none());
}

#[test]
fn reset_flips_gate_status_to_pending() {
    // New half of the fix (UI staleness): after a redirect the
    // gate's own status row must move from `awaiting_gate` to
    // `pending` so the timeline stops rendering the "Decide
    // Gate" button. The driver will re-emit `awaiting_gate`
    // when the gate is re-entered after the target completes.
    let (_adapter, projects, features, gates) = make_adapter();
    let f_id = seed_parent_rows(&*projects, &*features);
    let notif = CapturingNotif::new();

    let gate = make_gate_exec("se-gate", 2);
    features.step_create(gate.clone()).unwrap();
    let step_execs = vec![
        make_step_exec("se-research", "s-research", 0, "completed"),
        make_step_exec("se-spec", "s-spec", 1, "completed"),
        gate.clone(),
    ];

    reset_for_redirect(&*features, &*gates, &notif, &f_id, &step_execs, 1, &gate.id);

    // Gate status is no longer `awaiting_gate`.
    let gate_after = features.step_get(&gate.id).unwrap().unwrap();
    assert_eq!(gate_after.status, "pending");
}

#[test]
fn reset_emits_step_progress_for_both_affected_steps() {
    // UI contract: every DB mutation the helper performs must be
    // mirrored by a `StepProgress` event so the frontend's
    // local `steps` array reflects the redirect without waiting
    // for a manual refresh. Two events for two rows: the target
    // spec and the gate itself.
    let (_adapter, projects, features, gates) = make_adapter();
    let f_id = seed_parent_rows(&*projects, &*features);
    let notif = CapturingNotif::new();

    let spec = make_step_exec("se-spec", "s-spec", 1, "completed");
    let gate = make_gate_exec("se-gate", 2);
    let step_execs = vec![
        make_step_exec("se-research", "s-research", 0, "completed"),
        spec.clone(),
        gate.clone(),
    ];
    features.step_create(step_execs[0].clone()).unwrap();
    features.step_create(spec.clone()).unwrap();
    features.step_create(gate.clone()).unwrap();

    reset_for_redirect(&*features, &*gates, &notif, &f_id, &step_execs, 1, &gate.id);

    let events = notif.snapshot();
    let step_progress: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::StepProgress {
                step_id, status, ..
            } => Some((step_id.clone(), status.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        step_progress,
        vec![
            ("s-spec".to_string(), "pending".to_string()),
            ("s-gate".to_string(), "pending".to_string()),
        ],
        "expected one StepProgress per DB mutation, got {:?}",
        step_progress
    );
}

#[test]
fn reset_is_noop_when_target_index_out_of_bounds() {
    // Defensive: a misbehaving `resolve_redirect_target` could
    // return a stale index after the workflow shape changes
    // (e.g. the user re-ran `replay_from_step` and the indices
    // shifted). The helper must not panic; it just has nothing
    // to update. The gate decision is still cleared, since
    // that's an unconditional part of the redirect.
    let (_adapter, projects, features, gates) = make_adapter();
    let f_id = seed_parent_rows(&*projects, &*features);
    let notif = CapturingNotif::new();

    let gate = make_gate_exec("se-gate", 2);
    features.step_create(gate.clone()).unwrap();
    gates
        .create(GateDecision {
            id: GateDecisionId::from("gd-se-gate".to_string()),
            step_execution_id: gate.id.clone(),
            decision: Some("redirect".to_string()),
            feedback: None,
            created_at: crate::paths::now_ms(),
        })
        .unwrap();

    let step_execs = vec![gate.clone()];
    reset_for_redirect(
        &*features,
        &*gates,
        &notif,
        &f_id,
        &step_execs,
        99,
        &gate.id,
    );

    // The decision was still cleared; the out-of-bounds target
    // is silently skipped.
    assert!(gates.latest_for_step(&gate.id).unwrap().is_none());
}
