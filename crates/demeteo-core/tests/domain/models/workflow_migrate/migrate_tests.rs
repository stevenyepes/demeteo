//! v1 → v2 migration coverage (task P1.2).
//!
//! The seven bundled starters are the fixture set — the same files the
//! P0.2 baseline harness executes, loaded from `src-tauri/workflows/`.

use super::*;
use crate::domain::ids::StepId;
use crate::domain::models::workflow_v2::{JoinSemantics, RetryStrategy};
use std::path::Path;

const STARTERS: [&str; 7] = [
    "bugfix-pipeline",
    "ci-fix",
    "docs-update",
    "experiment",
    "refactor",
    "simple-task",
    "standard-feature-pipeline",
];

fn load_starter(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/workflows")
        .join(format!("{name}.json"));
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read starter {}: {e}", path.display()));
    serde_json::from_str(&body).expect("starter parses")
}

fn steps_of(doc: &serde_json::Value) -> Vec<StepConfig> {
    serde_json::from_value(doc["steps"].clone()).expect("starter steps parse as StepConfig")
}

#[test]
fn all_seven_starters_migrate_without_error() {
    for name in STARTERS {
        let doc = load_starter(name);
        let def = migrate_definition(&doc)
            .unwrap_or_else(|e| panic!("starter '{name}' failed to migrate: {e}"));
        assert_eq!(def.schema_version, WORKFLOW_SCHEMA_V2, "{name}");
        assert_eq!(def.id.as_str(), doc["id"].as_str().unwrap(), "{name}");
        assert_eq!(def.name, doc["name"].as_str().unwrap(), "{name}");

        // Serialized output re-parses as v2 — the storage round-trip.
        let json = serde_json::to_value(&def).unwrap();
        let back: WorkflowDefinitionV2 = serde_json::from_value(json).unwrap();
        assert_eq!(back, def, "{name}");
    }
}

#[test]
fn node_count_and_chain_edges_match_list_order() {
    for name in STARTERS {
        let doc = load_starter(name);
        let steps = steps_of(&doc);
        let def = migrate_v1_to_v2(doc["id"].as_str().unwrap().into(), name, &steps);

        assert_eq!(def.nodes.len(), steps.len(), "{name}: node per step");
        for (node, step) in def.nodes.iter().zip(&steps) {
            assert_eq!(node.id, step.id, "{name}: order preserved");
            assert_eq!(node.title, step.title, "{name}");
            assert!(node.node_type != "parallel", "{name}: alias resolved");
        }

        // The first steps.len()-1 edges are exactly the consecutive chain,
        // in order; anything after that is a task_list edge.
        let chain = &def.edges[..steps.len() - 1];
        for (edge, pair) in chain.iter().zip(steps.windows(2)) {
            assert_eq!(edge.from, pair[0].id, "{name}");
            assert_eq!(edge.to, pair[1].id, "{name}");
            assert!(edge.when.is_none(), "{name}: chains are unconditional");
        }
    }
}

