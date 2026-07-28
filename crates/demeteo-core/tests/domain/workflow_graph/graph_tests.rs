//! Table-driven coverage for `WorkflowGraph` + `lint_workflow_v2`
//! (task P1.4): every rule firing and passing, plus the migrated
//! starters linting clean.

use super::*;
use crate::domain::models::workflow_migrate::migrate_definition;
use crate::domain::models::workflow_v2::WorkflowDefinitionV2;

/// Build a definition from compact JSON. Nodes get sane defaults so
/// cases only state what they test.
fn def(nodes: serde_json::Value, edges: serde_json::Value) -> WorkflowDefinitionV2 {
    let mut nodes = nodes;
    for n in nodes.as_array_mut().unwrap() {
        let obj = n.as_object_mut().unwrap();
        obj.entry("type").or_insert("agent".into());
        obj.entry("title").or_insert("T".into());
        if obj["type"] == "agent" {
            obj.entry("config")
                .or_insert(serde_json::json!({ "prompt_template": "do the thing" }));
        }
    }
    serde_json::from_value(serde_json::json!({
        "schema_version": 2,
        "id": "wf-test",
        "name": "Test",
        "nodes": nodes,
        "edges": edges
    }))
    .unwrap()
}

fn codes(findings: &[LintFinding]) -> Vec<&'static str> {
    findings.iter().map(|f| f.code).collect()
}

fn errors(findings: &[LintFinding]) -> Vec<&LintFinding> {
    findings
        .iter()
        .filter(|f| f.severity == LintSeverity::Error)
        .collect()
}

// ---------- graph construction ----------

#[test]
fn chain_builds_with_definition_order_topology() {
    let d = def(
        serde_json::json!([{ "id": "a" }, { "id": "b" }, { "id": "c" }]),
        serde_json::json!([{ "from": "a", "to": "b" }, { "from": "b", "to": "c" }]),
    );
    let g = WorkflowGraph::build(&d).unwrap();
    assert_eq!(g.len(), 3);
    let order: Vec<&str> = g.topological_order().iter().map(|s| s.as_str()).collect();
    assert_eq!(order, vec!["a", "b", "c"]);
    assert_eq!(g.successors(&"a".into()).unwrap(), vec![&StepId::from("b")]);
    assert_eq!(
        g.predecessors(&"c".into()).unwrap(),
        vec![&StepId::from("b")]
    );
}

#[test]
fn diamond_ancestors_and_descendants() {
    //    a
    //   / \
    //  b   c
    //   \ /
    //    d
    let d = def(
        serde_json::json!([{ "id": "a" }, { "id": "b" }, { "id": "c" }, { "id": "d" }]),
        serde_json::json!([
            { "from": "a", "to": "b" }, { "from": "a", "to": "c" },
            { "from": "b", "to": "d" }, { "from": "c", "to": "d" }
        ]),
    );
    let g = WorkflowGraph::build(&d).unwrap();

    let anc: Vec<&str> = {
        let mut v: Vec<&str> = g
            .ancestors(&"d".into())
            .unwrap()
            .iter()
            .map(|s| s.as_str())
            .collect();
        v.sort();
        v
    };
    assert_eq!(anc, vec!["a", "b", "c"]);

    let desc: Vec<&str> = {
        let mut v: Vec<&str> = g
            .descendants(&"a".into())
            .unwrap()
            .iter()
            .map(|s| s.as_str())
            .collect();
        v.sort();
        v
    };
    assert_eq!(desc, vec!["b", "c", "d"]);

    assert!(g.is_ancestor(&"a".into(), &"d".into()));
    assert!(!g.is_ancestor(&"b".into(), &"c".into()), "siblings");
    assert!(!g.is_ancestor(&"d".into(), &"d".into()), "strict: not self");
    let order = g.topological_order();
    assert_eq!(order[0].as_str(), "a");
    assert_eq!(order[3].as_str(), "d");
}

#[test]
fn duplicate_node_id_rejected_at_construction() {
    let d = def(
        serde_json::json!([{ "id": "a" }, { "id": "a" }]),
        serde_json::json!([]),
    );
    let findings = WorkflowGraph::build(&d).unwrap_err();
    assert_eq!(codes(&findings), vec!["duplicate-node-id"]);
    assert_eq!(findings[0].node.as_ref().unwrap().as_str(), "a");
}

#[test]
fn edge_to_unknown_node_rejected_at_construction() {
    let d = def(
        serde_json::json!([{ "id": "a" }]),
        serde_json::json!([{ "from": "a", "to": "ghost" }]),
    );
    let findings = WorkflowGraph::build(&d).unwrap_err();
    assert_eq!(codes(&findings), vec!["edge-unknown-node"]);
    assert!(findings[0].message.contains("ghost"));
}

