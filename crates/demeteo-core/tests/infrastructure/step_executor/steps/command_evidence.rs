// The evidence half of a command node: what it reads back off the worktree,
// and what it does when the store refuses it. `super` = `steps::command`.
//
// Both rules under test are policy that only ever existed inside a 785-line
// `async fn`: a shell emits no tool-call events, so an event-shaped capture is
// *skipped* rather than reported missing; and a store failure degrades to a
// `tracing::warn!` rather than turning a green command red.

use std::sync::Mutex;

use super::*;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::artifact::{ArtifactCapture, ArtifactDecl, ArtifactMode};
use crate::domain::ids::{StepExecutionId, StepId};

const F_ID: &str = "f-cmd";
const WT: &str = "/wt/f-cmd-step-s-build";

/// An [`ArtifactStore`] that records every `put` and answers nothing else.
/// Every method it was not built to answer errors, so a path reaching it is a
/// failure rather than a default.
struct RecordingStore {
    put_fails: bool,
    puts: Mutex<Vec<(String, String, String)>>,
}

impl RecordingStore {
    fn new(put_fails: bool) -> Self {
        Self {
            put_fails,
            puts: Mutex::new(Vec::new()),
        }
    }

    fn names(&self) -> Vec<String> {
        self.puts
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.2.clone())
            .collect()
    }
}

impl ArtifactStore for RecordingStore {
    fn put(&self, feature_id: &str, step_id: &str, artifact: &Artifact) -> Result<String, String> {
        self.puts.lock().unwrap().push((
            feature_id.to_string(),
            step_id.to_string(),
            artifact.name.clone(),
        ));
        if self.put_fails {
            return Err("disk full".to_string());
        }
        Ok(format!("ref://{feature_id}/{step_id}/{}", artifact.name))
    }
    fn get(&self, reference: &str) -> Result<String, String> {
        Err(format!("RecordingStore: unexpected get `{reference}`"))
    }
    fn list_for_step(&self, _f: &str, _s: &str) -> Result<Vec<String>, String> {
        Err("RecordingStore: unexpected list_for_step".to_string())
    }
    fn clear_step(&self, _f: &str, _s: &str) -> Result<(), String> {
        Err("RecordingStore: unexpected clear_step".to_string())
    }
}

fn step_exec() -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-cmd"),
        feature_id: FeatureId::from(F_ID),
        step_id: StepId::from("s-build"),
        step_index: 0,
        step_kind: "command".to_string(),
        status: "running".to_string(),
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
        created_at: 1_700_000_000,
        updated_at: 1_700_000_000,
    }
}

fn step_conf(decls: Vec<ArtifactDecl>) -> StepConfig {
    let mut conf: StepConfig = serde_json::from_value(serde_json::json!({
        "id": "s-build",
        "kind": "command",
        "title": "Build",
        "command": "make",
    }))
    .expect("step parses");
    conf.artifacts = Some(decls);
    conf
}

fn declared(name: &str, path: &str) -> ArtifactDecl {
    ArtifactDecl::full_path(name, path)
}

fn by_name(name: &str) -> ArtifactDecl {
    ArtifactDecl {
        name: name.to_string(),
        capture: ArtifactCapture::ByName {
            name: name.to_string(),
        },
        mode: ArtifactMode::Full,
        inline: false,
    }
}

#[tokio::test]
async fn a_declared_file_that_reads_back_is_stored_and_referenced() {
    let exec = ScriptedExec::new(&[])
        .with_files(&[("/wt/f-cmd-step-s-build/docs/report.md", Ok("# it built\n"))]);
    let store = RecordingStore::new(false);
    let f_id = FeatureId::from(F_ID);
    let evidence = CommandEvidence {
        exec: &exec,
        artifacts: &store,
        f_id: &f_id,
    };

    let (refs, missing) = collect_declared_files(
        &evidence,
        &step_exec(),
        &step_conf(vec![declared("report", "docs/report.md")]),
        "local",
        WT,
    )
    .await;

    assert_eq!(refs, vec!["ref://f-cmd/s-build/report".to_string()]);
    assert!(missing.is_empty(), "{missing:?}");
    assert_eq!(store.names(), vec!["report".to_string()]);
}