#[test]
fn on_failure_becomes_verdict_redirect_retry() {
    let doc = load_starter("bugfix-pipeline");
    let steps = steps_of(&doc);
    let def = migrate_v1_to_v2("wf-bugfix".into(), "Bugfix", &steps);

    // s-fix: on_failure = s-gate-confirm, max_iterations = 2.
    let fix = def.nodes.iter().find(|n| n.id.as_str() == "s-fix").unwrap();
    let rule = fix.retry.as_ref().unwrap().verdict.as_ref().unwrap();
    assert_eq!(rule.strategy, RetryStrategy::Redirect);
    assert_eq!(
        rule.redirect_to.as_ref().unwrap().as_str(),
        "s-gate-confirm"
    );
    assert_eq!(rule.max_attempts, Some(2));
    assert!(rule.feedback, "RetryContext append behavior is preserved");
    // v1 routed plain agent failures through the same on_failure path —
    // the migrated policy must cover `agent_failure` identically, or the
    // v2 engine would silently stop redirecting them (P1.10 amendment).
    assert_eq!(
        fix.retry.as_ref().unwrap().agent_failure.as_ref(),
        Some(rule),
        "agent_failure mirrors the verdict redirect rule"
    );
    // Consumed into the policy — not duplicated in the opaque payload.
    assert!(fix.config.get("max_iterations").is_none());
    assert!(fix.config.get("on_failure").is_none());

    // s-verify: on_failure = s-plan-fix, no max_iterations → attempts resolve
    // through the runtime precedence chain, so the policy carries None.
    //
    // The target is the step that *produces* `s-fix`'s task list, not `s-fix`
    // itself (decision 43): a rework cycle re-scopes the defect into fresh
    // tickets against the branch the previous cycle already landed, rather
    // than re-running an execution step whose list still describes work that
    // is committed.
    let verify = def
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "s-verify")
        .unwrap();
    let rule = verify.retry.as_ref().unwrap().verdict.as_ref().unwrap();
    assert_eq!(rule.redirect_to.as_ref().unwrap().as_str(), "s-plan-fix");
    assert_eq!(rule.max_attempts, None);

    // A step without on_failure gets no retry policy; its inert
    // max_iterations survives as author intent in config.
    let reproduce = def
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "s-reproduce")
        .unwrap();
    assert!(reproduce.retry.is_none());
    assert_eq!(reproduce.config["max_iterations"], 2);
}

#[test]
fn task_list_from_becomes_an_edge_into_the_sequence_node() {
    // standard: s-implement (sequence) takes its task list from s-tickets,
    // which is not its chain predecessor (s-gate-review is).
    let doc = load_starter("standard-feature-pipeline");
    let steps = steps_of(&doc);
    let def = migrate_v1_to_v2("wf-std".into(), "Standard", &steps);

    let implement = def
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "s-implement")
        .unwrap();
    assert_eq!(implement.node_type, "sequence");
    assert!(
        implement.config.get("task_list_from").is_none(),
        "the magic field is lifted into graph structure"
    );

    let incoming: Vec<&str> = def
        .edges
        .iter()
        .filter(|e| e.to.as_str() == "s-implement")
        .map(|e| e.from.as_str())
        .collect();
    assert_eq!(
        incoming,
        vec!["s-gate-review", "s-tickets"],
        "chain edge plus the task_list dependency"
    );

    // refactor: same shape (s-analyse feeds s-refactor past s-gate-plan).
    let doc = load_starter("refactor");
    let steps = steps_of(&doc);
    let def = migrate_v1_to_v2("wf-ref".into(), "Refactor", &steps);
    let incoming: Vec<&str> = def
        .edges
        .iter()
        .filter(|e| e.to.as_str() == "s-refactor")
        .map(|e| e.from.as_str())
        .collect();
    assert_eq!(incoming, vec!["s-gate-plan", "s-analyse"]);
}

#[test]
fn task_list_from_pointing_at_the_chain_predecessor_adds_no_duplicate_edge() {
    let steps: Vec<StepConfig> = serde_json::from_value(serde_json::json!([
        { "id": "plan", "kind": "agent", "title": "Plan", "agent_kind": null,
          "prompt_template": null, "on_failure": null, "max_iterations": null,
          "artifacts": [ { "name": "task-list",
            "capture": { "kind": "last_write_to", "path": "artifacts/task-list.json" },
            "mode": "full" } ] },
        { "id": "impl", "kind": "sequence", "title": "Implement", "agent_kind": null,
          "prompt_template": null, "on_failure": null, "max_iterations": null,
          "task_list_from": "plan" }
    ]))
    .unwrap();
    let def = migrate_v1_to_v2("wf-x".into(), "X", &steps);
    assert_eq!(def.edges.len(), 1, "no duplicate plan→impl edge");
    assert_eq!(def.edges[0].from.as_str(), "plan");
    assert_eq!(def.edges[0].to.as_str(), "impl");
}

