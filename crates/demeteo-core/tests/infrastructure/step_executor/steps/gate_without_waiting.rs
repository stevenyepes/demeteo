// Tests extracted from
// `crates/demeteo-core/src/adapters/step_executor/steps/gate/without_waiting.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

use crate::domain::ids::{FeatureId, GateDecisionId};
use std::sync::Mutex;

const SE: &str = "se-f-1-s-gate-review";

/// One gate row, with only the two calls this module makes scripted. Any
/// other call is a change in what the function touches, and fails loudly.
#[derive(Default)]
struct OneGate {
    row: Mutex<Option<GateDecision>>,
    refuse_writes: bool,
}

impl OneGate {
    fn holding(decision: Option<&str>) -> Self {
        Self {
            row: Mutex::new(Some(GateDecision {
                id: GateDecisionId::from(format!("gd-{SE}")),
                step_execution_id: se(),
                decision: decision.map(str::to_string),
                feedback: None,
                created_at: 1,
                auto_approved: false,
            })),
            refuse_writes: false,
        }
    }

    fn stored(&self) -> Option<GateDecision> {
        self.row.lock().unwrap().clone()
    }
}

macro_rules! unscripted {
    ($($name:ident($($arg:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&self, $(_: $arg),*) -> $ret {
            panic!(concat!("unscripted GateRepository::", stringify!($name)))
        })*
    };
}

impl GateRepository for OneGate {
    fn latest_for_step(&self, id: &StepExecutionId) -> Result<Option<GateDecision>, String> {
        assert_eq!(id.0, SE);
        Ok(self.stored())
    }

    fn approve_by_policy(&self, id: &StepExecutionId, created_at: i64) -> Result<(), String> {
        assert_eq!(id.0, SE);
        if self.refuse_writes {
            return Err("database is locked".to_string());
        }
        *self.row.lock().unwrap() = Some(GateDecision {
            id: GateDecisionId::from(format!("gd-{SE}")),
            step_execution_id: id.clone(),
            decision: Some("approve".to_string()),
            feedback: None,
            created_at,
            auto_approved: true,
        });
        Ok(())
    }

    unscripted! {
        create(GateDecision) -> Result<(), String>;
        reopen(GateDecision) -> Result<(), String>;
        upsert_decision(&StepExecutionId, &str, Option<&str>, i64) -> Result<(), String>;
        decide(&StepExecutionId, &str, Option<&str>) -> Result<(), String>;
        pending_for_feature(&FeatureId) -> Result<Option<GateDecision>, String>;
        latest_decided_for_feature(&FeatureId) -> Result<Option<GateDecision>, String>;
        all_decided_for_feature(&FeatureId) -> Result<Vec<GateDecision>, String>;
        reset_for_step_execution(&StepExecutionId) -> Result<(), String>;
    }
}

fn se() -> StepExecutionId {
    StepExecutionId::from(SE.to_string())
}

/// For every gate whose answer does not turn on a review: reading one is a
/// change in what the function touches, and fails loudly.
fn ask(gates: &OneGate, autonomy: GateAutonomy, dangerous: bool) -> Option<Unasked> {
    decided_without_waiting(
        gates,
        &se(),
        autonomy,
        dangerous,
        || panic!("this gate's answer does not turn on a review"),
        7,
    )
}

fn ask_ship_gate_with(gates: &OneGate, review: ReviewStanding) -> Option<Unasked> {
    decided_without_waiting(gates, &se(), GateAutonomy::Clean, true, || review, 7)
}

#[test]
fn an_attended_project_asks_and_writes_nothing() {
    let gates = OneGate::default();
    assert!(ask(&gates, GateAutonomy::Attended, false).is_none());
    assert!(gates.stored().is_none());
}

#[test]
fn a_review_gate_approves_itself_and_says_whose_approval_it_was() {
    let gates = OneGate::default();
    let Some(Unasked::ApprovedByPolicy(row)) = ask(&gates, GateAutonomy::Review, false) else {
        panic!("a review gate under `review` autonomy should approve itself");
    };
    assert_eq!(row.decision.as_deref(), Some("approve"));
    assert!(row.auto_approved);
}

#[test]
fn the_ship_gate_still_asks_under_review_autonomy() {
    let gates = OneGate::default();
    assert!(ask(&gates, GateAutonomy::Review, true).is_none());
    assert!(gates.stored().is_none());
}

#[test]
fn the_ship_gate_approves_itself_under_full_autonomy() {
    let gates = OneGate::default();
    assert!(matches!(
        ask(&gates, GateAutonomy::Full, true),
        Some(Unasked::ApprovedByPolicy(_))
    ));
}

/// The policy writes over the row, so a person's recorded refusal has to be
/// read before it — or a restart turns "cancel" into an approval.
#[test]
fn a_recorded_human_answer_wins_over_the_policy() {
    let gates = OneGate::holding(Some("cancel"));
    let Some(Unasked::Recorded(row)) = ask(&gates, GateAutonomy::Full, true) else {
        panic!("a recorded answer should be returned as recorded");
    };
    assert_eq!(row.decision.as_deref(), Some("cancel"));
    assert_eq!(gates.stored().unwrap().decision.as_deref(), Some("cancel"));
}

/// An undecided row left by the startup watchdog is a question, not an
/// answer: the policy may still approve it.
#[test]
fn an_undecided_row_is_still_approved_by_the_policy() {
    let gates = OneGate::holding(None);
    assert!(matches!(
        ask(&gates, GateAutonomy::Review, false),
        Some(Unasked::ApprovedByPolicy(_))
    ));
}

#[test]
fn an_approval_that_was_not_recorded_is_not_given() {
    let gates = OneGate {
        refuse_writes: true,
        ..OneGate::default()
    };
    assert!(ask(&gates, GateAutonomy::Full, false).is_none());
}

#[test]
fn clean_autonomy_ships_on_a_clean_review_and_says_whose_approval_it_was() {
    let gates = OneGate::default();
    let Some(Unasked::ApprovedByPolicy(row)) = ask_ship_gate_with(&gates, ReviewStanding::Clean)
    else {
        panic!("a clean review should approve the ship gate under `clean` autonomy");
    };
    assert!(row.auto_approved);
}

#[test]
fn clean_autonomy_asks_on_a_review_that_blocks_or_cannot_be_read() {
    for review in [
        ReviewStanding::Blocking,
        ReviewStanding::Unreadable("no counts"),
    ] {
        let gates = OneGate::default();
        assert!(ask_ship_gate_with(&gates, review).is_none());
        assert!(gates.stored().is_none());
    }
}

/// `ask` panics if the review is read, so this is also the proof that a
/// review gate under `clean` reads none.
#[test]
fn clean_autonomy_approves_a_review_gate_without_reading_any_review() {
    let gates = OneGate::default();
    assert!(matches!(
        ask(&gates, GateAutonomy::Clean, false),
        Some(Unasked::ApprovedByPolicy(_))
    ));
}
