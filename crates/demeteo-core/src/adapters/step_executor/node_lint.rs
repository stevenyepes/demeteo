//! Registry-aware structural lint: the single answer to *"would this build's
//! engine accept this definition?"* (task P3.3, PRD §6.3).
//!
//! Two rule sources have to be combined, and neither can do it alone:
//!
//! - [`lint_workflow_v2`] owns the graph-level rules (cycles, reachability,
//!   sink shape, redirect targets, ports, joins) but takes its "known node
//!   types" list from the caller — it is a pure domain module and cannot see
//!   the registry.
//! - [`NodeHandler::lint`] owns the per-type rules, and only the registry can
//!   reach the handlers.
//!
//! [`lint_definition`] joins them and passes the **registry's** kinds as the
//! known-types list, which is what retires the hand-maintained
//! [`CORE_NODE_TYPES`] constant from every boundary caller: a node type added
//! in Rust (P3.5's `command`) stops linting as `unknown-node-type` the moment
//! it is registered, with no edit here.
//!
//! Retired aliases (`parallel`) count as known. The palette deliberately
//! excludes them (`node_catalog`) — new work must not be authored on a dead
//! name — but lint answers a different question: *the engine dispatches this
//! today*, so flagging it would make a workflow cloned before the rename
//! unsavable in the builder it is meant to be edited in.

use crate::domain::models::workflow_v2::{NodeConfig, PortType, WorkflowDefinitionV2};
use crate::domain::workflow_graph::{declared_ports, lint_workflow_v2, LintFinding, WorkflowGraph};

use super::registry::NodeTypeRegistry;

/// Every finding for `def`, graph-level rules first, then per-node type rules
/// in node order. An empty vec means the definition is clean; warnings are
/// included, so callers that gate on validity filter with
/// [`crate::domain::workflow_graph::has_errors`] (PRD §6.3: *"save is blocked
/// only by errors, not warnings"*).
pub fn lint_definition(def: &WorkflowDefinitionV2) -> Vec<LintFinding> {
    let registry = NodeTypeRegistry::global();
    let known = known_types(registry);
    // `&'static str` → `&str` for the domain module's borrowed signature.
    let known_refs: Vec<&str> = known.to_vec();

    let mut findings = lint_workflow_v2(def, &known_refs);

    // Per-type rules need adjacency. When the graph itself is broken the
    // graph-level pass has already said so and a handler asking about
    // ancestors would be answering about a graph the engine will never build.
    if let Ok(graph) = WorkflowGraph::build(def) {
        findings.extend(type_default_port_findings(def, registry));
        for node in &def.nodes {
            if let Some(handler) = registry.handler_for(&node.node_type) {
                findings.extend(handler.lint(node, &graph));
            }
        }
    }

    findings
}

/// The half of the port rule the pure domain module cannot express.
///
/// [`lint_workflow_v2`] only judges an edge when **both** endpoints declare
/// ports in their `config`, because it has no way to learn a node type's
/// default ports — those live on [`NodeHandler::ports`](super::registry::NodeHandler::ports),
/// behind the registry. A freshly dropped node declares nothing, so without
/// this pass every edge between two fresh nodes went unchecked in Rust while
/// the builder's `connectRules.ts` — which *does* read the type defaults, off
/// `node_types_list` — refused it at connect time. The editor was therefore
/// stricter than the engine, the exact inversion of the guarantee both files
/// claim ("the editor refuses exactly the shapes the engine refuses").
///
/// Fires only where the domain rule stayed silent, so an edge is never judged
/// twice, and defers to `finalize-not-sink` for the one case that already has
/// a dedicated, better-worded rule.
fn type_default_port_findings(
    def: &WorkflowDefinitionV2,
    registry: &'static NodeTypeRegistry,
) -> Vec<LintFinding> {
    let node_by_id = |id: &crate::domain::ids::StepId| def.nodes.iter().find(|n| n.id == *id);
    let effective = |node: &NodeConfig, key: &str| -> Vec<PortType> {
        let declared = declared_ports(node, key);
        if !declared.is_empty() {
            return declared;
        }
        match registry.handler_for(&node.node_type) {
            Some(h) if key == "outputs" => h.ports().outputs.to_vec(),
            Some(h) => h.ports().inputs.to_vec(),
            // An unknown type is already an `unknown-node-type` error; don't
            // pile a port complaint on top of it.
            None => vec![PortType::Any],
        }
    };

    let mut findings = Vec::new();
    for edge in &def.edges {
        let (Some(from), Some(to)) = (node_by_id(&edge.from), node_by_id(&edge.to)) else {
            continue;
        };
        // Both sides declared: `lint_workflow_v2` has already ruled on this.
        if !declared_ports(from, "outputs").is_empty() && !declared_ports(to, "inputs").is_empty() {
            continue;
        }
        let outputs = effective(from, "outputs");
        let inputs = effective(to, "inputs");
        // `finalize` produces nothing by design; `finalize-not-sink` says so
        // in words an author can act on.
        if outputs.is_empty() && from.node_type == "finalize" {
            continue;
        }
        let compatible = !outputs.is_empty()
            && !inputs.is_empty()
            && outputs
                .iter()
                .any(|o| inputs.iter().any(|i| o.compatible_with(*i)));
        if !compatible {
            findings.push(LintFinding::edge_error(
                "port-type-mismatch",
                &edge.from,
                &edge.to,
                format!(
                    "edge '{}' → '{}' connects no compatible ports (outputs {:?} vs \
                     inputs {:?})",
                    edge.from, edge.to, outputs, inputs
                ),
            ));
        }
    }
    findings
}

