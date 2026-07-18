// Regression tests for gate redirect handling.
// Extracted from `steps/gate.rs` (kept out of the source file per the
// crate's mirrored-tests convention); `super` resolves to that module.

use super::*;
use crate::domain::ids::StepId;

fn step(id: &str) -> StepConfig {
    StepConfig {
        effort: None,
        id: StepId::from(id.to_string()),
        kind: "agent".to_string(),
        title: id.to_string(),
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
    }
}

#[test]
fn explicit_step_id_in_feedback_wins() {
    let steps = vec![
        step("research"),
        step("spec"),
        step("gate"),
        step("implement"),
    ];
    let target = resolve_redirect_target(&steps, None, 2, Some("implement"));
    assert_eq!(target, Some(3));
}

#[test]
fn step_id_named_within_free_text_feedback_wins() {
    // A reviewer writing natural-language feedback that names a step
    // ("redo s-tickets, the split is too coarse") should land on that
    // step even though the feedback isn't *only* the bare id — matching
    // the standard pipeline's own gate prompt, which tells reviewers to
    // "Redirect to 's-tickets' if the decomposition needs revision".
    let steps = vec![
        step("s-research"),
        step("s-tickets"),
        step("s-spec"),
        step("s-gate-review"),
    ];
    let target = resolve_redirect_target(
        &steps,
        None,
        3,
        Some("redo s-tickets, the split is too coarse"),
    );
    assert_eq!(
        target,
        Some(1),
        "should route to the named step, not fall back to s-spec"
    );
}

#[test]
fn step_id_substring_within_a_word_does_not_match() {
    // "s-tickets2" (or any id that merely contains a step id as a
    // substring) must not be mistaken for "s-tickets" — matching is by
    // whole word/token, not substring. With no exact token match, this
    // falls through to the predecessor fallback (s-spec), not s-tickets.
    let steps = vec![
        step("s-research"),
        step("s-tickets"),
        step("s-spec"),
        step("s-gate-review"),
    ];
    let target = resolve_redirect_target(&steps, None, 3, Some("see s-tickets2 for context"));
    assert_eq!(
        target,
        Some(2),
        "no exact token match, so this falls back to the immediate predecessor (s-spec), not s-tickets"
    );
}

#[test]
fn free_text_feedback_falls_back_to_previous_step() {
    // The user's bug: typing implementation feedback used to
    // silently cancel the pipeline. The fallback should land on
    // the step immediately before the gate.
    let steps = vec![
        step("research"),
        step("spec"),
        step("gate"),
        step("implement"),
    ];
    let target = resolve_redirect_target(
        &steps,
        None,
        2, // gate is at index 2
        Some("make sure to use cargo before mise"),
    );
    assert_eq!(target, Some(1), "should fall back to the spec step");
}

#[test]
fn on_failure_takes_priority_over_previous_step() {
    let steps = vec![
        step("research"),
        step("spec"),
        step("gate"),
        step("implement"),
    ];
    let target = resolve_redirect_target(
        &steps,
        Some(&StepId::from("research".to_string())),
        2,
        Some("random feedback"),
    );
    assert_eq!(target, Some(0));
}

#[test]
fn gate_at_step_zero_cancels() {
    let steps = vec![step("gate")];
    let target = resolve_redirect_target(&steps, None, 0, Some("feedback"));
    assert_eq!(target, None);
}

#[test]
fn empty_feedback_with_no_on_failure_falls_back() {
    let steps = vec![step("research"), step("gate")];
    let target = resolve_redirect_target(&steps, None, 1, Some("   "));
    assert_eq!(target, Some(0));
}

// ── implement-step fallback (Bug 2 regression suite) ────────────────
//
// The previous-step fallback in `resolve_redirect_target` only
// walked one hop back. In the standard pipeline that meant
// free-text implementation feedback at `s-gate-ship` (index 6)
// landed on `s-validate` (index 5), which is verify-only and
// cannot modify source — the feedback just got logged into
// `validation-report.md` and bounced back via the verifier two
// iterations later. These tests pin the fix: when no explicit
// step id is in the feedback and `on_failure` is unset, walk
// back through every preceding step to the nearest
// implement-capable one.

fn step_with_cap(id: &str, capability: crate::domain::permission::StepCapability) -> StepConfig {
    let mut s = step(id);
    s.capability = Some(capability);
    s
}

#[test]
fn standard_pipeline_ship_gate_feedback_lands_on_implement() {
    // Mirror `workflows/standard-feature-pipeline.json` exactly:
    // s-gate-ship at index 7, s-critic (artifacts) at index 6,
    // s-validate (verify) at index 5, s-implement (implement)
    // at index 4. Free-text implementation feedback must land on
    // index 4, not on index 5 or 6.
    let steps = vec![
        step("s-research"),
        step("s-tickets"),
        step("s-spec"),
        step("s-gate-review"),
        step_with_cap(
            "s-implement",
            crate::domain::permission::StepCapability::Implement,
        ),
        step_with_cap(
            "s-validate",
            crate::domain::permission::StepCapability::Verify,
        ),
        step_with_cap(
            "s-critic",
            crate::domain::permission::StepCapability::Artifacts,
        ),
        step("s-gate-ship"),
    ];
    let target = resolve_redirect_target(
        &steps,
        None, // s-gate-ship.on_failure is null in the workflow
        7,
        Some("the implementation is missing the cancel-button handler — add it"),
    );
    assert_eq!(
        target,
        Some(4),
        "must walk past s-critic and s-validate to s-implement"
    );
}