#[test]
fn parallel_alias_resolves_to_sequence() {
    let steps: Vec<StepConfig> = serde_json::from_value(serde_json::json!([
        { "id": "fan", "kind": "parallel", "title": "Old Fan-Out", "agent_kind": null,
          "prompt_template": null, "on_failure": null, "max_iterations": null }
    ]))
    .unwrap();
    let def = migrate_v1_to_v2("wf-p".into(), "P", &steps);
    assert_eq!(def.nodes[0].node_type, "sequence");
}

#[test]
fn migration_is_idempotent_on_v2_input() {
    for name in STARTERS {
        let doc = load_starter(name);
        let migrated = migrate_definition(&doc).unwrap();
        let v2_doc = serde_json::to_value(&migrated).unwrap();
        let again = migrate_definition(&v2_doc)
            .unwrap_or_else(|e| panic!("{name}: v2 pass-through failed: {e}"));
        assert_eq!(again, migrated, "{name}: pass-through, not re-migration");
    }
}

#[test]
fn positions_form_a_stable_vertical_column() {
    let doc = load_starter("simple-task");
    let steps = steps_of(&doc);
    let def = migrate_v1_to_v2("wf-s".into(), "S", &steps);
    let ys: Vec<f64> = def
        .nodes
        .iter()
        .map(|n| n.position.expect("position synthesized").y)
        .collect();
    let mut sorted = ys.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(ys, sorted, "top-to-bottom in list order");
    assert!(ys.windows(2).all(|w| w[1] > w[0]), "strictly increasing ys");
    assert!(def.nodes.iter().all(|n| n.position.unwrap().x == 0.0));
}

#[test]
fn lifted_fields_never_leak_into_config() {
    for name in STARTERS {
        let doc = load_starter(name);
        let def = migrate_definition(&doc).unwrap();
        for node in &def.nodes {
            for field in ["id", "kind", "title", "on_failure", "task_list_from"] {
                assert!(
                    node.config.get(field).is_none(),
                    "{name}/{}: '{field}' must not appear in config",
                    node.id
                );
            }
            // Payload fields the handlers own must survive untouched.
            if node.node_type == "agent" {
                assert!(
                    node.config.get("prompt_template").is_some(),
                    "{name}/{}: agent config keeps its prompt",
                    node.id
                );
            }
        }
    }
}

#[test]
fn migrated_defaults_are_empty_and_join_falls_back_to_engine_default() {
    // Decision 39: the engine-wide default is all_success; migration
    // doesn't need to write it into every document.
    let doc = load_starter("ci-fix");
    let def = migrate_definition(&doc).unwrap();
    assert!(def.defaults.is_empty());
    assert!(def.nodes.iter().all(|n| n.join.is_none()));
    // (Sanity: the enum default the scheduler will apply exists.)
    let _ = JoinSemantics::AllSuccess;
}

#[test]
fn migrate_definition_rejects_garbage_with_a_readable_error() {
    let err = migrate_definition(&serde_json::json!({ "nope": true })).unwrap_err();
    assert!(err.contains("not a v1 workflow definition"), "{err}");

    let err =
        migrate_definition(&serde_json::json!({ "schema_version": 2, "id": "x" })).unwrap_err();
    assert!(err.contains("invalid schema-v2"), "{err}");
}

#[test]
fn a_v1_file_needs_only_its_steps() {
    // PRD §10 promises hand-written v1 files keep importing. Requiring `id`
    // and `name` broke that for the commonest hand-written shape, and bought
    // nothing: `save_definition` overwrites both from the workflow row it
    // mints.
    let def = migrate_definition(&serde_json::json!({
        "name": "Hand written",
        "steps": [
            { "id": "s1", "kind": "agent", "title": "Do it",
              "prompt_template": "go" }
        ]
    }))
    .expect("a file with no `id` still imports");
    assert_eq!(def.nodes.len(), 1);
    assert_eq!(def.name, "Hand written");

    // Not even a name is required — the import path defaults it.
    let def = migrate_definition(&serde_json::json!({
        "steps": [{ "id": "s1", "kind": "agent", "title": "T", "prompt_template": "go" }]
    }))
    .expect("a file with only `steps` still imports");
    assert_eq!(def.nodes.len(), 1);

    // `steps` remains the one thing that makes a document a v1 workflow.
    let err = migrate_definition(&serde_json::json!({ "id": "x", "name": "n" })).unwrap_err();
    assert!(err.contains("steps"), "{err}");
}

