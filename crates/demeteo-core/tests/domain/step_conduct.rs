//! How a turn is told to conduct itself. Pure over a capability.

use super::*;

const CAPS: [StepCapability; 4] = [
    StepCapability::Implement,
    StepCapability::Artifacts,
    StepCapability::Verify,
    StepCapability::ReadOnly,
];

#[test]
fn every_capability_is_told_to_finish_and_to_ground_its_claims() {
    for cap in CAPS {
        let out = inject_turn_conduct("do the work", cap);
        assert!(out.contains(CONDUCT_HEADING), "{cap:?}: {out}");
        assert!(out.contains("read your last paragraph"), "{cap:?}: {out}");
        assert!(
            out.contains("against a tool result from this session"),
            "{cap:?}: {out}"
        );
    }
}

#[test]
fn the_block_precedes_the_prompt_and_renders_once() {
    let out = inject_turn_conduct("do the work", StepCapability::Verify);
    assert!(out.ends_with("do the work"), "{out}");
    assert_eq!(out.matches(CONDUCT_HEADING).count(), 1);
    assert!(
        out.find(CONDUCT_HEADING).unwrap() < out.find("do the work").unwrap(),
        "a preamble the model reads after the template is not a preamble"
    );
}

#[test]
fn only_an_implement_turn_is_told_to_hold_its_scope() {
    let implement = inject_turn_conduct("p", StepCapability::Implement);
    assert!(implement.contains("The task sets the scope"), "{implement}");
    assert!(implement.contains("commit tests only where"), "{implement}");

    for cap in [
        StepCapability::Artifacts,
        StepCapability::Verify,
        StepCapability::ReadOnly,
    ] {
        let out = inject_turn_conduct("p", cap);
        assert!(
            !out.contains("The task sets the scope"),
            "{cap:?} cannot touch source; the fence holds its scope: {out}"
        );
    }
}

/// The block is written at normal volume: pressure words make the register
/// cautious and the model reads them literally.
#[test]
fn the_block_carries_no_pressure_language() {
    let out = inject_turn_conduct("", StepCapability::Implement);
    for word in ["MUST", "NEVER", "ALWAYS", "CRITICAL", "IMPORTANT", "DO NOT"] {
        assert!(!out.contains(word), "`{word}` in: {out}");
    }
}
