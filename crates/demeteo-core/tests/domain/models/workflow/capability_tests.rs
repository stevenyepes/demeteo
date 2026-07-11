// Tests extracted from `src/domain/models/workflow.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::*;
use crate::domain::artifact::ArtifactMode;

fn step(kind: &str, capability: Option<StepCapability>) -> StepConfig {
    StepConfig {
        id: StepId::from("s-x"),
        kind: kind.into(),
        title: "x".into(),
        agent_kind: None,
        model: None,
        prompt_template: None,
        on_failure: None,
        max_iterations: None,
        artifacts: None,
        verifier: None,
        capability,
        allow_network: false,
        allow_shell: false,
        gate_class: None,
    }
}

#[test]
fn explicit_capability_wins() {
    let s = step("agent", Some(StepCapability::ReadOnly));
    assert_eq!(s.effective_capability(), StepCapability::ReadOnly);
}

#[test]
fn undeclared_agent_step_defaults_to_artifacts() {
    let s = step("agent", None);
    assert_eq!(s.effective_capability(), StepCapability::Artifacts);
}

#[test]
fn parallel_step_infers_implement() {
    let s = step("parallel", None);
    assert_eq!(s.effective_capability(), StepCapability::Implement);
}

#[test]
fn unconstrained_capture_infers_implement() {
    let mut s = step("agent", None);
    s.artifacts = Some(vec![ArtifactDecl {
        name: "all".into(),
        capture: ArtifactCapture::AllWrites,
        mode: ArtifactMode::Full,
        inline: false,
    }]);
    assert_eq!(s.effective_capability(), StepCapability::Implement);
}

#[test]
fn explicit_capability_overrides_inference() {
    // A parallel step explicitly downgraded stays downgraded.
    let s = step("parallel", Some(StepCapability::Verify));
    assert_eq!(s.effective_capability(), StepCapability::Verify);
}
