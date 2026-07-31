//! Pure v1 → v2 workflow-definition migration (task P1.2, PRD §5.1).
//!
//! v1 is the ordered `Vec<StepConfig>` list ([`super::workflow`]); v2 is the
//! nodes+edges graph ([`super::workflow_v2`]). The mapping is **pure and
//! total** — every v1 definition (all seven starters plus any user custom)
//! migrates without error, and the produced graph is a chain, so behavior
//! under the v2 engine must be identical (the P0.2 starter-baseline harness
//! is the gate once the runtime consumes this in P1.12/P1.15):
//!
//! - **List order → chain edges.** `steps[i] → steps[i+1]` for every
//!   consecutive pair.
//! - **`on_failure` → retry policy.** The backward goto becomes the same
//!   `{ strategy: redirect, redirect_to, max_attempts, feedback: true }`
//!   rule for **both** `retry.verdict` and `retry.agent_failure` — v1 sent
//!   both failure classes through the one `on_failure` path (see
//!   `retry_policy::legacy_policy_for_step`), so a v2 policy that only
//!   covered `verdict` would silently stop redirecting plain agent
//!   failures once the engine executes migrated definitions natively.
//!   `max_attempts` carries the step's own `max_iterations` only — the
//!   run-time precedence above it (run override → project default →
//!   engine default 3) keeps applying at evaluation time (P1.10), exactly
//!   as `effective_loop_iterations` resolves today.
//! - **`task_list_from` → typed edge.** The magic field becomes a normal
//!   edge from the task-list-producing node into the sequence node
//!   (deduplicated when the source is already the chain predecessor). The
//!   binding stays derivable: it is the incoming edge whose source declares
//!   the `task-list` artifact.
//! - **`parallel` → `sequence`.** The superseded alias resolves at
//!   migration; v2 documents never contain `parallel`.
//! - **Positions synthesized** as a simple vertical column; the editor's
//!   auto-layout takes over from there.
//!
//! Everything else a step carries (prompt template, agent/model/effort,
//! capability, artifacts, verifier, shell/network switches, gate class)
//! moves verbatim into the node's opaque `config` payload — the per-type
//! schemas that formalize those fields arrive with the node handlers
//! (P1.6). A `max_iterations` on a step *without* `on_failure` is inert at
//! run time today (nothing consumes it outside the on-failure path), but it
//! is author intent, so it stays in `config` rather than being dropped.
//!
//! The reverse direction ([`project_v2_to_v1`], task P3.6) exists because
//! storage carries **both** representations: `workflow_versions` gained a
//! `definition_json` column (V34) holding the v2 document, while `steps_json`
//! keeps holding a v1 projection so the runner, replay, and export keep
//! working unchanged. The projection is lossy by construction — v1 has no
//! place for positions, joins, per-class retry, or edge guards — which is
//! exactly why the v2 document is stored beside it rather than derived from
//! it. For a *chain*, the two functions are inverses, and the round-trip over
//! all seven starters is a test.

use crate::domain::ids::{StepId, WorkflowId};
use crate::domain::models::workflow::StepConfig;
use crate::domain::models::workflow_v2::{
    validate_workflow_v2, EdgeConfig, NodeConfig, Position, RetryPolicy, RetryRule, RetryStrategy,
    WorkflowDefaults, WorkflowDefinitionV2, WORKFLOW_SCHEMA_V2,
};

/// Vertical spacing between synthesized node positions. Arbitrary but
/// stable: golden files and version diffs shouldn't churn on layout.
const VERTICAL_SPACING: f64 = 160.0;

/// Fields of [`StepConfig`] that become first-class node/graph structure
/// in v2 and therefore must not also appear in the opaque `config` payload.
const LIFTED_FIELDS: [&str; 5] = ["id", "kind", "title", "on_failure", "task_list_from"];

