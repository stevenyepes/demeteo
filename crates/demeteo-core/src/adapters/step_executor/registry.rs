//! `NodeTypeRegistry` — the extensibility seam for workflow node types
//! (PRD §5.2, task P1.6).
//!
//! Replaces the hard-coded `match step_conf.kind` dispatch in
//! `driver.rs` with a registry lookup, mirroring the [`AgentRegistry`]
//! pattern: a flat list of handlers, resolved by exact `kind` string.
//! Adding a node type becomes "implement [`NodeHandler`], add one
//! registration line" — the editor palette, config panels, and lint
//! all derive from the same trait surface (P3.1), and this is the seam
//! a future WASM plugin host plugs into.
//!
//! P1.6 re-homed `agent` and `sync` behind the trait; P1.7 finished
//! the set with `gate`, `sequence`, and `finalize` and deleted the
//! `match`. Handler *bodies* are untouched — each impl delegates to
//! the existing `ExecutionDriver` method, so the P0.2 starter-baseline
//! snapshots must stay byte-identical.
//!
//! [`AgentRegistry`]: crate::adapters::agent::registry::AgentRegistry

use std::sync::{Arc, LazyLock};
use std::time::Instant;

use async_trait::async_trait;

use crate::domain::models::workflow_v2::NodeConfig;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::workflow_graph::{LintFinding, WorkflowGraph};

use super::driver::ExecutionDriver;
use super::steps::StepOutcome;

/// How a node's in-flight work should be interrupted on cancel.
///
/// All five launch node types are [`Graceful`](CancelBehavior::Graceful):
/// their cancellation path today kills the agent session via the
/// registry and lets the driver's cancel handling merge/clean up the
/// worktree. [`Immediate`](CancelBehavior::Immediate) exists for
/// deterministic handlers (the P3.5 `command` node) whose child
/// process can simply be killed — nothing to hand back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Runtime caller lands with the P1.12 driver integration.
pub(crate) enum CancelBehavior {
    /// Signal, then allow the handler's own cleanup to run.
    Graceful,
    /// Hard-kill is safe; the handler holds no state worth a goodbye.
    Immediate,
}

/// Everything a node handler consumes for one dispatch, bundled so the
/// [`NodeHandler::execute`] signature stays stable as handlers migrate
/// behind the trait (P1.6/P1.7) and the ready-set scheduler replaces
/// the linear loop (P1.12).
///
/// The fields mirror the arguments the driver's `run` loop has always
/// passed to its `handle_*_step` methods — this is a re-bundling, not a
/// redesign. `driver` is `&mut` because `gate` (P1.7) mutates
/// `retry_ctx` on redirect-with-feedback; `agent`/`sync` only ever
/// reborrow it shared.
pub(crate) struct NodeCtx<'a> {
    /// The run's shared state + every repository/port handle. Handlers
    /// reach their collaborators through this, exactly as the
    /// `impl ExecutionDriver` methods always have.
    pub driver: &'a mut ExecutionDriver,
    /// The persisted execution row for this step.
    pub step_exec: &'a StepExecution,
    /// The step's definition (v1 model until P1.12 wires v2 through).
    pub step_conf: &'a StepConfig,
    /// Running cost total for this step; the driver reads it back for
    /// the final `update_step_status` and the failure paths.
    pub accumulated_cost: &'a mut f64,
    /// Running token total, same lifecycle as `accumulated_cost`.
    pub accumulated_tokens: &'a mut i64,
    /// When the driver started this step (wall-clock reporting).
    pub step_start: Instant,
    /// Index of this step in the ordered plan (v1 linear semantics;
    /// `RedirectTo` outcomes are expressed against it).
    pub step_index: usize,
    /// Every step-execution row for the feature, in plan order.
    pub step_execs: &'a [StepExecution],
    /// Out-slot: last-seen cache-read tokens for the live cache chip.
    pub out_cache_read: &'a mut Option<u64>,
    /// Out-slot: last-seen cache-creation tokens, peer of
    /// `out_cache_read`.
    pub out_cache_creation: &'a mut Option<u64>,
}

