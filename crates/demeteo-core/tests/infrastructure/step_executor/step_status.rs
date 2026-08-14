// Tests for `src/adapters/step_executor/step_status.rs` (mirrored-tests
// convention). `super` resolves to that module.
//
// The property under test is the P1.13 ordering the module doc states and
// nothing checked: the row is the truth, the event is its push. A test that
// only asserted the happy path would pass with the two swapped, and a run
// that announces a status it never persisted is exactly the drift the
// scheduler's skip persistence re-derives its state from.
//
// Two doubles, both of which refuse anything they were not told to answer.

use super::*;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId, WorkflowId};
use crate::domain::models::Feature;
use std::sync::Mutex;

const F_ID: &str = "f-status";

// ── doubles ─────────────────────────────────────────────────────────────────

/// A `FeatureRepository` that answers exactly one method — `step_update` —
/// with a scripted result, recording every patch it was handed. Every other
/// method panics rather than returning a plausible default.
struct FeaturesDouble {
    write_result: Result<(), String>,
    patches: Mutex<Vec<StepExecutionPatch>>,
}

impl FeaturesDouble {
    fn accepting() -> Self {
        Self {
            write_result: Ok(()),
            patches: Mutex::new(Vec::new()),
        }
    }

    fn refusing() -> Self {
        Self {
            write_result: Err("disk is on fire".to_string()),
            patches: Mutex::new(Vec::new()),
        }
    }

    fn patches(&self) -> Vec<StepExecutionPatch> {
        self.patches.lock().expect("not poisoned").clone()
    }
}

impl FeatureRepository for FeaturesDouble {
    fn step_update(&self, _id: &StepExecutionId, patch: &StepExecutionPatch) -> Result<(), String> {
        self.patches
            .lock()
            .expect("not poisoned")
            .push(patch.clone());
        self.write_result.clone()
    }
    fn get(&self, _id: &FeatureId) -> Result<Option<Feature>, String> {
        panic!("unscripted get")
    }
    fn get_active(&self, _p: &ProjectId) -> Result<Vec<Feature>, String> {
        panic!("unscripted get_active")
    }
    fn add(&self, _f: Feature) -> Result<(), String> {
        panic!("unscripted add")
    }
    fn update(
        &self,
        _id: &FeatureId,
        _patch: &crate::ports::db::FeaturePatch,
    ) -> Result<(), String> {
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

/// `(status, cost_usd, tokens, cache_read, cache_creation)` — the fields of one
/// `StepProgress` these tests assert on.
type ProgressRecord = (String, Option<f64>, Option<i64>, Option<u64>, Option<u64>);

/// Records every `StepProgress` it is handed; anything else is a test bug,
/// not a default to swallow.
#[derive(Default)]
struct NotifDouble {
    events: Mutex<Vec<ProgressRecord>>,
}

impl NotificationPort for NotifDouble {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        match event {
            DomainEvent::StepProgress {
                status,
                cost_usd,
                tokens,
                cache_read_input_tokens,
                cache_creation_input_tokens,
                ..
            } => self.events.lock().expect("not poisoned").push((
                status.clone(),
                *cost_usd,
                *tokens,
                *cache_read_input_tokens,
                *cache_creation_input_tokens,
            )),
            other => panic!("a step transition emits StepProgress, not {other:?}"),
        }
        Ok(())
    }
}

struct Harness {
    features: FeaturesDouble,
    notif: NotifDouble,
    f_id: FeatureId,
}

impl Harness {
    fn new(features: FeaturesDouble) -> Self {
        Self {
            features,
            notif: NotifDouble::default(),
            f_id: FeatureId::from(F_ID),
        }
    }

    fn writers(&self) -> StatusWriters<'_> {
        StatusWriters {
            features: &self.features,
            notif: &self.notif,
            f_id: &self.f_id,
        }
    }

    fn statuses(&self) -> Vec<String> {
        self.notif
            .events
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|e| e.0.clone())
            .collect()
    }
}

fn step_exec() -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-status"),
        feature_id: FeatureId::from(F_ID),
        step_id: StepId::from("s-implement"),
        step_index: 0,
        step_kind: "agent".to_string(),
        status: "running".to_string(),
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
        created_at: 1_700_000_000,
        updated_at: 1_700_000_000,
    }
}

// ── P1.13: the row is the truth, the event is its push ──────────────────────

/// A repository error means the transition did not happen. Announcing it
/// anyway would put a status on the user's screen that no restart, replay or
/// ready-set re-derivation can see — and the scheduler decides from the row.
#[test]
fn a_refused_write_emits_nothing() {
    let h = Harness::new(FeaturesDouble::refusing());

    let res = try_update_step_status(
        h.writers(),
        &step_exec(),
        StepTransition::failed(1.5, Some(20), 9, "boom".to_string(), CacheTokens::default()),
    );

    assert_eq!(res, Err("disk is on fire".to_string()));
    assert!(
        h.statuses().is_empty(),
        "a status nobody persisted must not reach the UI"
    );
}