#[test]
fn dangling_task_list_from_is_tolerated_not_fatal() {
    // Total function: a broken reference is a lint finding (P1.4), not a
    // migration crash. The edge is simply not synthesized.
    let steps: Vec<StepConfig> = serde_json::from_value(serde_json::json!([
        { "id": "impl", "kind": "sequence", "title": "Implement", "agent_kind": null,
          "prompt_template": null, "on_failure": null, "max_iterations": null,
          "task_list_from": "ghost" }
    ]))
    .unwrap();
    let def = migrate_v1_to_v2("wf-d".into(), "D", &steps);
    assert!(def.edges.is_empty());
    let _ = StepId::from("ghost");
}

/// The frontend canvas (task P2.1) renders migrated v2 definitions, and its
/// fixture-driven render test consumes committed JSON at
/// `src/components/canvas/__fixtures__/<starter>.v2.json`. Emitting those from
/// the *live* migration here — rather than hand-authoring them — guarantees the
/// canvas renders exactly what the engine produces and can never silently drift
/// from the migration. Off by default (asserts the committed fixtures are
/// current); regenerate with `UPDATE_CANVAS_FIXTURES=1 cargo test -p
/// demeteo-core canvas_fixtures_are_current`.
#[test]
fn canvas_fixtures_are_current() {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/components/canvas/__fixtures__");
    let update = std::env::var("UPDATE_CANVAS_FIXTURES").is_ok();
    if update {
        std::fs::create_dir_all(&dir).expect("create fixtures dir");
    }
    for name in STARTERS {
        let doc = load_starter(name);
        let def = migrate_definition(&doc)
            .unwrap_or_else(|e| panic!("starter '{name}' failed to migrate: {e}"));
        let json = serde_json::to_string_pretty(&def).expect("serialize v2") + "\n";
        let path = dir.join(format!("{name}.v2.json"));
        if update {
            std::fs::write(&path, &json)
                .unwrap_or_else(|e| panic!("write fixture {}: {e}", path.display()));
        } else {
            let existing = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "canvas fixture {} missing: {e}; run UPDATE_CANVAS_FIXTURES=1",
                    path.display()
                )
            });
            assert_eq!(
                existing, json,
                "canvas fixture '{name}' is stale; run UPDATE_CANVAS_FIXTURES=1"
            );
        }
    }
}

// ── v2 → v1 projection (task P3.6) ───────────────────────────────────────────

/// The load-bearing property of two-representation storage: for a chain — which
/// every starter is — projecting the migrated graph back must reproduce the
/// author's step list *exactly*. If it didn't, saving a workflow through the
/// builder would silently rewrite the definition the runner executes.
#[test]
fn every_starter_round_trips_through_the_v2_projection() {
    for name in STARTERS {
        let doc = load_starter(name);
        let original = steps_of(&doc);
        let def = migrate_definition(&doc).expect("migrates");
        let projected = project_v2_to_v1(&def);
        assert_eq!(
            projected, original,
            "starter '{name}' did not survive the v1 → v2 → v1 round trip"
        );
    }
}

