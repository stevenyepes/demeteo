// Tests for `src/adapters/step_executor/driver/run_loop/assignment.rs`
// (mirrored-tests convention). `super` resolves to that module.
//
// The regression these hold is invisible from any end-to-end run that starts
// and finishes in one go: a driver that armed before an assignment was edited
// keeps dispatching from the list it armed with, and every status, event and
// artifact still looks correct — only the agent that actually ran is wrong.
//
// They are cheap to write at all because the refresh reads one port. The
// double below answers `get` and panics on every other method, so the
// "touches only `get`" claim is enforced by the test rather than asserted in
// a comment.

use super::*;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{ProjectId, StepExecutionId, WorkflowId};
use crate::domain::models::{EffortLevel, Feature, StepExecution};
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use std::sync::Mutex;

const F_ID: &str = "f-assign";
const P_ID: &str = "p-assign";

// ── the double ──────────────────────────────────────────────────────────────

/// Answers `get` with whatever it was handed, records that it was asked, and
/// panics on every other method.
struct FeaturesDouble {
    answer: Result<Option<Feature>, String>,
    calls: Mutex<Vec<String>>,
}

impl FeaturesDouble {
    fn answering(answer: Result<Option<Feature>, String>) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("not poisoned").clone()
    }
}

impl FeatureRepository for FeaturesDouble {
    fn get(&self, _id: &FeatureId) -> Result<Option<Feature>, String> {
        self.calls
            .lock()
            .expect("not poisoned")
            .push("get".to_string());
        self.answer.clone()
    }
    fn get_active(&self, _p: &ProjectId) -> Result<Vec<Feature>, String> {
        panic!("unscripted get_active")
    }
    fn add(&self, _f: Feature) -> Result<(), String> {
        panic!("unscripted add")
    }
    fn update(&self, _id: &FeatureId, _patch: &FeaturePatch) -> Result<(), String> {
        panic!("unscripted update")
    }
    fn update_workflow_id(&self, _id: &FeatureId, _w: &WorkflowId) -> Result<(), String> {
        panic!("unscripted update_workflow_id")
    }
    fn merge_harness_baseline(
        &self,
        _id: &FeatureId,
        _b: &crate::domain::harness_baseline::HarnessBaseline,
    ) -> Result<(), String> {
        panic!("unscripted merge_harness_baseline")
    }
    fn pin_workflow_version(
        &self,
        _id: &FeatureId,
        _v: &crate::domain::ids::WorkflowVersionId,
    ) -> Result<(), String> {
        panic!("unscripted pin_workflow_version")
    }
    fn list_with_open_mr(&self) -> Result<Vec<Feature>, String> {
        panic!("unscripted list_with_open_mr")
    }
    fn step_create(&self, _s: StepExecution) -> Result<(), String> {
        panic!("unscripted step_create")
    }
    fn step_get(&self, _id: &StepExecutionId) -> Result<Option<StepExecution>, String> {
        panic!("unscripted step_get")
    }
    fn step_update(&self, _id: &StepExecutionId, _p: &StepExecutionPatch) -> Result<(), String> {
        panic!("unscripted step_update")
    }
    fn steps_for_feature(&self, _id: &FeatureId) -> Result<Vec<StepExecution>, String> {
        panic!("unscripted steps_for_feature")
    }
    fn attempt_open(
        &self,
        _id: &StepExecutionId,
        _now: i64,
        _fp: Option<&str>,
    ) -> Result<u32, String> {
        panic!("unscripted attempt_open")
    }
    #[allow(clippy::too_many_arguments)]
    fn attempt_close(
        &self,
        _id: &StepExecutionId,
        _no: u32,
        _status: &str,
        _cost: f64,
        _tokens: i64,
        _wall_ms: u64,
        _class: Option<&str>,
        _fp: Option<&str>,
        _rule: Option<&str>,
        _now: i64,
    ) -> Result<(), String> {
        panic!("unscripted attempt_close")
    }
    fn attempts_for_step(
        &self,
        _id: &StepExecutionId,
    ) -> Result<Vec<crate::domain::models::StepAttempt>, String> {
        panic!("unscripted attempts_for_step")
    }
    fn subtask_runs_for_step(
        &self,
        _id: &StepExecutionId,
    ) -> Result<Vec<crate::domain::models::SubtaskRunRow>, String> {
        panic!("unscripted subtask_runs_for_step")
    }
    fn subtask_runs_mirror_for_step(
        &self,
        _id: &StepExecutionId,
    ) -> Result<Vec<crate::domain::models::SubtaskRunMirrorRow>, String> {
        panic!("unscripted subtask_runs_mirror_for_step")
    }
    fn subtask_runs_replace_for_step(
        &self,
        _feature_id: &FeatureId,
        _id: &StepExecutionId,
        _rows: &[crate::domain::models::SubtaskRunMirrorRow],
    ) -> Result<(), String> {
        panic!("unscripted subtask_runs_replace_for_step")
    }
}

