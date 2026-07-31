//! Registry-aware structural lint: the single answer to *"would this build's
//! engine accept this definition?"* (task P3.3, PRD §6.3).
//!
//! Two rule sources have to be combined, and neither can do it alone:
//!
//! - [`lint_workflow_v2`] owns the graph-level rules (cycles, reachability,
//!   sink shape, redirect targets, ports, joins) but takes its "known node
//!   types" list from the caller — it is a pure domain module and cannot see
//!   the registry.
//! - [`NodeHandler::lint`](super::registry::NodeHandler::lint) owns the per-type rules, and only the registry can
//!   reach the handlers.
//!
//! [`lint_definition`] joins them and passes the **registry's** kinds as the
//! known-types list, which is what retires the hand-maintained
//! [`CORE_NODE_TYPES`](crate::domain::workflow_graph::CORE_NODE_TYPES) constant from every boundary caller: a node type added
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

    const STARTERS: [&str; 7] = [
        "bugfix-pipeline",
        "ci-fix",
        "docs-update",
        "experiment",
        "refactor",
        "simple-task",
        "standard-feature-pipeline",
    ];

    /// The bundled starter JSON as authored, straight off disk — the same file
    /// `seed_starter_workflows` ships in the binary.
    fn starter(name: &str) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src-tauri/workflows")
            .join(format!("{name}.json"));
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read starter {}: {e}", path.display()));
        serde_json::from_str(&body).expect("starter is valid JSON")
    }

    #[test]
    fn every_migrated_starter_lints_clean() {
        // The builder's lint surface must agree with what the engine already
        // runs every day — a starter that linted dirty would show error badges
        // on a workflow the user never touched, and (via `has_errors` at the
        // write paths) could not be re-saved after an edit.
        for name in STARTERS {
            let value = starter(name);
            let migrated = crate::domain::models::workflow_migrate::migrate_definition(&value)
                .unwrap_or_else(|e| panic!("{name} migrates: {e}"));

            let findings = lint_definition(&migrated);
            assert!(
                !has_errors(&findings),
                "{name} lints with errors: {findings:?}"
            );
        }
    }

    /// One `[attached — …]` reference found in a starter's prompt text.
    struct AttachedRef {
        /// The referencing step's id, for the assertion message.
        from: String,
        /// The text after the dash, as `resolve_attached_artifacts` sees it.
        payload: String,
    }

    /// Every `[attached — …]` reference in a starter's prompts, in declaration
    /// order. The `previous step artifact` spelling is skipped: it names a
    /// position, not a step, so there is nothing to resolve.
    fn attached_refs(wf: &serde_json::Value) -> Vec<AttachedRef> {
        let mut out = Vec::new();
        for step in wf["steps"].as_array().expect("starter has steps") {
            let from = step["id"].as_str().unwrap_or_default().to_string();
            let sources = [
                step["prompt_template"].as_str(),
                step["verifier"]["instructions"].as_str(),
                step["rework_prompt_template"].as_str(),
            ];
            for text in sources.into_iter().flatten() {
                // Mirrors `resolve_attached_artifacts`'s own scan: opening
                // token `[attached`, closing `]`, payload after the dash.
                let mut rest = text;
                while let Some(start) = rest.find("[attached") {
                    let after = &rest[start..];
                    let Some(end) = after.find(']') else { break };
                    let inside = &after[1..end];
                    let payload = inside
                        .split(['\u{2014}', '\u{2013}'])
                        .nth(1)
                        .map(str::trim)
                        .unwrap_or_default();
                    if !payload.is_empty() && payload != "previous step artifact" {
                        out.push(AttachedRef {
                            from: from.clone(),
                            payload: payload.to_string(),
                        });
                    }
                    rest = &rest[start + 1..];
                }
            }
        }
        out
    }

    #[test]
    fn every_starter_attachment_names_exactly_one_step_it_declares() {
        // A prompt that references a step the workflow does not declare renders
        // as "(Artifact '…' not found or not yet generated)" — silently, mid-run,
        // with the agent then reasoning from a hole. `lint_definition` cannot see
        // it because the reference lives inside prompt *text*, so nothing else in
        // the gate would notice a step being deleted out from under its readers.
        //
        // *Exactly* one, because `resolve_attached_artifacts` resolves by
        // substring in both directions (`content.contains(id) ||
        // id.contains(content)`) and takes the first hit, so two candidates make
        // the binding order-dependent.
        //
        // Deliberately **not** asserted: that the target is declared earlier. A
        // step downstream of the reader is a legitimate target on a rework cycle,
        // by which point it has already run once — `s-analyse`'s
        // `rework_prompt_template` attaches the `s-regression` report that
        // rejected it (decision 43), and Standard's `s-implement` attaches
        // `s-critic` under an explicit "on a re-run" heading.
        for name in STARTERS {
            let wf = starter(name);
            let ids: Vec<String> = wf["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["id"].as_str().unwrap_or_default().to_lowercase())
                .collect();

            for AttachedRef { from, payload } in attached_refs(&wf) {
                let payload_lower = payload.to_lowercase();
                let matches: Vec<usize> = ids
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| payload_lower.contains(*id) || id.contains(&payload_lower))
                    .map(|(i, _)| i)
                    .collect();
                assert_eq!(
                    matches.len(),
                    1,
                    "{name}: `{from}` attaches [attached — {payload}], which resolves to \
                     {} declared steps ({:?}) — it must name exactly one",
                    matches.len(),
                    matches.iter().map(|i| &ids[*i]).collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn the_refactor_starter_takes_its_baseline_from_the_engine_not_an_agent() {
        // F2 (`docs/HARNESS_BASELINE.md` §5). The starter used to carry two
        // baselines: the `s-baseline-harness` command node, which *measures*,
        // and an `s-baseline` agent step, which ran the suite itself and wrote
        // prose about it. Decision 44 rejects the second — the thing being
        // judged must not produce the evidence — so the agent step is gone and
        // its three readers were re-pointed at the measurement.
        let wf = starter("refactor");
        let steps = wf["steps"].as_array().unwrap();
        let ids: Vec<&str> = steps.iter().map(|s| s["id"].as_str().unwrap()).collect();

        assert!(
            !ids.contains(&"s-baseline"),
            "the agent baseline step is back: {ids:?}"
        );
        // The measurement is still the head of the graph, and still measures:
        // deleting the agent step must not have taken the record with it.
        assert_eq!(ids.first(), Some(&"s-baseline-harness"));
        assert_eq!(steps[0]["kind"].as_str(), Some("command"));
        assert_eq!(steps[0]["measure_baseline"].as_bool(), Some(true));

        // Nothing may still read the deleted step's artifact. This is the
        // failure mode the whole task is about: `artifacts/s-baseline.md` is
        // never written now, so a surviving reader would be reading a file that
        // does not exist.
        let raw = serde_json::to_string(&wf).unwrap();
        assert!(
            !raw.contains("s-baseline.md"),
            "a step still reads the deleted agent baseline's artifact"
        );
    }

    #[test]
    fn every_test_gated_starter_measures_the_harness_before_any_agent_runs() {
        // A test-gated run begins in a fresh worktree. Its configured prepare
        // command must run before the default test command, rather than being
        // left to an agent prompt to infer and execute on its own. Experiment
        // is deliberately excluded: it has no verifier and therefore no
        // default test gate to measure.
        for name in [
            "bugfix-pipeline",
            "ci-fix",
            "docs-update",
            "refactor",
            "simple-task",
            "standard-feature-pipeline",
        ] {
            let wf = starter(name);
            let steps = wf["steps"].as_array().unwrap();

            assert_eq!(
                steps.first().and_then(|step| step["id"].as_str()),
                Some("s-baseline-harness"),
                "{name} must measure its prepare and default test commands first"
            );
            assert_eq!(steps[0]["kind"].as_str(), Some("command"));
            assert_eq!(steps[0]["measure_baseline"].as_bool(), Some(true));
        }
    }

    #[test]
    fn the_refactor_no_harness_skip_branch_is_keyed_on_what_the_engine_renders() {
        // The `NO_HARNESS` path used to be an agent's own prose verdict line in
        // `artifacts/s-baseline.md`. With that step gone the skip branch is
        // driven by the `{{harness_baseline}}` block, which is engine-rendered —
        // so the branch is only reachable if the prompt keys on wording the
        // renderer actually emits. Pinning both sides in one test is what stops
        // a reword on either side from silently stranding the branch.
        let wf = starter("refactor");
        let regression = wf["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "s-regression")
            .expect("refactor declares s-regression");
        let prompt = regression["prompt_template"].as_str().unwrap();

        assert!(
            prompt.contains("{{harness_baseline}}"),
            "s-regression no longer binds the engine's baseline block"
        );

        // What the renderer says when the project configures no gate at all.
        let briefing = crate::domain::harness_baseline::render_harness_briefing(&[], None);
        let marker = "NOTHING";
        assert!(
            briefing.contains(marker),
            "the no-gate briefing no longer says {marker:?}: {briefing}"
        );
        assert!(
            prompt.contains(marker),
            "s-regression's skip branch no longer keys on {marker:?}"
        );

        // And the branch it guards is still the skip, not a comparison.
        assert!(prompt.contains("VERDICT: ALL CLEAR"));
    }
}
