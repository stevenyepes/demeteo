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
//! This module is still **unused by the runtime** — wiring happens in
//! P1.12 (driver) and P1.15 (version pinning).

use crate::domain::ids::WorkflowId;
use crate::domain::models::workflow::StepConfig;
use crate::domain::models::workflow_v2::{
    EdgeConfig, NodeConfig, Position, RetryPolicy, RetryRule, RetryStrategy, WorkflowDefaults,
    WorkflowDefinitionV2, WORKFLOW_SCHEMA_V2,
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

    #[derive(serde::Deserialize)]
    struct V1File {
        id: WorkflowId,
        name: String,
        steps: Vec<StepConfig>,
    }
    let v1: V1File = serde_json::from_value(value.clone())
        .map_err(|e| format!("not a v1 workflow definition (id/name/steps): {e}"))?;
    Ok(migrate_v1_to_v2(v1.id, v1.name, &v1.steps))
}

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow_migrate/migrate_tests.rs"]
mod migrate_tests;