/// Order is the graph's, not the definition array's: a builder that appends a
/// node writes it at the end of `nodes` regardless of where it was wired in,
/// so the projection has to sort or the v1 list would claim the wrong sequence.
#[test]
fn projection_orders_by_dependency_not_by_node_array_order() {
    let def: WorkflowDefinitionV2 = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "wf-order",
        "name": "Order",
        // Deliberately authored back-to-front.
        "nodes": [
            { "id": "c", "type": "finalize", "title": "Ship", "config": {} },
            { "id": "b", "type": "agent", "title": "Build", "config": {} },
            { "id": "a", "type": "agent", "title": "Plan", "config": {} }
        ],
        "edges": [ { "from": "a", "to": "b" }, { "from": "b", "to": "c" } ]
    }))
    .unwrap();

    let ids: Vec<String> = project_v2_to_v1(&def).into_iter().map(|s| s.id.0).collect();
    assert_eq!(ids, vec!["a", "b", "c"]);
}

/// A diamond has no single correct v1 order, but it must have a *valid* one:
/// every node appears exactly once, after all of its dependencies.
#[test]
fn a_branching_graph_projects_to_a_valid_linear_order() {
    let def: WorkflowDefinitionV2 = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "wf-diamond",
        "name": "Diamond",
        "nodes": [
            { "id": "plan", "type": "agent", "title": "Plan", "config": {} },
            { "id": "left", "type": "agent", "title": "Left", "config": {} },
            { "id": "right", "type": "agent", "title": "Right", "config": {} },
            { "id": "ship", "type": "finalize", "title": "Ship", "config": {} }
        ],
        "edges": [
            { "from": "plan", "to": "left" },
            { "from": "plan", "to": "right" },
            { "from": "left", "to": "ship" },
            { "from": "right", "to": "ship" }
        ]
    }))
    .unwrap();

    let ids: Vec<String> = project_v2_to_v1(&def).into_iter().map(|s| s.id.0).collect();
    assert_eq!(ids.len(), 4, "every node projects exactly once");
    let at = |id: &str| ids.iter().position(|s| s == id).expect("present");
    assert!(at("plan") < at("left") && at("plan") < at("right"));
    assert!(at("left") < at("ship") && at("right") < at("ship"));
}

/// What v1 cannot hold is dropped from the projection — and only from the
/// projection. This is the whole reason `definition_json` is stored beside it.
#[test]
fn the_projection_drops_exactly_what_v1_cannot_express() {
    let def: WorkflowDefinitionV2 = serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "wf-lossy",
        "name": "Lossy",
        "nodes": [
            {
                "id": "a", "type": "agent", "title": "A", "config": { "prompt_template": "go" },
                "position": { "x": 120.0, "y": 40.0 }
            },
            {
                "id": "b", "type": "agent", "title": "B", "config": {},
                "join": "any_success",
                "retry": { "environment": { "strategy": "in_place", "max_attempts": 2 } }
            }
        ],
        "edges": [ { "from": "a", "to": "b", "when": "${{ nodes.a.outputs.verdict != 'FAIL' }}" } ]
    }))
    .unwrap();

    let steps = project_v2_to_v1(&def);
    assert_eq!(steps.len(), 2);
    // The prompt (v1-expressible) survives…
    assert_eq!(steps[0].prompt_template.as_deref(), Some("go"));
    // …while the in-place environment rule leaves no v1 trace: only a
    // *redirect* maps onto `on_failure`.
    assert_eq!(steps[1].on_failure, None);
    // And the stored document keeps every one of them for the builder.
    assert_eq!(def.nodes[0].position.map(|p| p.x), Some(120.0));
    assert!(def.edges[0].when.is_some());
}

