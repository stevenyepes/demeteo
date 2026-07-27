// Tests extracted from `src-tauri/src/commands/workflows.rs` (mirrored-tests convention). `super` = that module.

use crate::domain::models::StepConfig;

use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::{WorkflowId, WorkflowVersionId};
use crate::domain::models::{Workflow, WorkflowVersion};
use crate::ports::db::WorkflowRepository;
use rusqlite::Connection;
use std::sync::Arc;

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

// ── Version history: restore + per-version graph (P3.4) ──────────────────
//
// These drive `super::restore_version` / `super::version_graph` — the cores the
// `#[tauri::command]` wrappers delegate to — against a real SQLite repository,
// so the immutability claim ("restore creates a new version row") is proven
// against the storage that actually holds it rather than a stand-in.

/// An in-memory workflow repository with migrations applied.
fn repo() -> Arc<dyn WorkflowRepository> {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    Arc::new(SqliteAdapter::new(conn).expect("run migrations")) as Arc<dyn WorkflowRepository>
}

/// A minimal but *real* v1 step list — `restore` copies this string verbatim,
/// so the tests assert on the exact bytes.
fn steps_json(step_ids: &[&str]) -> String {
    let steps: Vec<serde_json::Value> = step_ids
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "kind": "agent",
                "title": format!("Step {id}"),
                "agent_kind": null,
                "prompt_template": format!("do {id}"),
                "on_failure": null,
                "max_iterations": null,
            })
        })
        .collect();
    let json = serde_json::to_string(&steps).expect("serialize steps");
    serde_json::from_str::<Vec<StepConfig>>(&json).expect("fixture deserializes into StepConfig");
    json
}

/// A workflow with one version per entry in `versions`, numbered from 1.
fn seed(workflows: &Arc<dyn WorkflowRepository>, id: &str, versions: &[String]) -> WorkflowId {
    let wf_id = WorkflowId::from(id.to_string());
    workflows
        .create(Workflow {
            id: wf_id.clone(),
            name: "History Test".to_string(),
            description: "seeded".to_string(),
            is_starter: false,
            created_at: 1_000,
            updated_at: 1_000,
            schedule: None,
        })
        .expect("create workflow");
    for (i, steps) in versions.iter().enumerate() {
        let n = i as u32 + 1;
        workflows
            .save_version(WorkflowVersion {
                id: WorkflowVersionId::from(format!("{id}-v{n}")),
                workflow_id: wf_id.clone(),
                version: n,
                steps_json: steps.clone(),
                note: Some(format!("v{n}")),
                created_at: 1_000 + i64::from(n),
            })
            .expect("save version");
    }
    wf_id
}

#[test]
fn next_version_number_is_one_past_the_highest() {
    assert_eq!(super::next_version_number(&[]), 1);

    let rows: Vec<WorkflowVersion> = [1u32, 2, 7]
        .iter()
        .map(|n| WorkflowVersion {
            id: WorkflowVersionId::from(format!("wf-v{n}")),
            workflow_id: WorkflowId::from("wf".to_string()),
            version: *n,
            steps_json: "[]".to_string(),
            note: None,
            created_at: 0,
        })
        .collect();
    // A gap must not hand out a number that was already used.
    assert_eq!(super::next_version_number(&rows), 8);
}

/// The Done-when: restoring appends, and the row it copied is untouched.
#[test]
fn restore_appends_a_verbatim_copy_and_leaves_history_intact() {
    let workflows = repo();
    let v1_steps = steps_json(&["plan"]);
    let v2_steps = steps_json(&["plan", "implement"]);
    let wf_id = seed(&workflows, "wf-hist", &[v1_steps.clone(), v2_steps.clone()]);

    let restored = super::restore_version(
        &workflows,
        &wf_id,
        &WorkflowVersionId::from("wf-hist-v1".to_string()),
    )
    .expect("restore v1");

    assert_eq!(restored.version, 3, "restore mints the next version");
    assert_eq!(restored.version_id, "wf-hist-v3");
    assert_eq!(restored.steps.len(), 1);

    let rows = workflows.versions(&wf_id).expect("list versions");
    assert_eq!(rows.len(), 3, "history grew, nothing was replaced");
    assert_eq!(rows[0].steps_json, v1_steps, "v1 is untouched");
    assert_eq!(rows[1].steps_json, v2_steps, "v2 is untouched");
    assert_eq!(
        rows[2].steps_json, v1_steps,
        "the new version is a byte-exact copy of the restored one"
    );
    assert_eq!(rows[2].note.as_deref(), Some("Restored from v1"));
}

/// Name/description aren't versioned, so a content restore must not rewrite
/// them — the workflow keeps the name it had.
#[test]
fn restore_leaves_workflow_metadata_alone() {
    let workflows = repo();
    let wf_id = seed(&workflows, "wf-meta", &[steps_json(&["plan"])]);
    workflows
        .update_meta(&wf_id, "Renamed Later", "new description")
        .expect("rename");

    let restored = super::restore_version(
        &workflows,
        &wf_id,
        &WorkflowVersionId::from("wf-meta-v1".to_string()),
    )
    .expect("restore v1");

    assert_eq!(restored.name, "Renamed Later");
    assert_eq!(restored.description, "new description");
}

/// Version ids are guessable (`<workflow-id>-v3`), so the pairing is checked.
#[test]
fn restore_refuses_a_version_from_another_workflow() {
    let workflows = repo();
    let mine = seed(&workflows, "wf-mine", &[steps_json(&["plan"])]);
    seed(&workflows, "wf-theirs", &[steps_json(&["secret"])]);

    let err = match super::restore_version(
        &workflows,
        &mine,
        &WorkflowVersionId::from("wf-theirs-v1".to_string()),
    ) {
        Ok(_) => panic!("a cross-workflow restore must be refused"),
        Err(e) => e,
    };
    assert!(
        format!("{err:?}").contains("wf-theirs"),
        "error names the mismatch: {err:?}"
    );
    assert_eq!(
        workflows.versions(&mine).expect("list").len(),
        1,
        "and writes nothing"
    );
}

/// The drawer diffs a *named* version, not the latest one.
#[test]
fn version_graph_projects_the_named_version() {
    let workflows = repo();
    let wf_id = seed(
        &workflows,
        "wf-graph",
        &[steps_json(&["plan"]), steps_json(&["plan", "implement"])],
    );

    let older = super::version_graph(
        &workflows,
        &wf_id,
        &WorkflowVersionId::from("wf-graph-v1".to_string()),
    )
    .expect("graph for v1");
    let newer = super::version_graph(
        &workflows,
        &wf_id,
        &WorkflowVersionId::from("wf-graph-v2".to_string()),
    )
    .expect("graph for v2");

    assert_eq!(older.schema_version, 2);
    assert_eq!(
        older
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["plan"]
    );
    assert!(older.edges.is_empty());
    assert_eq!(
        newer
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["plan", "implement"]
    );
    assert_eq!(newer.edges.len(), 1, "list order became a chain edge");
}