/// One workflow node type: identity, config contract, lint, execution,
/// and cancel semantics (PRD §5.2).
#[async_trait]
pub(crate) trait NodeHandler: Send + Sync {
    /// Registry key (`"agent"`, `"sync"`, …). Exact-match lookup, same
    /// contract as `AgentRuntime::kind`.
    fn kind(&self) -> &'static str;

    /// Superseded kind names this handler also answers to (e.g. the
    /// `sequence` handler owns the retired `"parallel"` alias, so
    /// workflows the user cloned before the rename keep running).
    /// Aliases resolve in [`NodeTypeRegistry::handler_for`] but never
    /// appear in [`NodeTypeRegistry::kinds`] — retired names must not
    /// reach the editor palette (P3.1).
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    /// JSON Schema for this type's `config` payload (the opaque
    /// per-node object in [`NodeConfig::config`]). The editor renders
    /// and validates config forms from this (P3.2); until then it is
    /// documentation with teeth — kept honest by a unit test that
    /// asserts it is a valid schema object.
    #[allow(dead_code)] // First runtime reader is `node_types_list` (P3.1).
    fn config_schema(&self) -> &'static serde_json::Value;

    /// Per-type structural lint, run alongside the graph-level rules in
    /// [`lint_workflow_v2`](crate::domain::workflow_graph::lint_workflow_v2).
    /// Default: nothing type-specific to say. Handlers grow rules here
    /// as the v2 config payloads formalize (P3.x).
    #[allow(dead_code)] // First runtime caller is the `workflow_lint` command (P3.3).
    fn lint(&self, _node: &NodeConfig, _graph: &WorkflowGraph) -> Vec<LintFinding> {
        Vec::new()
    }

    /// Execute one dispatch of this node. Must preserve the exact
    /// behavior of the `ExecutionDriver` method it re-homes — the P0.2
    /// starter-baseline snapshots are the contract.
    async fn execute(&self, ctx: NodeCtx<'_>) -> StepOutcome;

    /// Cancel contract for in-flight work of this type.
    #[allow(dead_code)] // Runtime caller lands with the P1.12 driver integration.
    fn cancel_grace(&self) -> CancelBehavior {
        CancelBehavior::Graceful
    }
}

/// Exact-match lookup table of [`NodeHandler`]s.
///
/// Handlers are stateless delegates (all run state lives on
/// `ExecutionDriver`), so unlike [`AgentRegistry`] — which owns live
/// sessions — one process-wide instance suffices; see [`global`].
///
/// [`AgentRegistry`]: crate::adapters::agent::registry::AgentRegistry
/// [`global`]: NodeTypeRegistry::global
pub(crate) struct NodeTypeRegistry {
    handlers: Vec<Arc<dyn NodeHandler>>,
}

impl NodeTypeRegistry {
    pub(crate) fn new(handlers: Vec<Arc<dyn NodeHandler>>) -> Self {
        Self { handlers }
    }

    /// The process-wide registry of built-in node types — the launch
    /// five, in the [`CORE_NODE_TYPES`] order. A miss here is the
    /// "Unknown step kind" failure, exactly as the old catch-all
    /// `match` arm.
    ///
    /// [`CORE_NODE_TYPES`]: crate::domain::workflow_graph::CORE_NODE_TYPES
    pub(crate) fn global() -> &'static NodeTypeRegistry {
        static GLOBAL: LazyLock<NodeTypeRegistry> = LazyLock::new(|| {
            NodeTypeRegistry::new(vec![
                Arc::new(super::steps::agent::AgentNodeHandler),
                Arc::new(super::steps::gate::GateNodeHandler),
                Arc::new(super::steps::sequence::SequenceNodeHandler),
                Arc::new(super::steps::sync::SyncNodeHandler),
                Arc::new(super::steps::finalize::FinalizeNodeHandler),
            ])
        });
        &GLOBAL
    }

