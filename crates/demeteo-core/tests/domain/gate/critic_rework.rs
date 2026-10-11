//! The rework policy against the Standard pipeline as it ships, read from the
//! bundled file: if that file drifts from the shape `critic_step` expects, the
//! tool refuses every run, and nothing else would say so.

use super::*;
use crate::domain::ids::{FeatureId, StepExecutionId, StepId};
use crate::domain::workflow_starters::StarterDefinition;

fn standard_steps() -> Vec<StepConfig> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/workflows/standard-feature-pipeline.json");
    let json = std::fs::read_to_string(path).expect("the bundled Standard pipeline is readable");
    let starter = StarterDefinition::parse(&json).expect("the bundled Standard pipeline is JSON");
    assert_eq!(starter.id.as_ref(), STANDARD_WORKFLOW_ID);
    starter.steps
}

fn execution(steps: &[StepConfig], step_id: &str, status: &str) -> StepExecution {
    let index = steps
        .iter()
        .position(|s| s.id.as_ref() == step_id)
        .expect("the step is in the workflow");
    StepExecution {
        id: StepExecutionId::from(format!("se-{step_id}")),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: StepId::from(step_id.to_string()),
        step_index: index as u32,
        step_kind: steps[index].kind.clone(),
        status: status.to_string(),
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
        created_at: 0,
        updated_at: 0,
    }
}

const FAILING_REVIEW: &str = "## Critical Issues\n- the delete path in s-implement skips the \
audit log\n\n## Major Issues\nNone.\n\n## Minor Issues\nNone.\n\n## Verdict\n**FAIL** — one \
blocking issue.\n";

fn request<'a>(
    steps: &'a [StepConfig],
    gate: &'a StepExecution,
    review: &'a str,
) -> ReworkRequest<'a> {
    ReworkRequest {
        workflow_id: Some(STANDARD_WORKFLOW_ID),
        steps,
        gate,
        review,
        prior_reworks: 0,
    }
}

#[test]
fn the_shipped_standard_pipeline_is_sent_back_with_the_critics_review() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-ship", "awaiting_gate");

    let plan = plan(&request(&steps, &gate, FAILING_REVIEW)).expect("a FAIL verdict is reworked");

    assert_eq!(plan.verdict, CriticVerdict::Fail);
    assert!(plan.feedback.starts_with(PREAMBLE));
    assert!(plan.feedback.contains("skips the audit log"));
}

/// The review above names `s-implement`. Were the composed feedback short
/// enough to be read as an address, the redirect would land wherever the
/// critic happened to mention.
#[test]
fn a_step_id_quoted_by_the_critic_does_not_address_the_redirect() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-ship", "awaiting_gate");
    let feedback = plan(&request(&steps, &gate, "## Verdict\nFAIL s-research"))
        .expect("reworked")
        .feedback;

    let target = resolve_redirect_target(&steps, None, gate.step_index, Some(&feedback))
        .expect("a redirect from the ship gate has a target");

    assert_eq!(steps[target].id.as_ref(), "s-tickets");
}

#[test]
fn the_preamble_names_no_step_of_the_standard_pipeline() {
    for step in standard_steps() {
        assert!(
            !PREAMBLE.contains(step.id.as_ref()),
            "the preamble names {}",
            step.id.as_ref()
        );
    }
}

#[test]
fn a_passing_review_is_not_sent_back() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-ship", "awaiting_gate");

    let refused = plan(&request(&steps, &gate, "## Verdict\nPASS"));

    assert_eq!(refused, Err(ReworkRefusal::VerdictPass));
}

#[test]
fn pass_with_notes_is_sent_back() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-ship", "awaiting_gate");

    let plan = plan(&request(&steps, &gate, "## Verdict\n`PASS_WITH_NOTES`")).expect("reworked");

    assert_eq!(plan.verdict, CriticVerdict::PassWithNotes);
}

#[test]
fn a_review_without_a_verdict_is_refused() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-ship", "awaiting_gate");

    for review in [
        "",
        "## Critical Issues\nFAIL to compile",
        "## Verdict\nunsure",
    ] {
        assert_eq!(
            plan(&request(&steps, &gate, review)),
            Err(ReworkRefusal::NoVerdict),
            "review: {review:?}"
        );
    }
}

/// A critic that restates its verdict writes two headings; the last is the
/// one it settled on.
#[test]
fn the_last_verdict_heading_wins() {
    assert_eq!(
        parse_verdict("## Verdict\nFAIL\n\n## Verdict (revised)\nPASS\n"),
        Some(CriticVerdict::Pass)
    );
}

