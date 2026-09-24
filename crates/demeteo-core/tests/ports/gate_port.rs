// Contract test for `GateRepository`, included from
// `crates/demeteo-core/src/ports/db.rs` (mirrored-tests convention).
// `super` = that module.
//
// One body, two implementations, as in `sequence_resume_port.rs`: every
// `#[test]` runs against the real `SqliteAdapter` and an in-memory double.
//
// What they pin is the difference between `create` and `reopen` on a step
// execution that already has a row. A synthetic gate's row id is derived
// from the step execution, so a node interrupted twice is asked twice on one
// row — and a park consumes any answer it finds there without waiting. A
// `create` that kept the first answer is how a restart resumed over a moved
// workspace with nobody asked.

use super::*;

use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::GateDecisionId;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Mutex;

const SE: &str = "se-f-1-s-implement";

/// In-memory `GateRepository`, transcribed from
/// `adapters/database/repos/gate.rs`: one row per step execution.
#[derive(Default)]
struct InMemoryGates {
    rows: Mutex<HashMap<String, GateDecision>>,
}

impl GateRepository for InMemoryGates {
    fn create(&self, g: GateDecision) -> Result<(), String> {
        let mut rows = self.rows.lock().unwrap();
        if rows.contains_key(&g.step_execution_id.0) {
            return Err("UNIQUE constraint failed: gate_decisions.step_execution_id".into());
        }
        rows.insert(g.step_execution_id.0.clone(), g);
        Ok(())
    }

    fn reopen(&self, g: GateDecision) -> Result<(), String> {
        self.rows
            .lock()
            .unwrap()
            .insert(g.step_execution_id.0.clone(), g);
        Ok(())
    }

    fn upsert_decision(
        &self,
        step_execution_id: &StepExecutionId,
        decision: &str,
        feedback: Option<&str>,
        created_at: i64,
    ) -> Result<(), String> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows
            .entry(step_execution_id.0.clone())
            .or_insert_with(|| GateDecision {
                id: GateDecisionId::from(format!("gd-{}", step_execution_id.0)),
                step_execution_id: step_execution_id.clone(),
                decision: None,
                feedback: None,
                created_at,
            });
        row.decision = Some(decision.to_string());
        row.feedback = feedback.map(str::to_string);
        Ok(())
    }

    fn decide(
        &self,
        step_execution_id: &StepExecutionId,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), String> {
        if let Some(row) = self.rows.lock().unwrap().get_mut(&step_execution_id.0) {
            row.decision = Some(decision.to_string());
            row.feedback = feedback.map(str::to_string);
        }
        Ok(())
    }

    fn pending_for_feature(&self, _: &FeatureId) -> Result<Option<GateDecision>, String> {
        Err("not part of this contract".into())
    }

    fn latest_decided_for_feature(&self, _: &FeatureId) -> Result<Option<GateDecision>, String> {
        Err("not part of this contract".into())
    }

    fn all_decided_for_feature(&self, _: &FeatureId) -> Result<Vec<GateDecision>, String> {
        Err("not part of this contract".into())
    }

    fn latest_for_step(
        &self,
        step_execution_id: &StepExecutionId,
    ) -> Result<Option<GateDecision>, String> {
        Ok(self.rows.lock().unwrap().get(&step_execution_id.0).cloned())
    }

    fn reset_for_step_execution(&self, step_execution_id: &StepExecutionId) -> Result<(), String> {
        self.rows.lock().unwrap().remove(&step_execution_id.0);
        Ok(())
    }
}

fn against_every_impl(body: impl Fn(&dyn GateRepository, &str)) {
    let sqlite = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    {
        let conn = sqlite.conn.lock().unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0);
             INSERT INTO features (id, project_id, title, created_at)
             VALUES ('f-1', 'p-1', 'interrupted twice', 0);
             INSERT INTO step_executions
                 (id, feature_id, step_id, step_index, step_kind, status, created_at, updated_at)
             VALUES ('{SE}', 'f-1', 's-implement', 0, 'agent', 'interrupted', 0, 0);"
        ))
        .unwrap();
    }
    body(&sqlite, "SqliteAdapter");
    body(&InMemoryGates::default(), "InMemoryGates");
}

fn se() -> StepExecutionId {
    StepExecutionId::from(SE.to_string())
}

fn question(created_at: i64) -> GateDecision {
    GateDecision {
        id: GateDecisionId::from(format!("gd-syn-{SE}")),
        step_execution_id: se(),
        decision: None,
        feedback: None,
        created_at,
    }
}

#[test]
fn reopen_discards_the_answer_to_an_earlier_question() {
    against_every_impl(|gates, name| {
        gates.reopen(question(1)).unwrap();
        gates
            .upsert_decision(&se(), "approve", Some("ok"), 2)
            .unwrap();

        gates.reopen(question(3)).unwrap();

        let row = gates.latest_for_step(&se()).unwrap().unwrap();
        assert_eq!(row.decision, None, "{name}: earlier approval survived");
        assert_eq!(row.feedback, None, "{name}");
        assert_eq!(row.created_at, 3, "{name}");
    });
}

#[test]
fn reopen_inserts_when_there_is_no_row() {
    against_every_impl(|gates, name| {
        gates.reopen(question(1)).unwrap();
        let row = gates.latest_for_step(&se()).unwrap().unwrap();
        assert_eq!(row.id.0, format!("gd-syn-{SE}"), "{name}");
        assert_eq!(row.decision, None, "{name}");
        assert_eq!(row.created_at, 1, "{name}");
    });
}

#[test]
fn create_keeps_the_answer_to_an_earlier_question() {
    against_every_impl(|gates, name| {
        gates.create(question(1)).unwrap();
        gates.upsert_decision(&se(), "approve", None, 2).unwrap();

        assert!(gates.create(question(3)).is_err(), "{name}");

        let row = gates.latest_for_step(&se()).unwrap().unwrap();
        assert_eq!(row.decision.as_deref(), Some("approve"), "{name}");
    });
}