#[test]
fn standard_pipeline_review_gate_feedback_routes_to_spec() {
    // In the standard pipeline, `s-gate-review` sits at index 3,
    // BEFORE `s-implement` (index 4). The walk-back from index 3
    // only sees indices 0–2 (research + tickets + spec) — none is
    // implement-capable — so the implement-fallback returns
    // None. The predecessor fallback then routes to `s-spec`,
    // which is the right semantic: pre-implementation review
    // feedback typically means "revise the spec". Users who
    // want to forward-route can type the explicit step id
    // "s-implement" (covered by
    // `explicit_implement_step_id_still_wins_over_implement_fallback`
    // below), and decomposition feedback names "s-tickets".
    let steps = vec![
        step("s-research"),
        step("s-tickets"),
        step("s-spec"),
        step("s-gate-review"),
        step_with_cap(
            "s-implement",
            crate::domain::permission::StepCapability::Implement,
        ),
    ];
    let target = resolve_redirect_target(
        &steps,
        None,
        3,
        Some("revise the spec to use cargo before mise"),
    );
    assert_eq!(target, Some(2));
}

#[test]
fn walks_back_past_multiple_verify_steps_to_implement() {
    // A workflow with a long verify/review chain between the
    // gate and the implement step. The walk must traverse all
    // of them and land on the implement step.
    let steps = vec![
        step_with_cap(
            "s-implement",
            crate::domain::permission::StepCapability::Implement,
        ),
        step_with_cap(
            "s-review",
            crate::domain::permission::StepCapability::Artifacts,
        ),
        step_with_cap(
            "s-validate",
            crate::domain::permission::StepCapability::Verify,
        ),
        step_with_cap(
            "s-security",
            crate::domain::permission::StepCapability::Verify,
        ),
        step("s-gate-ship"),
    ];
    let target = resolve_redirect_target(
        &steps,
        None,
        4,
        Some("fix the SQL injection in the user lookup"),
    );
    assert_eq!(target, Some(0));
}

#[test]
fn falls_back_to_predecessor_when_no_implement_step_before_gate() {
    // A pre-implementation review gate with no implement step
    // preceding it. The implement-fallback walk finds nothing;
    // the predecessor fallback returns the immediately preceding
    // step so the pipeline never silently cancels.
    let steps = vec![
        step("s-research"),
        step("s-spec"),
        step("s-gate-pre-impl"),
        step_with_cap(
            "s-implement",
            crate::domain::permission::StepCapability::Implement,
        ),
    ];
    let target = resolve_redirect_target(&steps, None, 2, Some("loosen the spec a bit"));
    assert_eq!(target, Some(1));
}

#[test]
fn implement_fallback_does_not_traverse_through_the_gate_itself() {
    // The gate itself is at `gate_step_index`. The walk must
    // only look at `steps[..gate_idx]`, never at the gate row
    // or anything past it. A buggy implementation that scanned
    // the whole `steps` slice could land on an implement step
    // *after* the gate and send the run forward instead of
    // backward.
    let steps = vec![
        step_with_cap(
            "s-implement",
            crate::domain::permission::StepCapability::Implement,
        ),
        step("s-spec"),
        step("s-gate-review"),
        step_with_cap(
            "s-implement-later",
            crate::domain::permission::StepCapability::Implement,
        ),
    ];
    let target = resolve_redirect_target(&steps, None, 2, Some("the spec needs revision"));
    assert_eq!(
        target,
        Some(0),
        "must not jump over the gate to a later implement step"
    );
}

#[test]
fn explicit_implement_step_id_still_wins_over_implement_fallback() {
    // If the user types the id of an implement step that ISN'T
    // the nearest one (e.g. there's an implement at index 0 and
    // another at index 5), the explicit match must beat the
    // walk-back fallback.
    let steps = vec![
        step_with_cap(
            "s-implement",
            crate::domain::permission::StepCapability::Implement,
        ),
        step("s-spec"),
        step_with_cap(
            "s-validate",
            crate::domain::permission::StepCapability::Verify,
        ),
        step("s-gate-review"),
        step("s-spec-2"),
        step_with_cap(
            "s-implement-later",
            crate::domain::permission::StepCapability::Implement,
        ),
        step("s-gate-ship"),
    ];
    let target = resolve_redirect_target(&steps, None, 6, Some("s-implement-later"));
    assert_eq!(target, Some(5));
}
