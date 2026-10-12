//! The review contract against the Standard pipeline as it ships, so a
//! bundled critic step that stops declaring its review fails here.

use super::*;
use crate::domain::ids::WorkflowId;
use crate::domain::models::workflow_migrate::migrate_v1_to_v2;
use crate::domain::models::workflow_v2::EdgeConfig;
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

fn step_id(id: &str) -> StepId {
    StepId::from(id.to_string())
}

/// A Standard-pipeline step under another id, with nothing that ties it to
/// its old neighbours.
fn pick(source: &str, id: &str) -> StepConfig {
    let steps = standard_steps();
    let mut step = steps[index_of(&steps, source)].clone();
    step.id = step_id(id);
    step.task_list_from = None;
    step.on_failure = None;
    step
}

/// `steps` as the chain a list-shaped workflow is, or wired by `edges`.
fn graph_of(steps: &[StepConfig], edges: Option<&[(&str, &str)]>) -> WorkflowGraph {
    let mut def = migrate_v1_to_v2(WorkflowId::from("wf".to_string()), "wf", steps);
    if let Some(edges) = edges {
        def.edges = edges
            .iter()
            .map(|(from, to)| EdgeConfig {
                from: step_id(from),
                to: step_id(to),
                when: None,
            })
            .collect();
    }
    WorkflowGraph::build(&def).expect("the fixture is an acyclic graph")
}

fn covering<'a>(steps: &'a [StepConfig], graph: &WorkflowGraph, gate: &str) -> Option<&'a str> {
    review_for_gate(steps, graph, &step_id(gate)).map(|review| review.step.id.as_ref())
}

/// `i`, `j` and `b` do work, `r` reviews, `g` is the ship gate.
fn canvas_steps() -> Vec<StepConfig> {
    vec![
        pick("s-research", "i"),
        pick("s-research", "j"),
        pick("s-validate", "b"),
        pick("s-critic", "r"),
        pick("s-gate-ship", "g"),
    ]
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

/// The file ends on its counts; a fenced quote of the shape above them is an
/// illustration, however it is dressed.
#[test]
fn a_fenced_quote_of_the_example_does_not_outrank_the_counts_the_file_ends_on() {
    let ending = review(
        "None.",
        "- no test for the error path",
        "None.",
        "PASS_WITH_NOTES",
        &counts(0, 1, 0),
    );
    let quoting = format!(
        "The format asked for:\n```json\n{}\n```\n\n{ending}",
        counts(0, 0, 0)
    );
    assert_eq!(standing(&quoting), standing(&ending));
    assert!(matches!(standing(&quoting), ReviewStanding::Unreadable(_)));
}

/// One unbalanced brace in a quoted snippet hides every bare object after it
/// from a brace scan; the fence around the counts is what still finds them.
#[test]
fn fenced_counts_are_read_past_an_unbalanced_brace_in_the_prose() {
    let block = format!("```json\n{}\n```", counts(0, 0, 1));
    let text = review(
        "None.",
        "None.",
        "- `if ready {` reads better as a guard",
        "PASS_WITH_NOTES",
        &block,
    );
    assert_eq!(standing(&text), ReviewStanding::Clean);
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
    let graph = graph_of(&steps, None);
    let review = review_for_gate(&steps, &graph, &step_id("s-gate-ship"))
        .expect("the ship gate has a review step before it");
    assert_eq!(review.step.id.as_ref(), "s-critic");
    assert_eq!(review.path, "artifacts/critic-review.md");
}

/// The Standard pipeline's earlier gates follow steps that review nothing.
#[test]
fn a_gate_that_does_not_follow_a_review_step_is_not_covered() {
    let steps = standard_steps();
    let graph = graph_of(&steps, None);
    let uncovered = steps
        .iter()
        .filter(|s| s.kind == "gate" && s.id.as_ref() != "s-gate-ship")
        .collect::<Vec<_>>();
    assert!(!uncovered.is_empty());
    for gate in uncovered {
        assert_eq!(covering(&steps, &graph, gate.id.as_ref()), None);
    }
}

/// Work done after the review is work the review did not see.
#[test]
fn a_step_between_the_review_and_the_gate_uncovers_it() {
    let mut steps = standard_steps();
    let gate = index_of(&steps, "s-gate-ship");
    steps.insert(gate, pick("s-validate", "s-intruder"));
    let graph = graph_of(&steps, None);
    assert_eq!(covering(&steps, &graph, "s-gate-ship"), None);
}

#[test]
fn another_gate_between_the_review_and_the_gate_does_not_uncover_it() {
    let mut steps = standard_steps();
    let gate = index_of(&steps, "s-gate-ship");
    let mut extra = pick("s-gate-ship", "s-gate-extra");
    extra.gate_class = None;
    steps.insert(gate, extra);
    let graph = graph_of(&steps, None);
    assert_eq!(covering(&steps, &graph, "s-gate-ship"), Some("s-critic"));
}

/// `b` feeds the gate beside the review, so it is free to run after it —
/// and does, in the order the scheduler picks, while sitting before the
/// review in the list the canvas saves. A redirect to `b` would also re-run
/// it without re-running the review.
#[test]
fn a_branch_that_joins_the_gate_beside_the_review_uncovers_it() {
    let steps = canvas_steps();
    let graph = graph_of(
        &steps,
        Some(&[("i", "r"), ("j", "b"), ("r", "g"), ("b", "g")]),
    );
    assert_eq!(covering(&steps, &graph, "g"), None);
}

#[test]
fn branches_that_join_before_the_review_leave_the_gate_covered() {
    let steps = canvas_steps();
    let graph = graph_of(
        &steps,
        Some(&[("i", "r"), ("j", "b"), ("b", "r"), ("r", "g")]),
    );
    assert_eq!(covering(&steps, &graph, "g"), Some("r"));
}

/// Not upstream of the gate at all, and still able to run before it.
#[test]
fn a_branch_that_never_reaches_the_gate_uncovers_it_too() {
    let steps = canvas_steps();
    let graph = graph_of(
        &steps,
        Some(&[("i", "r"), ("j", "r"), ("r", "g"), ("r", "b")]),
    );
    assert_eq!(covering(&steps, &graph, "g"), None);
}

#[test]
fn a_body_the_turn_found_at_the_review_path_is_not_its_review() {
    assert!(carried_over(Some("PASS"), "PASS"));
    assert!(!carried_over(Some("PASS"), "FAIL"));
    assert!(!carried_over(None, "PASS"));
}

#[test]
fn a_clean_review_holds_nothing_and_every_other_standing_says_why() {
    assert_eq!(ReviewStanding::Clean.held_reason(), None);
    assert_eq!(
        ReviewStanding::Blocking.held_reason(),
        Some("the review lists critical or major issues")
    );
    assert_eq!(
        ReviewStanding::Unreadable("the review has no Verdict section").held_reason(),
        Some("the review has no Verdict section")
    );
}
