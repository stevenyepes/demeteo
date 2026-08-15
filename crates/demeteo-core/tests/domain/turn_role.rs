// Tests extracted from `crates/demeteo-core/src/domain/turn_role.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

fn step_using_agent_skills() -> StepConfig {
    StepConfig {
        uses_agent_skills: true,
        ..Default::default()
    }
}

#[test]
fn a_step_turn_takes_the_workflow_authors_answer() {
    assert!(TurnRole::Step(&step_using_agent_skills()).keeps_harness_personalization());
    assert!(!TurnRole::Step(&StepConfig::default()).keeps_harness_personalization());
}

/// The arm a "make this consistent" refactor would flatten. A skill the
/// reviewed repository committed must not reach the turn that decides whether
/// the run terminates, whatever the step around it asked for.
#[test]
fn an_orchestrator_turn_keeps_nothing_whatever_the_step_asked() {
    let opted_in = step_using_agent_skills();
    assert!(TurnRole::Step(&opted_in).keeps_harness_personalization());
    assert!(!TurnRole::Orchestrator.keeps_harness_personalization());
}

#[test]
fn an_interactive_session_keeps_the_users_own_setup() {
    assert!(TurnRole::Interactive.keeps_harness_personalization());
}