/// Canonical kinds plus retired aliases — everything `handler_for` resolves.
fn known_types(registry: &'static NodeTypeRegistry) -> Vec<&'static str> {
    let mut known = registry.kinds();
    for handler in registry.handlers() {
        known.extend(handler.aliases().iter().copied());
    }
    known
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow_graph::{has_errors, LintSeverity};

    fn def(value: serde_json::Value) -> WorkflowDefinitionV2 {
        serde_json::from_value(value).expect("test definition parses as v2")
    }

    fn codes(findings: &[LintFinding]) -> Vec<&str> {
        findings.iter().map(|f| f.code).collect()
    }

    #[test]
    fn a_plain_chain_lints_clean() {
        let findings = lint_definition(&def(serde_json::json!({
            "schema_version": 2,
            "id": "wf-chain",
            "name": "chain",
            "nodes": [
                { "id": "plan", "type": "agent", "title": "Plan",
                  "config": { "prompt_template": "do the thing" } },
                { "id": "publish", "type": "finalize", "title": "Publish", "config": {} }
            ],
            "edges": [{ "from": "plan", "to": "publish" }]
        })));
        assert!(findings.is_empty(), "expected clean, got {findings:?}");
    }

    #[test]
    fn registered_kinds_are_known_and_unregistered_ones_are_not() {
        // The point of routing lint through the registry: `sequence` is known
        // because a handler owns it, not because a constant lists it.
        for kind in NodeTypeRegistry::global().kinds() {
            let findings = lint_definition(&def(serde_json::json!({
                "schema_version": 2,
                "id": "wf-k",
                "name": "k",
                "nodes": [
                    { "id": "n1", "type": kind, "title": "N",
                      "config": { "prompt_template": "p" } }
                ],
                "edges": []
            })));
            assert!(
                !codes(&findings).contains(&"unknown-node-type"),
                "{kind} is registered but linted as unknown: {findings:?}"
            );
        }

        let findings = lint_definition(&def(serde_json::json!({
            "schema_version": 2,
            "id": "wf-k",
            "name": "k",
            "nodes": [{ "id": "n1", "type": "teleport", "title": "N", "config": {} }],
            "edges": []
        })));
        assert!(codes(&findings).contains(&"unknown-node-type"));
        assert!(has_errors(&findings));
    }

    #[test]
    fn a_retired_alias_is_known_because_the_engine_still_dispatches_it() {
        let findings = lint_definition(&def(serde_json::json!({
            "schema_version": 2,
            "id": "wf-alias",
            "name": "alias",
            "nodes": [{ "id": "n1", "type": "parallel", "title": "N", "config": {} }],
            "edges": []
        })));
        assert!(
            !codes(&findings).contains(&"unknown-node-type"),
            "the `parallel` alias still runs, so it must stay savable: {findings:?}"
        );
    }

    #[test]
    fn graph_and_node_level_errors_both_surface() {
        let findings = lint_definition(&def(serde_json::json!({
            "schema_version": 2,
            "id": "wf-bad",
            "name": "bad",
            "nodes": [
                // No prompt (node rule) …
                { "id": "plan", "type": "agent", "title": "Plan", "config": {} },
                // … and a redirect to a node that isn't an ancestor (graph rule).
                { "id": "check", "type": "agent", "title": "Check",
                  "config": { "prompt_template": "p" },
                  "retry": { "verdict": { "strategy": "redirect", "redirect_to": "nowhere" } } }
            ],
            "edges": [{ "from": "plan", "to": "check" }]
        })));
        let codes = codes(&findings);
        assert!(codes.contains(&"missing-prompt"), "{findings:?}");
        assert!(codes.contains(&"redirect-unknown-target"), "{findings:?}");
        assert!(has_errors(&findings));
    }

    #[test]
    fn warnings_alone_do_not_block_save() {
        // No finalize node: a real observation (nothing gets published) but
        // the run is legal, so it must not stop the author saving.
        let findings = lint_definition(&def(serde_json::json!({
            "schema_version": 2,
            "id": "wf-warn",
            "name": "warn",
            "nodes": [
                { "id": "plan", "type": "agent", "title": "Plan",
                  "config": { "prompt_template": "p" } }
            ],
            "edges": []
        })));
        assert_eq!(codes(&findings), vec!["no-finalize"]);
        assert_eq!(findings[0].severity, LintSeverity::Warning);
        assert!(!has_errors(&findings));
    }

    #[test]
    fn a_stray_task_list_from_is_caught_before_the_run() {
        // v2 puts the binding on an edge, so the builder can never write this
        // — but a hand-edited or imported document can, and the schema allows
        // additional properties. Left unlinted it surfaces as a mid-run
        // `NonRetryable` from `load_task_list_artifact`.
        let findings = lint_definition(&def(serde_json::json!({
            "schema_version": 2,
            "id": "wf-tl",
            "name": "tl",
            "nodes": [
                { "id": "plan", "type": "agent", "title": "Plan",
                  "config": { "prompt_template": "p" } },
                { "id": "work", "type": "sequence", "title": "Work",
                  "config": { "task_list_from": "ghost" } }
            ],
            "edges": [{ "from": "plan", "to": "work" }]
        })));
        assert!(
            codes(&findings).contains(&"task-list-unknown-source"),
            "{findings:?}"
        );
        assert!(has_errors(&findings));
    }

    #[test]
    fn a_task_list_from_naming_a_real_node_is_only_a_warning() {
        // It still runs — it just acts as a dependency the canvas never draws,
        // which is an observation, not a reason to block the save (PRD §6.3).
        let findings = lint_definition(&def(serde_json::json!({
            "schema_version": 2,
            "id": "wf-tl2",
            "name": "tl2",
            "nodes": [
                { "id": "plan", "type": "agent", "title": "Plan",
                  "config": { "prompt_template": "p" } },
                { "id": "work", "type": "sequence", "title": "Work",
                  "config": { "task_list_from": "plan" } }
            ],
            "edges": [{ "from": "plan", "to": "work" }]
        })));
        assert!(
            codes(&findings).contains(&"task-list-legacy-binding"),
            "{findings:?}"
        );
        assert!(!has_errors(&findings));
    }

    #[test]
    fn type_default_ports_are_enforced_so_the_editor_is_not_stricter_than_the_engine() {
        // `connectRules.ts` reads the registry's type-level ports and refuses a
        // `finalize → x` edge at connect time. Before this pass the Rust lint
        // only judged edges where *both* nodes declared ports in their config,
        // so a definition the builder wouldn't let you draw could still be
        // imported and saved.
        let findings = lint_definition(&def(serde_json::json!({
            "schema_version": 2,
            "id": "wf-ports",
            "name": "ports",
            "nodes": [
                { "id": "publish", "type": "finalize", "title": "Publish", "config": {} },
                { "id": "after", "type": "agent", "title": "After",
                  "config": { "prompt_template": "p" } }
            ],
            "edges": [{ "from": "publish", "to": "after" }]
        })));
        // `finalize-not-sink` is the better-worded rule for this shape and
        // still owns it; the port pass must not double-report.
        assert!(
            codes(&findings).contains(&"finalize-not-sink"),
            "{findings:?}"
        );
        assert_eq!(
            codes(&findings)
                .iter()
                .filter(|c| **c == "port-type-mismatch")
                .count(),
            0,
            "the dedicated finalize rule owns this edge: {findings:?}"
        );
        assert!(has_errors(&findings));
    }

    #[test]
    fn every_migrated_starter_lints_clean() {
        // The builder's lint surface must agree with what the engine already
        // runs every day — a starter that linted dirty would show error badges
        // on a workflow the user never touched, and (via `has_errors` at the
        // write paths) could not be re-saved after an edit.
        const STARTERS: [&str; 7] = [
            "bugfix-pipeline",
            "ci-fix",
            "docs-update",
            "experiment",
            "refactor",
            "simple-task",
            "standard-feature-pipeline",
        ];
        for name in STARTERS {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../src-tauri/workflows")
                .join(format!("{name}.json"));
            let body = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read starter {}: {e}", path.display()));
            let value: serde_json::Value =
                serde_json::from_str(&body).expect("starter is valid JSON");
            let migrated = crate::domain::models::workflow_migrate::migrate_definition(&value)
                .unwrap_or_else(|e| panic!("{name} migrates: {e}"));

            let findings = lint_definition(&migrated);
            assert!(
                !has_errors(&findings),
                "{name} lints with errors: {findings:?}"
            );
        }
    }
}