/// `WorkflowVersion::definition` is the one seam every graph reader uses.
#[test]
fn a_version_prefers_its_stored_document_and_falls_back_to_migration() {
    use crate::domain::ids::{WorkflowId, WorkflowVersionId};
    use crate::domain::models::WorkflowVersion;

    let doc = load_starter("simple-task");
    let steps = steps_of(&doc);
    let migrated = migrate_definition(&doc).expect("migrates");

    let row = |definition_json: Option<String>| WorkflowVersion {
        id: WorkflowVersionId::from("wf-x-v1"),
        workflow_id: WorkflowId::from("wf-x"),
        version: 1,
        steps_json: serde_json::to_string(&steps).unwrap(),
        definition_json,
        note: None,
        created_at: 0,
    };

    // No stored document → migrate the step list (every pre-P3.6 row).
    let fallback = row(None).definition("Simple Task");
    assert_eq!(fallback.nodes.len(), migrated.nodes.len());

    // Stored document wins, layout and all.
    let mut authored = migrated.clone();
    authored.nodes[0].position =
        Some(crate::domain::models::workflow_v2::Position { x: 999.0, y: 111.0 });
    let stored = row(Some(serde_json::to_string(&authored).unwrap())).definition("Simple Task");
    assert_eq!(stored.nodes[0].position.map(|p| p.x), Some(999.0));

    // An unreadable document degrades to the migration rather than failing:
    // `steps_json` is always valid, so there is a good answer available.
    let broken = row(Some("{not json".to_string())).definition("Simple Task");
    assert_eq!(broken.nodes.len(), migrated.nodes.len());
}

// ── import (task P3.6) ───────────────────────────────────────────────────

/// A v1 file keeps importing, and its top-level `description` — which the v2
/// schema has no place for — survives beside the graph.
#[test]
fn a_v1_file_imports_with_its_description() {
    let mut doc = load_starter("simple-task");
    doc["description"] = serde_json::json!("the shipped summary");

    let imported = read_import(&doc).expect("a v1 starter imports");
    assert_eq!(imported.definition.schema_version, 2);
    assert_eq!(imported.description, "the shipped summary");
    assert_eq!(imported.name, doc["name"].as_str().expect("named"));
}

/// The round trip `workflow_export` writes: a v2 document with `description`
/// added alongside comes back as the same graph.
#[test]
fn a_v2_document_imports_unchanged() {
    let migrated = migrate_definition(&load_starter("simple-task")).expect("migrates");
    let mut doc = serde_json::to_value(&migrated).expect("serializes");
    doc["description"] = serde_json::json!("exported earlier");

    let imported = read_import(&doc).expect("a v2 document imports");
    assert_eq!(imported.definition.nodes.len(), migrated.nodes.len());
    assert_eq!(imported.description, "exported earlier");
}

/// What export writes, import reads — graph *and* the description the graph
/// itself has no place for.
#[test]
fn what_export_writes_import_reads_back() {
    let migrated =
        migrate_definition(&load_starter("standard-feature-pipeline")).expect("migrates");
    let file = write_export(&migrated, "the workflow's own summary").expect("exports");

    let doc: serde_json::Value = serde_json::from_str(&file).expect("export is JSON");
    let imported = read_import(&doc).expect("its own export imports");

    assert_eq!(imported.definition, migrated, "the graph survives verbatim");
    assert_eq!(imported.description, "the workflow's own summary");
}

/// A nameless workflow would be unfindable in the library, so import names it.
#[test]
fn a_blank_name_falls_back_to_a_placeholder() {
    let mut doc = load_starter("simple-task");
    doc["name"] = serde_json::json!("   ");

    assert_eq!(
        read_import(&doc).expect("imports").name,
        "Imported Workflow"
    );
}

/// A hand-edited v2 file is judged by the published schema *before* serde
/// sees it, so the refusal locates the violation instead of naming a field.
#[test]
fn a_schema_invalid_v2_document_is_refused_by_the_schema() {
    let migrated = migrate_definition(&load_starter("simple-task")).expect("migrates");
    let mut doc = serde_json::to_value(&migrated).expect("serializes");
    doc["nodes"][0]["id"] = serde_json::json!(42);

    let err = read_import(&doc).expect_err("a numeric node id is not schema-valid");
    assert!(
        err.contains("schema-v2 workflow failed validation"),
        "{err}"
    );
}

/// Neither shape: the error says what was missing rather than passing an
/// unusable definition on to the write path.
#[test]
fn a_document_of_neither_shape_is_refused() {
    let err = read_import(&serde_json::json!({ "name": "no steps here" }))
        .expect_err("neither v1 nor v2");
    assert!(err.contains("not a v1 workflow definition"), "{err}");
}