    /// Resolve the handler owning `kind`. Exact match on the canonical
    /// kind or a declared alias; `None` means the definition references
    /// a type this build doesn't ship.
    pub(crate) fn handler_for(&self, kind: &str) -> Option<Arc<dyn NodeHandler>> {
        self.handlers
            .iter()
            .find(|h| h.kind() == kind || h.aliases().contains(&kind))
            .cloned()
    }

    /// Registered kinds, in registration order. Feeds the editor
    /// palette (P3.1) and the "known types" input of
    /// [`lint_workflow_v2`](crate::domain::workflow_graph::lint_workflow_v2).
    #[allow(dead_code)] // First external reader is `node_types_list` (P3.1).
    pub(crate) fn kinds(&self) -> Vec<&'static str> {
        self.handlers.iter().map(|h| h.kind()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every canonical launch kind — must stay in lockstep with
    /// [`crate::domain::workflow_graph::CORE_NODE_TYPES`].
    const ALL_KINDS: [&str; 5] = ["agent", "gate", "sequence", "sync", "finalize"];

    #[test]
    fn global_registry_resolves_all_core_kinds() {
        let reg = NodeTypeRegistry::global();
        for kind in ALL_KINDS {
            let handler = reg
                .handler_for(kind)
                .unwrap_or_else(|| panic!("{kind} must be registered"));
            assert_eq!(handler.kind(), kind);
            assert_eq!(handler.cancel_grace(), CancelBehavior::Graceful);
        }
        // The registry and the graph lint's boundary constant must agree
        // on what a known type is, until P3.1 makes the registry the
        // single authority.
        let mut kinds = NodeTypeRegistry::global().kinds();
        kinds.sort_unstable();
        let mut core = crate::domain::workflow_graph::CORE_NODE_TYPES;
        core.sort_unstable();
        assert_eq!(kinds, core);
    }

    #[test]
    fn parallel_alias_resolves_to_sequence() {
        // `parallel` is the superseded name for `sequence`: workflows the
        // user cloned before the rename keep running, but the alias never
        // shows up as a kind of its own.
        let reg = NodeTypeRegistry::global();
        let handler = reg.handler_for("parallel").expect("alias resolves");
        assert_eq!(handler.kind(), "sequence");
        assert!(!reg.kinds().contains(&"parallel"));
    }

    #[test]
    fn unregistered_kind_is_a_miss() {
        assert!(NodeTypeRegistry::global()
            .handler_for("no-such-kind")
            .is_none());
    }

    #[test]
    fn kinds_lists_registration_order() {
        assert_eq!(
            NodeTypeRegistry::global().kinds(),
            vec!["agent", "gate", "sequence", "sync", "finalize"]
        );
    }

    #[test]
    fn config_schemas_are_valid_schema_objects() {
        for kind in ALL_KINDS {
            let handler = NodeTypeRegistry::global().handler_for(kind).unwrap();
            let schema = handler.config_schema();
            let obj = schema.as_object().expect("schema is a JSON object");
            assert_eq!(
                obj.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "{kind} config schema describes an object payload"
            );
            // Must be a *compilable* JSON Schema, not just any object.
            jsonschema::validator_for(schema)
                .unwrap_or_else(|e| panic!("{kind} config schema must compile: {e}"));
        }
    }

    #[test]
    fn lint_defaults_to_no_findings() {
        use crate::domain::models::workflow_v2::WorkflowDefinitionV2;

        // A minimal single-node definition per kind: the launch
        // handlers add no type-specific rules yet, so lint is empty.
        for kind in ALL_KINDS {
            let def: WorkflowDefinitionV2 = serde_json::from_value(serde_json::json!({
                "schema_version": 2,
                "id": "wf-lint-default",
                "name": "lint default",
                "nodes": [
                    { "id": "n1", "type": kind, "title": "only node", "config": {} }
                ],
                "edges": []
            }))
            .expect("minimal v2 definition parses");
            let graph = WorkflowGraph::build(&def).expect("single node graph builds");
            let handler = NodeTypeRegistry::global().handler_for(kind).unwrap();
            assert!(handler.lint(&def.nodes[0], &graph).is_empty());
        }
    }
}