/// Migrate a v1 ordered step list into a v2 graph. Pure and total: never
/// fails, never does I/O. See the module docs for the mapping.
pub fn migrate_v1_to_v2(
    id: WorkflowId,
    name: impl Into<String>,
    steps: &[StepConfig],
) -> WorkflowDefinitionV2 {
    let nodes = steps
        .iter()
        .enumerate()
        .map(|(i, step)| migrate_step(step, i))
        .collect();

    let mut edges: Vec<EdgeConfig> = steps
        .windows(2)
        .map(|pair| EdgeConfig {
            from: pair[0].id.clone(),
            to: pair[1].id.clone(),
            when: None,
        })
        .collect();

    // `task_list_from` becomes a real dependency edge. Skip when the
    // source is already the chain predecessor (edge exists), and skip
    // self/dangling references — those are lint findings (P1.4), not
    // something a *total* migration may crash on.
    for (i, step) in steps.iter().enumerate() {
        let Some(source) = step.task_list_from.as_ref().filter(|s| !s.0.is_empty()) else {
            continue;
        };
        let exists = edges.iter().any(|e| e.from == *source && e.to == step.id);
        let dangling = !steps.iter().take(i).any(|s| s.id == *source);
        if !exists && !dangling {
            edges.push(EdgeConfig {
                from: source.clone(),
                to: step.id.clone(),
                when: None,
            });
        }
    }

    WorkflowDefinitionV2 {
        schema_version: WORKFLOW_SCHEMA_V2,
        id,
        name: name.into(),
        nodes,
        edges,
        defaults: WorkflowDefaults::default(),
    }
}

fn migrate_step(step: &StepConfig, index: usize) -> NodeConfig {
    let node_type = if step.is_sequence() {
        // Resolves the superseded `parallel` alias; v2 has no such kind.
        "sequence".to_string()
    } else {
        step.kind.clone()
    };

    let retry = step
        .on_failure
        .as_ref()
        .filter(|t| !t.0.is_empty())
        .map(|target| {
            let rule = RetryRule {
                strategy: RetryStrategy::Redirect,
                max_attempts: step.max_iterations,
                backoff_secs: None,
                feedback: true,
                redirect_to: Some(target.clone()),
            };
            RetryPolicy {
                // v1 routed verdict failures *and* plain agent failures
                // through the same `on_failure` goto — both classes get
                // the rule, mirroring `legacy_policy_for_step`.
                verdict: Some(rule.clone()),
                agent_failure: Some(rule),
                ..Default::default()
            }
        });

    // Everything not lifted into first-class structure stays in the
    // opaque per-type payload. StepConfig always serializes to an object,
    // so the expect can't fire for any real input.
    let mut config = serde_json::to_value(step).expect("StepConfig serializes");
    if let Some(obj) = config.as_object_mut() {
        for field in LIFTED_FIELDS {
            obj.remove(field);
        }
        // Consumed by the retry policy above; only inert author intent
        // (no on_failure to power it) is worth preserving in config.
        if retry.is_some() {
            obj.remove("max_iterations");
        }
    }

    NodeConfig {
        id: step.id.clone(),
        node_type,
        type_version: 1,
        title: step.title.clone(),
        config,
        retry,
        join: None,
        position: Some(Position {
            x: 0.0,
            y: index as f64 * VERTICAL_SPACING,
        }),
    }
}

/// Project a v2 graph back onto the v1 ordered step list (task P3.6).
///
/// This is what keeps `workflow_versions.steps_json` meaningful now that the
/// builder authors v2 documents: the runner, replay, `workflow_list`, and the
/// export path all still read the v1 list, and a version written by this build
/// must stay runnable by a build that has never heard of `definition_json`.
///
/// **Lossy on purpose, and only in the ways v1 cannot express**: node
/// positions, `join`, per-class retry beyond the single `on_failure` redirect,
/// and edge `when` guards have no v1 form. What survives:
///
/// - **Order** is the graph's topological order, so a chain keeps its authored
///   sequence and a branchy graph produces an order the v1 engine can walk.
/// - **`config`** merges straight back onto the step (it *is* a serialized
///   `StepConfig` minus the lifted fields — see [`migrate_step`]).
/// - **`retry.verdict` / `retry.agent_failure` redirect** → `on_failure` +
///   `max_iterations`, the v1 shape it came from.
/// - **A `task_list` dependency** → `task_list_from`, recovered from the
///   incoming edge whose source declares a `task-list` artifact — the same
///   rule the forward migration used to *create* that edge.
///
/// Pure and total: an unreadable `config` yields a step with default fields
/// rather than an error, because refusing to project would make a workflow
/// unsavable over a payload the engine would have ignored anyway.
pub fn project_v2_to_v1(def: &WorkflowDefinitionV2) -> Vec<StepConfig> {
    let order = topological_order(def);
    order
        .into_iter()
        .map(|i| project_node(&def.nodes[i], def))
        .collect()
}

