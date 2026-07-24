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

    // s-fix: on_failure = s-reproduce, max_iterations = 3.
    let fix = def.nodes.iter().find(|n| n.id.as_str() == "s-fix").unwrap();
    let rule = fix.retry.as_ref().unwrap().verdict.as_ref().unwrap();
    assert_eq!(rule.strategy, RetryStrategy::Redirect);
    assert_eq!(rule.redirect_to.as_ref().unwrap().as_str(), "s-reproduce");
    assert_eq!(rule.max_attempts, Some(3));
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

    // s-verify: on_failure = s-fix, no max_iterations → attempts resolve
    // through the runtime precedence chain, so the policy carries None.
    let verify = def
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "s-verify")
        .unwrap();
    let rule = verify.retry.as_ref().unwrap().verdict.as_ref().unwrap();
    assert_eq!(rule.redirect_to.as_ref().unwrap().as_str(), "s-fix");
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