// ── fixtures ────────────────────────────────────────────────────────────────

fn pin(step_id: &str, model: &str) -> StepOverride {
    StepOverride {
        step_id: step_id.to_string(),
        agent_kind: Some("claude-code".to_string()),
        model: Some(model.to_string()),
        effort: Some(EffortLevel::High),
    }
}

fn feature_with(step_overrides: Vec<StepOverride>) -> Feature {
    Feature {
        id: FeatureId::from(F_ID),
        project_id: ProjectId::from(P_ID),
        workflow_id: Some(WorkflowId::from("w-1")),
        workflow_version_id: None,
        title: "assignable feature".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: 1_700_000_000,
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
        step_overrides,
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    }
}

/// What the driver armed with, before anyone edited anything.
fn armed_snapshot() -> Vec<StepOverride> {
    vec![pin("s-implement", "sonnet")]
}

// ── the regression ──────────────────────────────────────────────────────────

/// The whole point: the row moved on after the driver armed, and the refresh
/// hands back what the row says now, not what the caller is holding.
#[test]
fn a_row_edited_after_arming_yields_the_new_list() {
    let edited = vec![pin("s-implement", "opus"), pin("s-review", "haiku")];
    let features = FeaturesDouble::answering(Ok(Some(feature_with(edited.clone()))));

    let refreshed = refresh_step_overrides(&features, &FeatureId::from(F_ID));

    assert_eq!(refreshed, Some(edited));
    assert_ne!(refreshed, Some(armed_snapshot()));
}

/// An emptied list is a real state — the user reset every step to inherited —
/// so it has to arrive as `Some(vec![])` and overwrite the snapshot, not as
/// the `None` that means "keep what you have".
#[test]
fn a_row_with_every_pin_removed_yields_an_empty_list_not_no_change() {
    let features = FeaturesDouble::answering(Ok(Some(feature_with(Vec::new()))));

    assert_eq!(
        refresh_step_overrides(&features, &FeatureId::from(F_ID)),
        Some(Vec::new())
    );
}

// ── the two ways it declines to answer ──────────────────────────────────────

/// A read that failed knows nothing about the pins. Reporting "no pins" here
/// would un-pin a live run on a transient SQLite error, and the next dispatch
/// would silently pick a different agent.
#[test]
fn a_repository_error_is_no_change() {
    let features = FeaturesDouble::answering(Err("database is locked".to_string()));

    assert_eq!(
        refresh_step_overrides(&features, &FeatureId::from(F_ID)),
        None
    );
}

/// Same reasoning for a row that is no longer there: the run is being torn
/// down and the loop's own guards end it — the refresh must not rewrite the
/// pins on the way out.
#[test]
fn a_missing_feature_row_is_no_change() {
    let features = FeaturesDouble::answering(Ok(None));

    assert_eq!(
        refresh_step_overrides(&features, &FeatureId::from(F_ID)),
        None
    );
}

// ── the port surface ────────────────────────────────────────────────────────

/// One `get` and nothing else — the count matters as much as the method,
/// since this runs on every tick of every live run. Widening the refresh to a
/// second port fails here, which is the moment to ask whether the work
/// belongs in the tick at all.
#[test]
fn the_refresh_reads_exactly_one_method_once() {
    let features = FeaturesDouble::answering(Ok(Some(feature_with(armed_snapshot()))));

    refresh_step_overrides(&features, &FeatureId::from(F_ID));

    assert_eq!(features.calls(), vec!["get".to_string()]);
}