#[tokio::test]
async fn a_declared_file_that_is_not_there_is_missing_not_stored() {
    // The port errors on any path it was not told about, which is exactly the
    // "the command exited 0 but never wrote it" case.
    let exec = ScriptedExec::new(&[]);
    let store = RecordingStore::new(false);
    let f_id = FeatureId::from(F_ID);
    let evidence = CommandEvidence {
        exec: &exec,
        artifacts: &store,
        f_id: &f_id,
    };

    let (refs, missing) = collect_declared_files(
        &evidence,
        &step_exec(),
        &step_conf(vec![declared("report", "docs/report.md")]),
        "local",
        WT,
    )
    .await;

    assert!(refs.is_empty());
    assert_eq!(
        missing,
        vec!["declared artifact 'report' at docs/report.md".to_string()]
    );
    assert!(store.names().is_empty(), "nothing to store");
}

#[tokio::test]
async fn an_event_shaped_capture_is_skipped_rather_than_reported_missing() {
    // A shell emits no tool-call events, so `by_name` can never match here.
    // Reporting it missing would fail every command node that declares one.
    let exec = ScriptedExec::new(&[]);
    let store = RecordingStore::new(false);
    let f_id = FeatureId::from(F_ID);
    let evidence = CommandEvidence {
        exec: &exec,
        artifacts: &store,
        f_id: &f_id,
    };

    let (refs, missing) = collect_declared_files(
        &evidence,
        &step_exec(),
        &step_conf(vec![by_name("summary")]),
        "local",
        WT,
    )
    .await;

    assert!(refs.is_empty());
    assert!(missing.is_empty(), "{missing:?}");
    assert!(
        store.names().is_empty(),
        "no read was attempted, so nothing was stored"
    );
}

#[tokio::test]
async fn a_store_failure_yields_no_reference_and_does_not_fail_the_command() {
    let exec = ScriptedExec::new(&[])
        .with_files(&[("/wt/f-cmd-step-s-build/docs/report.md", Ok("# it built\n"))]);
    let store = RecordingStore::new(true);
    let f_id = FeatureId::from(F_ID);
    let evidence = CommandEvidence {
        exec: &exec,
        artifacts: &store,
        f_id: &f_id,
    };

    let (refs, missing) = collect_declared_files(
        &evidence,
        &step_exec(),
        &step_conf(vec![declared("report", "docs/report.md")]),
        "local",
        WT,
    )
    .await;

    assert!(refs.is_empty(), "a refused store contributes no reference");
    assert!(
        missing.is_empty(),
        "the file was produced; only its filing failed"
    );
    assert_eq!(store.names(), vec!["report".to_string()], "it was offered");
}

#[test]
fn the_store_is_keyed_by_the_feature_and_step_the_artifact_belongs_to() {
    let exec = ScriptedExec::new(&[]);
    let store = RecordingStore::new(false);
    let f_id = FeatureId::from(F_ID);
    let evidence = CommandEvidence {
        exec: &exec,
        artifacts: &store,
        f_id: &f_id,
    };

    let stored = store_command_artifact(
        &evidence,
        &step_exec(),
        Artifact {
            name: "command-output".to_string(),
            mime: "text/plain".to_string(),
            content: "out".to_string(),
            source: ArtifactSource::AgentText,
        },
    );

    assert_eq!(
        stored.as_deref(),
        Some("ref://f-cmd/s-build/command-output")
    );
    assert_eq!(
        store
            .puts
            .lock()
            .unwrap()
            .first()
            .map(|p| (p.0.clone(), p.1.clone())),
        Some((F_ID.to_string(), "s-build".to_string()))
    );
}
