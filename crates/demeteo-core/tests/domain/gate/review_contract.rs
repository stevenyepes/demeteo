//! The review contract against the Standard pipeline as it ships, so a
//! bundled critic step that stops declaring its review fails here.

use super::*;
use crate::domain::workflow_starters::StarterDefinition;

fn standard_steps() -> Vec<StepConfig> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/workflows/standard-feature-pipeline.json");
    let json = std::fs::read_to_string(path).expect("the bundled Standard pipeline is readable");
    StarterDefinition::parse(&json)
        .expect("the bundled Standard pipeline is JSON")
        .steps
}

fn index_of(steps: &[StepConfig], id: &str) -> usize {
    steps
        .iter()
        .position(|s| s.id.as_ref() == id)
        .expect("the step is in the workflow")
}

fn review(critical: &str, major: &str, minor: &str, verdict: &str, block: &str) -> String {
    format!(
        "## Critical Issues\n{critical}\n\n## Major Issues\n{major}\n\n## Minor Issues\n{minor}\n\n\
         ## Verdict\n{verdict}\n\n{block}\n"
    )
}

fn counts(critical: u32, major: u32, minor: u32) -> String {
    format!(r#"{{"review": {{"critical": {critical}, "major": {major}, "minor": {minor}}}}}"#)
}

#[test]
fn a_review_with_nothing_listed_is_clean() {
    let text = review("None.", "None.", "None.", "PASS", &counts(0, 0, 0));
    assert_eq!(standing(&text), ReviewStanding::Clean);
}

#[test]
fn minor_issues_do_not_hold_the_gate() {
    let text = review(
        "None.",
        "None.",
        "- rename `tmp`",
        "PASS_WITH_NOTES",
        &counts(0, 0, 1),
    );
    assert_eq!(standing(&text), ReviewStanding::Clean);
}

#[test]
fn a_major_issue_blocks_even_with_no_critical_one() {
    let text = review(
        "None.",
        "- no test for the error path",
        "None.",
        "FAIL",
        &counts(0, 1, 0),
    );
    assert_eq!(standing(&text), ReviewStanding::Blocking);
}

#[test]
fn a_critical_issue_blocks() {
    let text = review(
        "- path traversal",
        "None.",
        "None.",
        "**FAIL**",
        &counts(1, 0, 0),
    );
    assert_eq!(standing(&text), ReviewStanding::Blocking);
}

/// The case the contract exists for: the old prompt let a critic list major
/// issues under a passing verdict.
#[test]
fn a_passing_verdict_over_major_issues_is_not_trusted() {
    let text = review(
        "None.",
        "- no test for the error path",
        "None.",
        "PASS_WITH_NOTES",
        &counts(0, 1, 0),
    );
    assert!(matches!(standing(&text), ReviewStanding::Unreadable(_)));
}

#[test]
fn a_failing_verdict_over_clean_counts_is_not_trusted() {
    let text = review("None.", "None.", "None.", "FAIL", &counts(0, 0, 0));
    assert!(matches!(standing(&text), ReviewStanding::Unreadable(_)));
}

#[test]
fn a_review_without_counts_is_unreadable_however_it_reads() {
    let text = review("None.", "None.", "None.", "PASS", "");
    assert!(matches!(standing(&text), ReviewStanding::Unreadable(_)));
}

#[test]
fn a_review_without_a_verdict_is_unreadable() {
    let text = format!("## Critical Issues\nNone.\n\n{}\n", counts(0, 0, 0));
    assert!(matches!(standing(&text), ReviewStanding::Unreadable(_)));
}

#[test]
fn counts_that_are_not_the_declared_shape_are_unreadable() {
    for block in [
        r#"{"review": {"critical": 0, "major": 0}}"#,
        r#"{"review": {"critical": "none", "major": 0, "minor": 0}}"#,
        r#"{"review": {"critical": -1, "major": 0, "minor": 0}}"#,
        r#"{"critical": 0, "major": 0, "minor": 0}"#,
    ] {
        let text = review("None.", "None.", "None.", "PASS", block);
        assert!(
            matches!(standing(&text), ReviewStanding::Unreadable(_)),
            "{block} should not be read as counts"
        );
    }
}

/// A critic that quotes the contract's zeroed example before its own block
/// has to be read by the block it ended on.
#[test]
fn the_last_counts_block_is_the_one_read() {
    let text = review(
        "- path traversal",
        "None.",
        "None.",
        "FAIL",
        &format!("{}\n\n{}", counts(0, 0, 0), counts(1, 0, 0)),
    );
    assert_eq!(standing(&text), ReviewStanding::Blocking);
}

#[test]
fn a_fenced_counts_block_is_read() {
    let block = format!("```json\n{}\n```", counts(0, 0, 2));
    let text = review("None.", "None.", "- a\n- b", "PASS_WITH_NOTES", &block);
    assert_eq!(standing(&text), ReviewStanding::Clean);
}

/// Ask and parse are one contract: what the section shows the agent is what
/// `standing` reads.
#[test]
fn the_example_the_contract_shows_is_one_the_reader_accepts() {
    let section = contract_section("artifacts/critic-review.md");
    let example = section
        .lines()
        .find(|line| line.starts_with("{\"review\""))
        .expect("the contract shows the counts object on its own line");
    let text = review("None.", "None.", "None.", "PASS", example);
    assert_eq!(standing(&text), ReviewStanding::Clean);
    for heading in [
        "## Critical Issues",
        "## Major Issues",
        "## Minor Issues",
        "## Verdict",
    ] {
        assert!(section.contains(heading), "{heading} is not asked for");
    }
}

#[test]
fn the_shipped_critic_is_given_the_contract_at_the_path_it_declares() {
    let steps = standard_steps();
    let critic = &steps[index_of(&steps, "s-critic")];

    let prompt = append_contract("Review the change.".to_string(), critic);

    assert!(prompt.starts_with("Review the change."));
    assert!(prompt.contains("## Review Format (orchestrator contract)"));
    assert!(prompt.contains("`artifacts/critic-review.md`"));
}

#[test]
fn a_step_that_declares_no_review_keeps_its_prompt() {
    let steps = standard_steps();
    let validate = &steps[index_of(&steps, "s-validate")];
    assert_eq!(
        append_contract("Validate.".to_string(), validate),
        "Validate."
    );
}

#[test]
fn a_review_naming_an_artifact_the_step_does_not_declare_is_no_review() {
    let steps = standard_steps();
    let mut critic = steps[index_of(&steps, "s-critic")].clone();
    critic.review_artifact = Some("not-declared".to_string());
    assert_eq!(review_path(&critic), None);
}

#[test]
fn the_shipped_ship_gate_is_covered_by_the_critic() {
    let steps = standard_steps();
    let covering = review_step_for_gate(&steps, index_of(&steps, "s-gate-ship"))
        .expect("the ship gate has a review step before it");
    assert_eq!(covering.id.as_ref(), "s-critic");
}

/// The Standard pipeline's earlier gates follow steps that review nothing.
#[test]
fn a_gate_that_does_not_follow_a_review_step_is_not_covered() {
    let steps = standard_steps();
    let uncovered = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == "gate" && s.id.as_ref() != "s-gate-ship")
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    assert!(!uncovered.is_empty());
    for idx in uncovered {
        assert!(review_step_for_gate(&steps, idx).is_none());
    }
}

/// Work done after the review is work the review did not see.
#[test]
fn a_step_between_the_review_and_the_gate_uncovers_it() {
    let mut steps = standard_steps();
    let gate = index_of(&steps, "s-gate-ship");
    let intruder = steps[index_of(&steps, "s-validate")].clone();
    steps.insert(gate, intruder);
    assert!(review_step_for_gate(&steps, gate + 1).is_none());
}

#[test]
fn another_gate_between_the_review_and_the_gate_does_not_uncover_it() {
    let mut steps = standard_steps();
    let gate = index_of(&steps, "s-gate-ship");
    let mut extra = steps[gate].clone();
    extra.gate_class = None;
    steps.insert(gate, extra);
    let covering = review_step_for_gate(&steps, gate + 1).expect("still covered");
    assert_eq!(covering.id.as_ref(), "s-critic");
}