/// Definition-order-stable topological sort. Falls back to definition order
/// for anything left over, so a cyclic document (which lint refuses, but which
/// this total function may still be handed) still projects every node exactly
/// once.
fn topological_order(def: &WorkflowDefinitionV2) -> Vec<usize> {
    let index: std::collections::HashMap<&StepId, usize> = def
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (&n.id, i))
        .collect();
    let mut indegree = vec![0usize; def.nodes.len()];
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); def.nodes.len()];
    for edge in &def.edges {
        if let (Some(&f), Some(&t)) = (index.get(&edge.from), index.get(&edge.to)) {
            out[f].push(t);
            indegree[t] += 1;
        }
    }
    // Definition order breaks ties, so a chain projects back to exactly the
    // list it was migrated from and an unchanged save is a no-op diff.
    let mut ready: Vec<usize> = (0..def.nodes.len()).filter(|i| indegree[*i] == 0).collect();
    let mut order = Vec::with_capacity(def.nodes.len());
    let mut emitted = vec![false; def.nodes.len()];
    while !ready.is_empty() {
        ready.sort_unstable();
        let next = ready.remove(0);
        emitted[next] = true;
        order.push(next);
        for &t in &out[next] {
            indegree[t] -= 1;
            if indegree[t] == 0 {
                ready.push(t);
            }
        }
    }
    for (i, done) in emitted.iter().enumerate() {
        if !done {
            order.push(i);
        }
    }
    order
}

fn project_node(node: &NodeConfig, def: &WorkflowDefinitionV2) -> StepConfig {
    // `config` is a serialized `StepConfig` minus the lifted fields, so
    // putting those back yields the original step. A payload that isn't an
    // object (or won't deserialize) degrades to defaults rather than failing.
    let mut value = match node.config.clone() {
        serde_json::Value::Object(map) => serde_json::Value::Object(map),
        _ => serde_json::Value::Object(Default::default()),
    };
    let obj = value.as_object_mut().expect("object by construction");
    obj.insert("id".into(), serde_json::json!(node.id));
    obj.insert("kind".into(), serde_json::json!(node.node_type));
    obj.insert("title".into(), serde_json::json!(node.title));

    // The redirect rule is where v1's `on_failure` went; bring it back. Either
    // class carries it (the forward migration writes both), so read whichever
    // is present.
    let redirect = node
        .retry
        .as_ref()
        .and_then(|p| p.verdict.as_ref().or(p.agent_failure.as_ref()))
        .filter(|r| r.strategy == RetryStrategy::Redirect);
    if let Some(rule) = redirect {
        if let Some(target) = rule.redirect_to.as_ref() {
            obj.insert("on_failure".into(), serde_json::json!(target));
        }
        if let Some(max) = rule.max_attempts {
            obj.insert("max_iterations".into(), serde_json::json!(max));
        }
    }

    // `task_list_from` is recoverable from the graph: the incoming edge whose
    // source declares the `task-list` artifact. Same rule that created it.
    if let Some(source) = task_list_source(node, def) {
        obj.insert("task_list_from".into(), serde_json::json!(source));
    }

    serde_json::from_value(value).unwrap_or_else(|_| StepConfig {
        id: node.id.clone(),
        kind: node.node_type.clone(),
        title: node.title.clone(),
        ..Default::default()
    })
}

/// The predecessor that feeds `node` a task list, if any.
///
/// Only a `sequence` node can *have* a task-list binding — `task_list_from` is
/// meaningless on every other kind — and restricting it here is load-bearing,
/// not cosmetic: a gate sitting between a planner and its sequence node has an
/// incoming edge from a `task-list` producer too, and without the kind check
/// the gate would come back from the projection carrying a binding its author
/// never wrote (the `refactor` starter is exactly that shape).
///
/// Residual ambiguity, deliberately resolved toward v2: a sequence node whose
/// predecessor declares a `task-list` artifact reads as *bound*, even if the v1
/// step it came from left `task_list_from` unset and used the planner fallback.
/// In v2 the edge **is** the binding (see the module docs), so this is the v2
/// reading of that graph — but it means such a workflow gains an explicit
/// binding the first time it is re-saved through the builder. No bundled
/// starter has that shape; both sequence-bearing starters name their source.
fn task_list_source<'a>(node: &NodeConfig, def: &'a WorkflowDefinitionV2) -> Option<&'a StepId> {
    if node.node_type != "sequence" {
        return None;
    }
    def.edges
        .iter()
        .filter(|e| e.to == node.id)
        .map(|e| &e.from)
        .find(|from| {
            def.nodes
                .iter()
                .find(|n| n.id == **from)
                .is_some_and(declares_task_list)
        })
}