/// The infallible form exists for callers with nothing useful to do about a
/// repository error — it still must not announce the transition it lost.
#[test]
fn the_infallible_form_swallows_the_error_without_emitting() {
    let h = Harness::new(FeaturesDouble::refusing());

    update_step_status(
        h.writers(),
        &step_exec(),
        StepTransition::completed(0.25, 7, 3, None, CacheTokens::default()),
    );

    assert!(h.statuses().is_empty());
}

/// The write happening is what earns the event, and the event carries the
/// same numbers the row just took.
#[test]
fn a_landed_write_emits_the_transition_it_persisted() {
    let h = Harness::new(FeaturesDouble::accepting());

    update_step_status(
        h.writers(),
        &step_exec(),
        StepTransition::completed(
            2.5,
            41,
            12,
            Some("art/plan.md".to_string()),
            CacheTokens {
                read: Some(900),
                creation: Some(30),
            },
        ),
    );

    let patches = h.features.patches();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].status.as_deref(), Some("completed"));
    assert_eq!(patches[0].cost_usd, Some(Some(2.5)));
    assert_eq!(patches[0].tokens, Some(Some(41)));
    assert_eq!(patches[0].wall_clock_secs, Some(Some(12)));

    let events = h.notif.events.lock().expect("not poisoned").clone();
    assert_eq!(
        events,
        vec![(
            "completed".to_string(),
            Some(2.5),
            Some(41),
            Some(900),
            Some(30)
        )],
        "the event repeats the row, cache telemetry included"
    );
}

// ── the artifact column's two meanings ──────────────────────────────────────

/// `None` is "I have nothing to say about the artifact", **not** "clear it".
/// The double `Option` in the patch is what distinguishes them, and a
/// completed step whose handler already wrote its own path depends on it.
#[test]
fn a_none_artifact_path_leaves_the_column_alone() {
    let h = Harness::new(FeaturesDouble::accepting());

    update_step_status(
        h.writers(),
        &step_exec(),
        StepTransition::completed(0.0, 0, 1, None, CacheTokens::default()),
    );

    assert_eq!(
        h.features.patches()[0].artifact_path,
        None,
        "an untouched column is `None`, not `Some(None)`"
    );
}

/// A path the caller does have is written through as the column's new value.
#[test]
fn a_supplied_artifact_path_is_written() {
    let h = Harness::new(FeaturesDouble::accepting());

    update_step_status(
        h.writers(),
        &step_exec(),
        StepTransition::completed(
            0.0,
            0,
            1,
            Some("art/spec.md".to_string()),
            CacheTokens::default(),
        ),
    );

    assert_eq!(
        h.features.patches()[0].artifact_path,
        Some(Some("art/spec.md".to_string()))
    );
}

// ── each constructor fixes its own status ───────────────────────────────────

/// The statuses are the run loop's vocabulary: the ready-set derivation reads
/// them back out of the row (`completed` terminal, everything else pending),
/// so a constructor writing the wrong word silently re-schedules a step.
#[test]
fn every_constructor_writes_the_status_it_is_named_for() {
    let cases = [
        (StepTransition::running(0.0, None, 0), "running"),
        (
            StepTransition::completed(0.0, 0, 0, None, CacheTokens::default()),
            "completed",
        ),
        (
            StepTransition::failed(0.0, None, 0, "e".to_string(), CacheTokens::default()),
            "failed",
        ),
        (
            StepTransition::interrupted(0.0, 0, 0, "e".to_string(), CacheTokens::default()),
            "interrupted",
        ),
        (
            StepTransition::pending(0.0, 0, 0, "e".to_string(), CacheTokens::default()),
            "pending",
        ),
        (
            StepTransition::skipped("skipped", 0.0, None, 0, "e".to_string()),
            "skipped",
        ),
    ];

    for (transition, expected) in cases {
        let h = Harness::new(FeaturesDouble::accepting());
        update_step_status(h.writers(), &step_exec(), transition);
        assert_eq!(h.statuses(), vec![expected.to_string()]);
    }
}

/// A transition that reports success has no failure to report, and one that
/// reports a failure has no artifact to point at. Encoding that in the
/// constructors is what keeps a `completed` row from carrying a stale error.
#[test]
fn a_success_carries_no_error_and_a_failure_carries_no_artifact() {
    let h = Harness::new(FeaturesDouble::accepting());
    update_step_status(
        h.writers(),
        &step_exec(),
        StepTransition::completed(0.0, 0, 0, None, CacheTokens::default()),
    );
    assert_eq!(h.features.patches()[0].error_message, None);

    let h = Harness::new(FeaturesDouble::accepting());
    update_step_status(
        h.writers(),
        &step_exec(),
        StepTransition::failed(0.0, None, 0, "boom".to_string(), CacheTokens::default()),
    );
    let patch = &h.features.patches()[0];
    assert_eq!(patch.artifact_path, None);
    assert_eq!(patch.error_message, Some(Some("boom".to_string())));
}
