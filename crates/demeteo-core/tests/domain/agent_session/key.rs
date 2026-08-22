// Tests extracted from `crates/demeteo-core/src/domain/agent_session/key.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::ids::StepId;

fn step() -> StepConfig {
    StepConfig {
        effort: None,
        id: StepId::from("s-impl".to_string()),
        kind: "agent".to_string(),
        title: "Implement".to_string(),
        agent_kind: None,
        model: None,
        prompt_template: None,
        on_failure: None,
        max_iterations: None,
        artifacts: None,
        verifier: None,
        capability: None,
        allow_network: false,
        allow_shell: false,
        gate_class: None,
        task_list_from: None,
        ..Default::default()
    }
}

/// AC6 — regression guard. Two efforts must produce two keys (see
/// `agent_session_key` doc comment).
#[test]
fn session_key_distinguishes_two_efforts() {
    let s = step();
    let low = agent_session_key("f-1", &s, Some("m"), EffortLevel::Low);
    let max = agent_session_key("f-1", &s, Some("m"), EffortLevel::Max);
    assert_ne!(
        low, max,
        "a change in effort alone must force a fresh session"
    );
    assert_eq!(
        low,
        agent_session_key("f-1", &s, Some("m"), EffortLevel::Low),
        "the same effort shares one key (the --resume cache hit exists to preserve)"
    );
}

/// Same shape as the effort guard above: the flag is spelled into argv at
/// spawn and the session freezes its context there, so two steps sharing a key
/// across it would run the second on the first's answer — a review step told
/// it kept the user's skills, executing with them stripped.
#[test]
fn session_key_distinguishes_a_step_that_keeps_the_harness_personalization() {
    let bare = step();
    let keeping = StepConfig {
        uses_agent_skills: true,
        ..step()
    };
    assert_ne!(
        agent_session_key("f-1", &bare, Some("m"), EffortLevel::High),
        agent_session_key("f-1", &keeping, Some("m"), EffortLevel::High),
    );
}

/// Sanity: identical inputs → identical keys (the fingerprint is
/// deterministic, not random).
#[test]
fn session_key_same_effort_shares_key() {
    let s = step();
    let a = agent_session_key("f-1", &s, Some("m"), EffortLevel::High);
    let b = agent_session_key("f-1", &s, Some("m"), EffortLevel::High);
    assert_eq!(a, b);
}
