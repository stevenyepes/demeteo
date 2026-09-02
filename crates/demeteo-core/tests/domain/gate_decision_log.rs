// Tests extracted from `crates/demeteo-core/src/domain/gate_decision_log.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

fn approved<'a>(step_id: &'a str, feedback: Option<&'a str>) -> DecidedGate<'a> {
    DecidedGate {
        step_id,
        decision: "approve",
        feedback,
    }
}

/// A template may reference the token unconditionally, so nothing to say
/// has to render as nothing — not as a heading over an empty list, which
/// reads as "asked and answered: none".
#[test]
fn no_decisions_render_nothing_at_all() {
    assert_eq!(render_gate_decision_log(&[]), "");
}

#[test]
fn a_decision_names_its_node_and_carries_the_humans_words() {
    let out = render_gate_decision_log(&[approved(
        "s-gate-review",
        Some("approved the network-gating line"),
    )]);
    assert!(out.contains("s-gate-review"), "{out}");
    assert!(out.contains("approve"), "{out}");
    assert!(out.contains("approved the network-gating line"), "{out}");
}

/// The whole point of the block: an approval is evidence a validator should
/// spend, not a curiosity. If this sentence goes, the block is decorative.
#[test]
fn the_block_tells_the_reader_an_approval_is_evidence() {
    let out = render_gate_decision_log(&[approved("s-gate-review", Some("ship it"))]);
    assert!(
        out.contains("is** the evidence"),
        "the block must say an approval counts as evidence: {out}"
    );
}

/// Rows are deleted on rewind, so silence is ambiguous. A reader told
/// otherwise would turn a rewound approval into a denial — worse than
/// having no block.
#[test]
fn the_block_warns_that_an_absence_is_not_a_refusal() {
    let out = render_gate_decision_log(&[approved("s-gate-review", None)]);
    assert!(out.contains("live view"), "{out}");
    assert!(out.contains("do not read an absence as a refusal"), "{out}");
}

#[test]
fn a_decision_with_no_comment_says_so_rather_than_trailing_off() {
    let out = render_gate_decision_log(&[approved("s-gate-ship", None)]);
    assert!(out.contains("(no comment)"), "{out}");
}

/// Whitespace-only feedback is the same as none — an agent that wrote a
/// blank comment must not produce `s-gate-ship — approve: `.
#[test]
fn blank_feedback_reads_as_no_comment() {
    let out = render_gate_decision_log(&[approved("s-gate-ship", Some("   "))]);
    assert!(out.contains("(no comment)"), "{out}");
}

/// Oldest first: the order a reader follows a history in, and the reason
/// the port method deliberately reverses `latest_decided_for_feature`.
#[test]
fn decisions_render_in_the_order_they_were_given() {
    let out = render_gate_decision_log(&[
        approved("s-gate-review", Some("first")),
        approved("s-gate-ship", Some("second")),
    ]);
    let review = out.find("s-gate-review").expect("review present");
    let ship = out.find("s-gate-ship").expect("ship present");
    assert!(review < ship, "oldest decision should render first: {out}");
}

/// A cancel is not an approval. Rendering every entry with approving
/// language would let a refusal read as a sign-off.
#[test]
fn a_cancel_renders_as_a_cancel() {
    let out = render_gate_decision_log(&[DecidedGate {
        step_id: "s-gate-ship",
        decision: "cancel",
        feedback: Some("wrong branch"),
    }]);
    assert!(out.contains("cancel"), "{out}");
    assert!(out.contains("wrong branch"), "{out}");
}
