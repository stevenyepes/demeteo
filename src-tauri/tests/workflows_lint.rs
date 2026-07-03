//! Structural lint over the shipped starter workflow templates in
//! `workflows/*.json`. Catches authoring mistakes that don't crash
//! anything at runtime but silently defeat a workflow's intended retry
//! behavior — a typo'd `on_failure` target, a duplicate step id, or a
//! `verify`-capability step whose `on_failure` can never trigger because
//! it has no `verifier` config to translate a failed check into
//! `StepOutcome::Failed`.
//!
//! See `demeteo_lib::domain::models::lint_workflow_steps` for the
//! invariants checked and why each one matters.

use demeteo_lib::domain::models::{lint_workflow_steps, StepConfig};

fn workflows_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("workflows")
}

#[test]
fn every_shipped_workflow_passes_the_structural_lint() {
    let dir = workflows_dir();
    let mut checked = 0;
    let mut failures: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&dir).expect("workflows/ directory must exist") {
        let entry = entry.expect("readable dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_string();

        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", file_name, e));
        let value: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{} is not valid JSON: {}", file_name, e));
        let steps: Vec<StepConfig> = serde_json::from_value(value["steps"].clone())
            .unwrap_or_else(|e| panic!("{} has an invalid `steps` array: {}", file_name, e));

        checked += 1;
        let violations = lint_workflow_steps(&steps);
        if !violations.is_empty() {
            failures.push(format!("{}:\n  - {}", file_name, violations.join("\n  - ")));
        }
    }

    assert!(
        checked > 0,
        "expected to find at least one workflow JSON file in {}",
        dir.display()
    );
    assert!(
        failures.is_empty(),
        "structural lint failures in shipped workflows:\n\n{}",
        failures.join("\n\n")
    );
}