#[test]
fn cycles_rejected_and_every_cyclic_node_named() {
    let d = def(
        serde_json::json!([{ "id": "a" }, { "id": "b" }, { "id": "c" }]),
        serde_json::json!([
            { "from": "a", "to": "b" }, { "from": "b", "to": "c" }, { "from": "c", "to": "b" }
        ]),
    );
    let findings = WorkflowGraph::build(&d).unwrap_err();
    assert_eq!(codes(&findings), vec!["cycle", "cycle"]);
    let named: Vec<&str> = findings
        .iter()
        .map(|f| f.node.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(named, vec!["b", "c"], "'a' is not on the cycle");
}

#[test]
fn self_edge_is_a_cycle() {
    let d = def(
        serde_json::json!([{ "id": "a" }]),
        serde_json::json!([{ "from": "a", "to": "a" }]),
    );
    let findings = WorkflowGraph::build(&d).unwrap_err();
    assert_eq!(codes(&findings), vec!["cycle"]);
}

// ---------- lint rules: firing and passing ----------

#[test]
fn unknown_node_type_fires_and_known_passes() {
    let d = def(
        serde_json::json!([{ "id": "a", "type": "teleport" }]),
        serde_json::json!([]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    assert!(codes(&findings).contains(&"unknown-node-type"));

    // The registry (via known_types) is the authority: a wider set passes.
    let findings = lint_workflow_v2(&d, &["agent", "teleport"]);
    assert!(!codes(&findings).contains(&"unknown-node-type"));
}

#[test]
fn missing_prompt_fires_only_for_agent_nodes() {
    let d = def(
        serde_json::json!([
            { "id": "a", "config": { "prompt_template": "  " } },
            { "id": "g", "type": "gate" }
        ]),
        serde_json::json!([{ "from": "a", "to": "g" }]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "missing-prompt")
        .collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].node.as_ref().unwrap().as_str(), "a");
}

#[test]
fn redirect_target_must_exist_and_be_an_ancestor() {
    // b redirects to a (its ancestor): fine. c redirects to sibling d: error.
    // e redirects to nothing that exists: error. f omits the target: error.
    let retry =
        |to: &str| serde_json::json!({ "verdict": { "strategy": "redirect", "redirect_to": to } });
    let d = def(
        serde_json::json!([
            { "id": "a" },
            { "id": "b", "retry": retry("a") },
            { "id": "c", "retry": retry("d") },
            { "id": "d" },
            { "id": "e", "retry": retry("ghost") },
            { "id": "f", "retry": { "verdict": { "strategy": "redirect" } } }
        ]),
        serde_json::json!([
            { "from": "a", "to": "b" }, { "from": "b", "to": "c" },
            { "from": "b", "to": "d" }, { "from": "d", "to": "e" },
            { "from": "e", "to": "f" }
        ]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    let by_code = |code: &str| -> Vec<&str> {
        findings
            .iter()
            .filter(|f| f.code == code)
            .map(|f| f.node.as_ref().unwrap().as_str())
            .collect()
    };
    assert_eq!(by_code("redirect-not-ancestor"), vec!["c"]);
    assert_eq!(by_code("redirect-unknown-target"), vec!["e"]);
    assert_eq!(by_code("redirect-missing-target"), vec!["f"]);
    assert!(
        !by_code("redirect-not-ancestor").contains(&"b"),
        "valid ancestor redirect passes"
    );
}

#[test]
fn finalize_shape_rules() {
    // Zero finalize → warning.
    let d = def(serde_json::json!([{ "id": "a" }]), serde_json::json!([]));
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    assert!(codes(&findings).contains(&"no-finalize"));
    assert!(errors(&findings).is_empty(), "warning only");

    // Two finalize → error on each.
    let d = def(
        serde_json::json!([
            { "id": "a" }, { "id": "f1", "type": "finalize" }, { "id": "f2", "type": "finalize" }
        ]),
        serde_json::json!([{ "from": "a", "to": "f1" }, { "from": "a", "to": "f2" }]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    assert_eq!(
        codes(&findings)
            .iter()
            .filter(|c| **c == "multiple-finalize")
            .count(),
        2
    );

    // Finalize with outgoing edges → error.
    let d = def(
        serde_json::json!([{ "id": "f", "type": "finalize" }, { "id": "after" }]),
        serde_json::json!([{ "from": "f", "to": "after" }]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    assert!(codes(&findings).contains(&"finalize-not-sink"));

    // One trailing finalize → clean of all three.
    let d = def(
        serde_json::json!([{ "id": "a" }, { "id": "f", "type": "finalize" }]),
        serde_json::json!([{ "from": "a", "to": "f" }]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    for code in ["no-finalize", "multiple-finalize", "finalize-not-sink"] {
        assert!(!codes(&findings).contains(&code), "{code}");
    }
}

#[test]
fn non_finalize_sink_warns_as_dead_end() {
    let d = def(
        serde_json::json!([
            { "id": "a" }, { "id": "orphan" }, { "id": "f", "type": "finalize" }
        ]),
        serde_json::json!([{ "from": "a", "to": "f" }, { "from": "a", "to": "orphan" }]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    let hits: Vec<&str> = findings
        .iter()
        .filter(|f| f.code == "dead-end")
        .map(|f| f.node.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(hits, vec!["orphan"]);
    assert!(errors(&findings).is_empty(), "dead-end is a warning");
}

#[test]
fn port_type_mismatch_fires_only_when_both_sides_declare() {
    let nodes = serde_json::json!([
        { "id": "producer", "config": { "prompt_template": "p",
            "outputs": [ { "name": "verdict", "type": "verdict" } ] } },
        { "id": "consumer", "type": "sequence", "config": {
            "inputs": [ { "name": "tasks", "type": "task_list" } ] } },
        { "id": "flexible", "type": "sequence", "config": {
            "inputs": [ { "name": "anything", "type": "any" } ] } },
        { "id": "undeclared", "type": "gate" },
        { "id": "f", "type": "finalize" }
    ]);
    let d = def(
        nodes,
        serde_json::json!([
            { "from": "producer", "to": "consumer" },
            { "from": "producer", "to": "flexible" },
            { "from": "producer", "to": "undeclared" },
            { "from": "consumer", "to": "f" }, { "from": "flexible", "to": "f" },
            { "from": "undeclared", "to": "f" }
        ]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "port-type-mismatch")
        .collect();
    assert_eq!(hits.len(), 1, "{findings:?}");
    assert_eq!(
        hits[0].edge.as_ref().unwrap(),
        &(StepId::from("producer"), StepId::from("consumer")),
        "verdict → task_list is incompatible; any accepts; undeclared is skipped"
    );
}

#[test]
fn guarded_all_success_join_warns_and_optins_pass() {
    let mk = |join: Option<&str>| {
        let mut node = serde_json::json!({ "id": "j", "type": "gate" });
        if let Some(j) = join {
            node["join"] = j.into();
        }
        def(
            serde_json::json!([{ "id": "a" }, { "id": "b" }, node]),
            serde_json::json!([
                { "from": "a", "to": "j", "when": "${{ nodes.a.outputs.verdict != 'FAIL' }}" },
                { "from": "b", "to": "j" }
            ]),
        )
    };

    let findings = lint_workflow_v2(&mk(None), &CORE_NODE_TYPES);
    assert!(codes(&findings).contains(&"guarded-all-success-join"));
    assert!(errors(&findings).is_empty(), "warning, not error");

    let findings = lint_workflow_v2(&mk(Some("any_success")), &CORE_NODE_TYPES);
    assert!(!codes(&findings).contains(&"guarded-all-success-join"));

    let findings = lint_workflow_v2(&mk(Some("all_done")), &CORE_NODE_TYPES);
    assert!(!codes(&findings).contains(&"guarded-all-success-join"));
}

#[test]
fn build_failure_short_circuits_but_reports_node_local_rules() {
    // A cyclic graph still reports the unknown node type; graph-dependent
    // rules are skipped rather than run against a broken graph.
    let d = def(
        serde_json::json!([
            { "id": "a", "type": "teleport" }, { "id": "b" }
        ]),
        serde_json::json!([{ "from": "a", "to": "b" }, { "from": "b", "to": "a" }]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    assert!(codes(&findings).contains(&"unknown-node-type"));
    assert!(codes(&findings).contains(&"cycle"));
}

// ---------- the seven starters ----------

#[test]
fn migrated_starters_lint_clean() {
    for name in [
        "bugfix-pipeline",
        "ci-fix",
        "docs-update",
        "experiment",
        "refactor",
        "simple-task",
        "standard-feature-pipeline",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/workflows")
            .join(format!("{name}.json"));
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let migrated = migrate_definition(&doc).unwrap();
        let findings = lint_workflow_v2(&migrated, &CORE_NODE_TYPES);
        assert!(
            findings.is_empty(),
            "starter '{name}' should lint clean, got: {findings:#?}"
        );
    }
}

// ---------- rework-target-without-template ----------
//
// Redirecting a verdict to the step that *writes* a task list is what makes
// a rework cycle cheap — the producer reads the verdict and emits a delta.
// It only works if the producer knows it is in a rework cycle. Without a
// rework template it answers with a whole fresh decomposition, so the
// redirect costs a planning turn *and* re-runs every task: strictly worse
// than the shape it was meant to improve on.

/// A producer → sequence → verdict chain, where the verdict redirects to
/// the producer.
fn rework_shape(rework_template: Option<&str>) -> WorkflowDefinitionV2 {
    let mut producer_config = serde_json::json!({ "prompt_template": "decompose" });
    if let Some(t) = rework_template {
        producer_config["rework_prompt_template"] = serde_json::json!(t);
    }
    def(
        serde_json::json!([
            { "id": "tickets", "config": producer_config },
            { "id": "implement", "type": "sequence",
              "config": { "task_list_from": "tickets" } },
            { "id": "validate",
              "config": { "prompt_template": "check" },
              "retry": { "verdict": { "strategy": "redirect", "redirect_to": "tickets" } } }
        ]),
        serde_json::json!([
            { "from": "tickets", "to": "implement" },
            { "from": "implement", "to": "validate" }
        ]),
    )
}

#[test]
fn redirecting_to_a_producer_with_no_rework_template_warns() {
    let findings = lint_workflow_v2(&rework_shape(None), &CORE_NODE_TYPES);
    assert!(
        codes(&findings).contains(&"rework-target-without-template"),
        "{findings:?}"
    );
    // A warning: the workflow runs, it is just expensive. Blocking the save
    // would refuse a definition the engine executes fine.
    assert!(errors(&findings).is_empty(), "{findings:?}");
}

#[test]
fn redirecting_to_a_producer_that_declares_one_is_clean() {
    let findings = lint_workflow_v2(
        &rework_shape(Some("emit only what closes the verdict")),
        &CORE_NODE_TYPES,
    );
    assert!(
        !codes(&findings).contains(&"rework-target-without-template"),
        "{findings:?}"
    );
}

#[test]
fn a_blank_rework_template_does_not_silence_the_warning() {
    let findings = lint_workflow_v2(&rework_shape(Some("  ")), &CORE_NODE_TYPES);
    assert!(
        codes(&findings).contains(&"rework-target-without-template"),
        "{findings:?}"
    );
}

#[test]
fn redirecting_to_a_node_that_produces_no_task_list_is_not_flagged() {
    // The overwhelmingly common shape — a verdict sending the run back to
    // the step that implements — must stay silent, or every workflow in
    // existence grows a badge.
    let d = def(
        serde_json::json!([
            { "id": "spec", "config": { "prompt_template": "spec" } },
            { "id": "implement", "config": { "prompt_template": "build" } },
            { "id": "validate",
              "config": { "prompt_template": "check" },
              "retry": { "verdict": { "strategy": "redirect", "redirect_to": "implement" } } }
        ]),
        serde_json::json!([
            { "from": "spec", "to": "implement" },
            { "from": "implement", "to": "validate" }
        ]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    assert!(
        !codes(&findings).contains(&"rework-target-without-template"),
        "{findings:?}"
    );
}

#[test]
fn the_v2_edge_binding_counts_as_a_producer_too() {
    // v2 expresses the binding as an edge from a `task-list` artifact
    // producer into a sequence node, and `project_v2_to_v1` reads it back
    // that way. A rule that only read the config key would fire on the
    // migrated form and go silent on the native one it round-trips to.
    let d = def(
        serde_json::json!([
            { "id": "tickets", "config": {
                "prompt_template": "decompose",
                "artifacts": [{ "name": "task-list" }] } },
            { "id": "implement", "type": "sequence", "config": {} },
            { "id": "validate",
              "config": { "prompt_template": "check" },
              "retry": { "verdict": { "strategy": "redirect", "redirect_to": "tickets" } } }
        ]),
        serde_json::json!([
            { "from": "tickets", "to": "implement" },
            { "from": "implement", "to": "validate" }
        ]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    assert!(
        codes(&findings).contains(&"rework-target-without-template"),
        "{findings:?}"
    );
}

#[test]
fn a_gate_between_a_producer_and_its_sequence_node_is_not_a_consumer() {
    // The `refactor` starter's shape. Without the sequence-kind check the
    // gate's incoming edge would make its predecessor read as a producer
    // for the *gate*, and a redirect at the gate would grow a warning about
    // a binding nobody wrote.
    let d = def(
        serde_json::json!([
            { "id": "analyse", "config": {
                "prompt_template": "plan",
                "artifacts": [{ "name": "task-list" }] } },
            { "id": "gate", "type": "gate", "config": {} },
            { "id": "check",
              "config": { "prompt_template": "check" },
              "retry": { "verdict": { "strategy": "redirect", "redirect_to": "analyse" } } }
        ]),
        serde_json::json!([
            { "from": "analyse", "to": "gate" },
            { "from": "gate", "to": "check" }
        ]),
    );
    let findings = lint_workflow_v2(&d, &CORE_NODE_TYPES);
    assert!(
        !codes(&findings).contains(&"rework-target-without-template"),
        "nothing executes this list, so there is no rework cycle to warn about: {findings:?}"
    );
}