/// Does this node's config declare an artifact named `task-list`?
fn declares_task_list(node: &NodeConfig) -> bool {
    node.config
        .get("artifacts")
        .and_then(|a| a.as_array())
        .is_some_and(|decls| {
            decls
                .iter()
                .any(|d| d.get("name").and_then(|n| n.as_str()) == Some("task-list"))
        })
}

/// Migrate a raw definition document of either schema version.
///
/// - `schema_version: 2` (or already-shaped v2 JSON) deserializes and
///   passes through untouched — the migration is idempotent.
/// - Anything else is treated as the v1 workflow-file shape
///   (`{ id, name, steps: [...] }`, the format `workflow_export` writes
///   and the starters ship).
///
/// Returns a readable error when the document fits neither shape; this is
/// the seam `workflow_import` will call in P1.3+ alongside schema
/// validation.
pub fn migrate_definition(value: &serde_json::Value) -> Result<WorkflowDefinitionV2, String> {
    let version = value.get("schema_version").and_then(|v| v.as_u64());
    if version == Some(WORKFLOW_SCHEMA_V2 as u64) {
        return serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid schema-v2 workflow definition: {e}"));
    }

    // `steps` is the only field a v1 file must carry. `id` and `name` default
    // because the import path mints a fresh workflow id and takes the name
    // from the workflow row anyway — requiring them would reject the
    // hand-written `{ "name": …, "steps": [...] }` files PRD §10 promises keep
    // working, and buy nothing, since `save_definition` overwrites both.
    #[derive(serde::Deserialize)]
    struct V1File {
        #[serde(default)]
        id: WorkflowId,
        #[serde(default)]
        name: String,
        steps: Vec<StepConfig>,
    }
    let v1: V1File = serde_json::from_value(value.clone())
        .map_err(|e| format!("not a v1 workflow definition (needs `steps`): {e}"))?;
    Ok(migrate_v1_to_v2(v1.id, v1.name, &v1.steps))
}

/// The workflow file `workflow_export` writes: the schema-v2 definition with
/// the workflow's `description` beside it.
///
/// `description` lives on the workflow row, not in the graph — the v2 schema
/// has no place for it — so an export that dropped it would lose the
/// workflow's own summary. [`read_import`] reads it back from exactly here,
/// which is what makes export → import a round trip.
pub fn write_export(
    definition: &WorkflowDefinitionV2,
    description: &str,
) -> Result<String, String> {
    let mut export = serde_json::to_value(definition).map_err(|e| e.to_string())?;
    if let Some(obj) = export.as_object_mut() {
        obj.insert("description".into(), serde_json::json!(description));
    }
    serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
}

/// A workflow file read for import: the graph it describes, plus the two
/// fields that graph cannot carry.
#[derive(Debug)]
pub struct ImportedWorkflow {
    pub definition: WorkflowDefinitionV2,
    pub name: String,
    pub description: String,
}

/// Read a workflow file of **either schema version** into something storable
/// (P3.6).
///
/// A v2 document is checked against the published JSON Schema **before**
/// [`migrate_definition`] deserializes it, so a hand-edited file gets located,
/// readable errors instead of a serde message about one missing field
/// somewhere in a hundred-node graph.
///
/// The two fields alongside `definition` are the ones a v2 document has no
/// place for. `name` falls back to a placeholder because a nameless workflow
/// is unfindable in the library; `description` is read from the top level,
/// where `workflow_export` writes it and where a v1 file has always carried
/// it.
pub fn read_import(value: &serde_json::Value) -> Result<ImportedWorkflow, String> {
    if value.get("schema_version").and_then(|v| v.as_u64()) == Some(WORKFLOW_SCHEMA_V2 as u64) {
        validate_workflow_v2(value)
            .map_err(|e| format!("schema-v2 workflow failed validation:\n{e}"))?;
    }
    let definition = migrate_definition(value)?;
    let name = if definition.name.trim().is_empty() {
        "Imported Workflow".to_string()
    } else {
        definition.name.clone()
    };
    Ok(ImportedWorkflow {
        definition,
        name,
        description: value["description"].as_str().unwrap_or("").to_string(),
    })
}

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow_migrate/migrate_tests.rs"]
mod migrate_tests;
