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
                definition_json: None,
                note: Some(format!("v{n}")),
                created_at: 1_000 + i64::from(n),
            })
            .expect("save version");
    }
    wf_id
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

// ── v2 persistence (task P3.6) ───────────────────────────────────────────
//
// The prerequisite P3.3 flagged: the builder produces a schema-v2 graph, and
// four things it holds — node positions, join semantics, per-class retry, and
// edge guards — have no v1 representation. These drive `super::save_definition`
// (the `workflow_save` command's core) against a real SQLite repository, so
// "nothing the author drew is lost on save" is proven against the storage that
// actually holds it.

use crate::domain::models::workflow_v2::WorkflowDefinitionV2;

/// A graph that uses every construct v1 cannot express.
fn v2_graph(id: &str) -> WorkflowDefinitionV2 {
    serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": id,
        "name": "Authored",
        "nodes": [
            {
                "id": "plan", "type": "agent", "title": "Plan",
                "config": { "prompt_template": "plan it" },
                "position": { "x": 12.5, "y": 0.0 }
            },
            {
                "id": "scan", "type": "agent", "title": "Security scan",
                "config": { "prompt_template": "scan it" },
                "position": { "x": 320.0, "y": 160.0 }
            },
            {
                "id": "build", "type": "agent", "title": "Build",
                "config": { "prompt_template": "build it" },
                "position": { "x": 0.0, "y": 160.0 },
                "retry": {
                    "environment": { "strategy": "in_place", "max_attempts": 2, "backoff_secs": 30 }
                }
            },
            {
                "id": "ship", "type": "finalize", "title": "Ship",
                "config": {},
                "position": { "x": 160.0, "y": 320.0 },
                "join": "all_done"
            }
        ],
        "edges": [
            { "from": "plan", "to": "build" },
            { "from": "plan", "to": "scan" },
            { "from": "build", "to": "ship" },
            { "from": "scan", "to": "ship", "when": "${{ nodes.scan.outputs.verdict != 'FAIL' }}" }
        ]
    }))
    .expect("fixture is a valid v2 definition")
}

/// The Done-when of the persistence prerequisite: everything the author drew
/// comes back, byte for byte.
#[test]
fn saving_a_v2_graph_preserves_what_v1_cannot_hold() {
    let workflows = repo();
    let saved = super::save_definition(
        &workflows,
        None,
        "Authored",
        "built in the canvas",
        v2_graph("placeholder"),
        None,
    )
    .expect("save");

    let wf_id = WorkflowId::from(saved.id.clone());
    let version = workflows
        .latest_version(&wf_id)
        .expect("read")
        .expect("a version exists");
    let reloaded = version.definition("Authored");

    let node = |id: &str| reloaded.nodes.iter().find(|n| n.id.0 == id).expect(id);
    assert_eq!(node("scan").position.map(|p| p.x), Some(320.0), "layout");
    assert_eq!(
        node("ship").join,
        Some(crate::domain::models::workflow_v2::JoinSemantics::AllDone)
    );
    assert_eq!(
        node("build")
            .retry
            .as_ref()
            .and_then(|r| r.environment.as_ref())
            .map(|r| r.max_attempts),
        Some(Some(2)),
        "per-class retry"
    );
    assert!(
        reloaded.edges.iter().any(|e| e.when.is_some()),
        "edge guard survived"
    );
    assert_eq!(reloaded.edges.len(), 4, "the fan-out/fan-in shape survived");
}

/// The compatibility half: the v1 projection is written too, so the runner,
/// replay, and every pre-P3.6 reader still see a runnable step list.
#[test]
fn saving_also_writes_a_runnable_v1_projection() {
    let workflows = repo();
    let saved = super::save_definition(
        &workflows,
        None,
        "Authored",
        "",
        v2_graph("placeholder"),
        None,
    )
    .expect("save");

    let version = workflows
        .latest_version(&WorkflowId::from(saved.id))
        .expect("read")
        .expect("a version exists");
    let steps: Vec<StepConfig> =
        serde_json::from_str(&version.steps_json).expect("projection parses as v1");

    let ids: Vec<&str> = steps.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids.len(), 4);
    let at = |id: &str| ids.iter().position(|s| *s == id).expect("present");
    assert!(at("plan") < at("build") && at("build") < at("ship"));
    assert!(version.definition_json.is_some(), "and the v2 document too");
}

/// The workflow's own identity wins over whatever the definition claims —
/// otherwise a graph copied from another workflow (or a template's placeholder
/// id) would travel into storage and the version would describe the wrong one.
#[test]
fn the_stored_definition_is_normalized_to_the_workflow_it_belongs_to() {
    let workflows = repo();
    let saved = super::save_definition(
        &workflows,
        None,
        "Renamed",
        "",
        v2_graph("wf-some-other-workflow"),
        None,
    )
    .expect("save");

    let version = workflows
        .latest_version(&WorkflowId::from(saved.id.clone()))
        .expect("read")
        .expect("exists");
    let reloaded = version.definition("Renamed");
    assert_eq!(reloaded.id.0, saved.id);
    assert_eq!(reloaded.name, "Renamed");
}

/// Editing appends; it never rewrites. Same guarantee the restore path has.
#[test]
fn saving_an_existing_workflow_appends_a_version() {
    let workflows = repo();
    let first = super::save_definition(&workflows, None, "W", "", v2_graph("x"), None).expect("v1");
    assert_eq!(first.version, 1);

    let mut edited = v2_graph("x");
    edited.nodes.retain(|n| n.id.0 != "scan");
    edited
        .edges
        .retain(|e| e.from.0 != "scan" && e.to.0 != "scan");
    let second = super::save_definition(
        &workflows,
        Some(WorkflowId::from(first.id.clone())),
        "W",
        "",
        edited,
        Some("dropped the scan branch".to_string()),
    )
    .expect("v2");

    assert_eq!(second.version, 2);
    let rows = workflows
        .versions(&WorkflowId::from(first.id))
        .expect("list");
    assert_eq!(rows.len(), 2, "history grew");
    assert_eq!(
        rows[0].definition_json.as_ref().map(|d| d.contains("scan")),
        Some(true),
        "v1 still holds the branch it was saved with"
    );
}

/// P3.3's guarantee, at the write path rather than the Save button: a graph
/// with an error-severity finding cannot be stored at all.
#[test]
fn a_structurally_invalid_graph_is_refused_by_the_write_path() {
    let workflows = repo();
    let mut broken = v2_graph("x");
    // Two finalize sinks — the `multiple-finalize` lint error.
    broken.nodes.push(
        serde_json::from_value(serde_json::json!({
            "id": "ship2", "type": "finalize", "title": "Ship again", "config": {}
        }))
        .unwrap(),
    );
    broken.edges.push(
        serde_json::from_value(serde_json::json!({ "from": "build", "to": "ship2" })).unwrap(),
    );

    let err = match super::save_definition(&workflows, None, "W", "", broken, None) {
        Err(e) => e,
        Ok(_) => panic!("a graph with a structural error must not be storable"),
    };
    assert!(
        format!("{err:?}").contains("structural errors"),
        "unexpected error: {err:?}"
    );
}
