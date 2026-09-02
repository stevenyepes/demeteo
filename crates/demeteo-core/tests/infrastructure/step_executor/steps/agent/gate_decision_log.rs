// Tests extracted from
// `crates/demeteo-core/src/adapters/step_executor/steps/agent/gate_decision.rs`
// (mirrored-tests convention). `super` = that module.
//
// The double errors on every method it was not explicitly told to answer
// (AGENTS.md §7): a `GateRepository` stub returning `Ok(vec![])` for
// everything would let this pass while reading the wrong port method
// entirely.

use super::*;
use crate::domain::ids::StepExecutionId;
use crate::domain::models::GateDecision;

const F_ID: &str = "f-1";

struct GateDouble {
    decided: Vec<GateDecision>,
    fail_read: bool,
}

macro_rules! unscripted {
    ($($name:ident($($arg:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&self, $(_: $arg),*) -> $ret {
            panic!(concat!("unscripted GateRepository::", stringify!($name)))
        })*
    };
}

impl GateRepository for GateDouble {
    fn all_decided_for_feature(&self, feature_id: &FeatureId) -> Result<Vec<GateDecision>, String> {
        if feature_id.0 != F_ID {
            return Err(format!("unscripted feature {}", feature_id.0));
        }
        if self.fail_read {
            return Err("gate rows could not be read".to_string());
        }
        Ok(self.decided.clone())
    }

    unscripted! {
        create(GateDecision) -> Result<(), String>;
        pending_for_feature(&FeatureId) -> Result<Option<GateDecision>, String>;
        latest_decided_for_feature(&FeatureId) -> Result<Option<GateDecision>, String>;
        latest_for_step(&StepExecutionId) -> Result<Option<GateDecision>, String>;
        reset_for_step_execution(&StepExecutionId) -> Result<(), String>;
    }

    fn upsert_decision(
        &self,
        _: &StepExecutionId,
        _: &str,
        _: Option<&str>,
        _: i64,
    ) -> Result<(), String> {
        panic!("unscripted GateRepository::upsert_decision")
    }

    fn decide(&self, _: &StepExecutionId, _: &str, _: Option<&str>) -> Result<(), String> {
        panic!("unscripted GateRepository::decide")
    }
}

fn decision(step_exec_id: &str, decision: &str, feedback: Option<&str>) -> GateDecision {
    GateDecision {
        id: format!("gd-{step_exec_id}").into(),
        step_execution_id: StepExecutionId::from(step_exec_id.to_string()),
        decision: Some(decision.to_string()),
        feedback: feedback.map(str::to_string),
        created_at: 1,
    }
}

fn step_exec(id: &str, step_id: &str) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from(id.to_string()),
        feature_id: FeatureId::from(F_ID.to_string()),
        step_id: crate::domain::ids::StepId::from(step_id.to_string()),
        step_index: 0,
        step_kind: "gate".to_string(),
        status: "completed".to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: vec![],
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// The log names the node, not the row id — a validator reading it has the
/// workflow in front of it and has never seen a `step_execution_id`.
#[test]
fn a_decision_is_labelled_with_its_node_id() {
    let gates = GateDouble {
        decided: vec![decision("se-f-1-s-gate-review", "approve", Some("ship it"))],
        fail_read: false,
    };
    let execs = vec![step_exec("se-f-1-s-gate-review", "s-gate-review")];

    let out = gate_decision_log(&gates, &execs, F_ID);
    assert!(out.contains("s-gate-review"), "{out}");
    assert!(out.contains("ship it"), "{out}");
    assert!(
        !out.contains("se-f-1-s-gate-review"),
        "the row id should not leak into the prompt: {out}"
    );
}

/// A decision whose step row is missing still appears. Dropping it would
/// make the log silently shorter than the truth, which is the one thing a
/// block about "what was approved" must never be.
#[test]
fn a_decision_with_no_matching_step_row_still_appears() {
    let gates = GateDouble {
        decided: vec![decision("se-orphan", "approve", None)],
        fail_read: false,
    };

    let out = gate_decision_log(&gates, &[], F_ID);
    assert!(out.contains("se-orphan"), "{out}");
    assert!(out.contains("approve"), "{out}");
}

/// Best-effort: a prompt is worth less without the block, not nothing.
#[test]
fn an_unreadable_gate_table_renders_no_block_rather_than_failing() {
    let gates = GateDouble {
        decided: vec![],
        fail_read: true,
    };
    assert_eq!(gate_decision_log(&gates, &[], F_ID), "");
}

#[test]
fn a_run_with_no_decided_gates_renders_no_block() {
    let gates = GateDouble {
        decided: vec![],
        fail_read: false,
    };
    assert_eq!(gate_decision_log(&gates, &[], F_ID), "");
}