#[test]
fn the_third_rework_is_refused() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-ship", "awaiting_gate");
    let mut req = request(&steps, &gate, FAILING_REVIEW);

    req.prior_reworks = MAX_CLIENT_REWORKS - 1;
    assert!(plan(&req).is_ok());
    req.prior_reworks = MAX_CLIENT_REWORKS;
    assert_eq!(
        plan(&req),
        Err(ReworkRefusal::CapReached(MAX_CLIENT_REWORKS))
    );
}

#[test]
fn another_workflow_is_refused_even_with_the_same_steps() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-ship", "awaiting_gate");
    let mut req = request(&steps, &gate, FAILING_REVIEW);

    req.workflow_id = Some("wf-custom");
    assert_eq!(plan(&req), Err(ReworkRefusal::NotStandardWorkflow));
    req.workflow_id = None;
    assert_eq!(plan(&req), Err(ReworkRefusal::NotStandardWorkflow));
}

#[test]
fn the_review_gate_is_not_the_final_gate() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-review", "awaiting_gate");

    assert!(matches!(
        plan(&request(&steps, &gate, FAILING_REVIEW)),
        Err(ReworkRefusal::NotAwaitingFinalGate { .. })
    ));
}

#[test]
fn a_final_gate_that_is_not_parked_is_refused() {
    let steps = standard_steps();

    for status in ["pending", "completed", "failed"] {
        let gate = execution(&steps, "s-gate-ship", status);
        assert!(
            matches!(
                plan(&request(&steps, &gate, FAILING_REVIEW)),
                Err(ReworkRefusal::NotAwaitingFinalGate { .. })
            ),
            "status: {status}"
        );
    }
}

/// A verifier park on the validate step holds a pending decision row too, and
/// is what `list_pending_gates` would hand a client for it.
#[test]
fn a_park_on_a_step_that_is_not_a_gate_is_refused() {
    let steps = standard_steps();
    let park = execution(&steps, "s-validate", "awaiting_gate");

    assert!(matches!(
        plan(&request(&steps, &park, FAILING_REVIEW)),
        Err(ReworkRefusal::NotAwaitingFinalGate { .. })
    ));
}

#[test]
fn an_edited_standard_pipeline_is_refused_by_its_shape() {
    let shipped = standard_steps();
    let gate = execution(&shipped, "s-gate-ship", "awaiting_gate");
    let position = |id: &str| shipped.iter().position(|s| s.id.as_ref() == id).unwrap();

    let mut no_critic = shipped.clone();
    no_critic.remove(position("s-critic"));

    let mut step_after_finalize = shipped.clone();
    let mut extra = shipped[position("s-research")].clone();
    extra.id = StepId::from("s-extra".to_string());
    step_after_finalize.push(extra);

    let mut no_rework_template = shipped.clone();
    no_rework_template[position("s-tickets")].rework_prompt_template = None;

    for (name, steps) in [
        ("critic removed", no_critic),
        ("a step after finalize", step_after_finalize),
        ("rework template removed", no_rework_template),
    ] {
        assert!(
            matches!(
                plan(&request(&steps, &gate, FAILING_REVIEW)),
                Err(ReworkRefusal::WorkflowShape(_))
            ),
            "{name}"
        );
    }
}

#[test]
fn a_long_review_keeps_its_tail_and_stays_on_a_char_boundary() {
    let steps = standard_steps();
    let gate = execution(&steps, "s-gate-ship", "awaiting_gate");
    let review = format!("HEAD-MARKER {}\n## Verdict\nFAIL", "é".repeat(20_000));

    let feedback = plan(&request(&steps, &gate, &review))
        .expect("reworked")
        .feedback;

    assert!(!feedback.contains("HEAD-MARKER"));
    assert!(feedback.ends_with("## Verdict\nFAIL"));
    assert!(feedback.len() < PREAMBLE.len() + 17 * 1024);
}

/// A section the critic adds after its verdict is not part of the verdict,
/// whatever words it uses.
#[test]
fn a_heading_after_the_verdict_closes_it() {
    assert_eq!(
        parse_verdict("## Verdict\nPASS\n\n## Appendix\nTests that FAIL on main\n"),
        Some(CriticVerdict::Pass)
    );
    assert_eq!(parse_verdict("## Verdict\n\n## Appendix\nFAIL\n"), None);
}
