// Tests extracted from `src-tauri/src/commands/workflows.rs` (mirrored-tests convention). `super` = that module.

use crate::domain::models::StepConfig;

/// Every embedded starter workflow must deserialize into `StepConfig`
/// (guards the V13 `model`/`verifier` JSON edits against typos).
fn parse(json: &str) -> Vec<StepConfig> {
    let v: serde_json::Value = serde_json::from_str(json).expect("starter JSON parses");
    serde_json::from_value(v["steps"].clone()).expect("steps deserialize into StepConfig")
}

#[test]
fn all_starters_deserialize() {
    for json in [
        include_str!("../../workflows/standard-feature-pipeline.json"),
        include_str!("../../workflows/bugfix-pipeline.json"),
        include_str!("../../workflows/docs-update.json"),
        include_str!("../../workflows/refactor.json"),
        include_str!("../../workflows/experiment.json"),
        include_str!("../../workflows/ci-fix.json"),
        include_str!("../../workflows/simple-task.json"),
    ] {
        let steps = parse(json);
        assert!(!steps.is_empty());
    }
}

/// The six looping workflows must each have a step that both redirects on
/// failure AND carries a verifier (the harness + agent-judgment gate).
#[test]
fn looping_starters_have_verifier_and_redirect() {
    for json in [
        include_str!("../../workflows/standard-feature-pipeline.json"),
        include_str!("../../workflows/bugfix-pipeline.json"),
        include_str!("../../workflows/docs-update.json"),
        include_str!("../../workflows/refactor.json"),
        include_str!("../../workflows/ci-fix.json"),
        include_str!("../../workflows/simple-task.json"),
    ] {
        let steps = parse(json);
        let has_loop = steps
            .iter()
            .any(|s| s.on_failure.is_some() && s.verifier.is_some());
        assert!(
            has_loop,
            "expected a validate step with on_failure + verifier"
        );
    }
}
