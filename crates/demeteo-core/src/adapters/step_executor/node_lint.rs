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

use crate::domain::models::workflow_v2::WorkflowDefinitionV2;
use crate::domain::workflow_graph::{lint_workflow_v2, LintFinding, WorkflowGraph};

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
        for node in &def.nodes {
            if let Some(handler) = registry.handler_for(&node.node_type) {
                findings.extend(handler.lint(node, &graph));
            }
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
