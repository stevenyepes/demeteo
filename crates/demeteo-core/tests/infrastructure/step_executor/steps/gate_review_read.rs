// Tests extracted from
// `crates/demeteo-core/src/adapters/step_executor/steps/gate/review_read.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

use std::sync::Arc;

use crate::adapters::artifact_store::fs::FsArtifactStore;
use crate::adapters::step_executor::artifacts::resolve_declared_artifacts;
use crate::domain::artifact::{Artifact, ArtifactCapture};
use crate::domain::ids::{FeatureId, StepExecutionId, WorkflowId};
use crate::domain::models::workflow_migrate::migrate_v1_to_v2;
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

fn read_from(
    store: &dyn ArtifactStore,
    steps: &[StepConfig],
    critic_status: &str,
    gate: &str,
) -> ReviewStanding {
    let def = migrate_v1_to_v2(WorkflowId::from("wf".to_string()), "wf", steps);
    let graph = WorkflowGraph::build(&def).expect("the Standard pipeline is an acyclic graph");
    review_standing(
        store,
        F_ID,
        steps,
        &graph,
        &[execution("s-critic", critic_status)],
        &StepId::from(gate.to_string()),
    )
}

fn read(store: &CriticStore, critic_status: &str, gate: &str) -> ReviewStanding {
    read_from(store, &standard_steps(), critic_status, gate)
}

/// The Standard pipeline with its critic's review declared the way most
/// bundled steps declare a report: an artifact name that is not its file's.
fn steps_with_review_named_apart_from_its_file() -> Vec<StepConfig> {
    let mut steps = standard_steps();
    let critic = steps
        .iter_mut()
        .find(|s| s.id.as_ref() == "s-critic")
        .expect("the Standard pipeline has a critic");
    let decl = critic
        .artifacts
        .iter_mut()
        .flatten()
        .find(|decl| decl.name == "critic-review")
        .expect("the critic declares its review");
    decl.name = "critic".to_string();
    decl.capture = ArtifactCapture::LastWriteTo {
        path: "artifacts/s-critic.md".to_string(),
    };
    critic.review_artifact = Some("critic".to_string());
    steps
}

/// Through the real store and the real capture, so the name the gate looks
/// for is the name `put` gave the file and not one a fixture chose.
#[test]
fn a_review_whose_artifact_name_is_not_its_file_name_is_found_in_the_store() {
    let dir = crate::support::test_dir::scratch("demeteo_test_review_read_store");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(dir.clone()));
    let steps = steps_with_review_named_apart_from_its_file();
    let critic = steps
        .iter()
        .find(|s| s.id.as_ref() == "s-critic")
        .expect("the Standard pipeline has a critic");
    let written = Artifact::tool_write("s-critic", "artifacts/s-critic.md", CLEAN);

    let (stored, missing) = resolve_declared_artifacts(
        critic.artifacts.as_deref().unwrap_or(&[]),
        &[written],
        &store,
        F_ID,
        "s-critic",
    );
    assert_eq!((stored.len(), missing.len()), (1, 0));

    assert_eq!(
        read_from(&*store, &steps, "completed", "s-gate-ship"),
        ReviewStanding::Clean
    );
    let _ = std::fs::remove_dir_all(dir);
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
