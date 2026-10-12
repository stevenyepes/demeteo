// Tests extracted from
// `crates/demeteo-core/src/adapters/step_executor/steps/gate/review_read.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

use crate::domain::artifact::Artifact;
use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::workflow_starters::StarterDefinition;

const F_ID: &str = "f-1";
const CLEAN: &str = "## Critical Issues\nNone.\n\n## Major Issues\nNone.\n\n## Minor Issues\n\
None.\n\n## Verdict\nPASS\n\n{\"review\": {\"critical\": 0, \"major\": 0, \"minor\": 0}}\n";
const BLOCKING: &str = "## Critical Issues\n- path traversal\n\n## Major Issues\nNone.\n\n\
## Minor Issues\nNone.\n\n## Verdict\nFAIL\n\n\
{\"review\": {\"critical\": 1, \"major\": 0, \"minor\": 0}}\n";

/// Holds the critic step's references and nothing else. Any other step, any
/// other feature and any write is an error, so a lookup that drifts to the
/// wrong step reads as a failure rather than as an empty answer.
struct CriticStore {
    references: Vec<(&'static str, &'static str)>,
}

impl ArtifactStore for CriticStore {
    fn put(&self, _f: &str, _s: &str, _a: &Artifact) -> Result<String, String> {
        Err("CriticStore: unexpected put".to_string())
    }
    fn get(&self, reference: &str) -> Result<String, String> {
        self.references
            .iter()
            .find(|(r, _)| *r == reference)
            .map(|(_, body)| body.to_string())
            .ok_or_else(|| format!("CriticStore: unexpected get `{reference}`"))
    }
    fn list_for_step(&self, feature_id: &str, step_id: &str) -> Result<Vec<String>, String> {
        if feature_id != F_ID || step_id != "s-critic" {
            return Err(format!(
                "CriticStore: unexpected list_for_step `{feature_id}` `{step_id}`"
            ));
        }
        Ok(self.references.iter().map(|(r, _)| r.to_string()).collect())
    }
    fn clear_step(&self, _f: &str, _s: &str) -> Result<(), String> {
        Err("CriticStore: unexpected clear_step".to_string())
    }
}

fn standard_steps() -> Vec<StepConfig> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/workflows/standard-feature-pipeline.json");
    let json = std::fs::read_to_string(path).expect("the bundled Standard pipeline is readable");
    StarterDefinition::parse(&json)
        .expect("the bundled Standard pipeline is JSON")
        .steps
}

fn execution(step_id: &str, status: &str) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from(format!("se-{step_id}")),
        feature_id: FeatureId::from(F_ID.to_string()),
        step_id: StepId::from(step_id.to_string()),
        step_index: 0,
        step_kind: "agent".to_string(),
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

fn read(store: &CriticStore, critic_status: &str, gate: &str) -> ReviewStanding {
    review_standing(
        store,
        F_ID,
        &standard_steps(),
        &[execution("s-critic", critic_status)],
        &StepId::from(gate.to_string()),
    )
}

#[test]
fn the_ship_gate_reads_the_critics_stored_review() {
    let store = CriticStore {
        references: vec![("/store/s-critic/critic-review.md", CLEAN)],
    };
    assert_eq!(
        read(&store, "completed", "s-gate-ship"),
        ReviewStanding::Clean
    );
}

#[test]
fn a_blocking_review_is_read_as_blocking() {
    let store = CriticStore {
        references: vec![("/store/s-critic/critic-review.md", BLOCKING)],
    };
    assert_eq!(
        read(&store, "completed", "s-gate-ship"),
        ReviewStanding::Blocking
    );
}

/// The critic's other artifacts sit in the same listing; only the one the
/// step names as its review is the review.
#[test]
fn an_artifact_of_another_name_is_not_the_review() {
    let store = CriticStore {
        references: vec![("/store/s-critic/notes.md", CLEAN)],
    };
    assert!(matches!(
        read(&store, "completed", "s-gate-ship"),
        ReviewStanding::Unreadable(_)
    ));
}

#[test]
fn a_rework_cycles_later_review_is_the_one_read() {
    let store = CriticStore {
        references: vec![
            ("/store/s-critic/1/critic-review.md", CLEAN),
            ("/store/s-critic/2/critic-review.md", BLOCKING),
        ],
    };
    assert_eq!(
        read(&store, "completed", "s-gate-ship"),
        ReviewStanding::Blocking
    );
}

/// A stored review from an earlier cycle outlives the critic being reset.
#[test]
fn a_review_step_that_has_not_completed_is_not_read() {
    let store = CriticStore {
        references: vec![("/store/s-critic/critic-review.md", CLEAN)],
    };
    for status in ["pending", "running", "failed"] {
        assert!(matches!(
            read(&store, status, "s-gate-ship"),
            ReviewStanding::Unreadable(_)
        ));
    }
}

#[test]
fn a_gate_no_review_step_covers_reads_nothing() {
    let store = CriticStore {
        references: vec![("/store/s-critic/critic-review.md", CLEAN)],
    };
    let steps = standard_steps();
    let other_gate = steps
        .iter()
        .find(|s| s.kind == "gate" && s.id.as_ref() != "s-gate-ship")
        .expect("the Standard pipeline has an earlier gate");
    assert!(matches!(
        read(&store, "completed", other_gate.id.as_ref()),
        ReviewStanding::Unreadable(_)
    ));
}
