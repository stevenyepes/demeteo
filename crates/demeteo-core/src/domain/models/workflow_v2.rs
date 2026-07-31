//! Workflow-as-data **schema v2**: the true-DAG definition model.
//!
//! PRD: `docs/PRD_DAG_WORKFLOWS.md` §5.1 (shape) and §5.4 (retry policy);
//! decisions 38–42 in `docs/DECISIONS.md` settle the open policy questions.
//!
//! v2 replaces the ordered `Vec<StepConfig>` list (see [`super::workflow`])
//! with explicit `nodes` + forward-dependency `edges`. This module is **pure
//! data + serde**: no engine, repo, or command code references it yet. The
//! v1 → v2 auto-migration lands separately (`workflow_migrate`, task P1.2),
//! JSON-Schema validation at the write boundaries lands in P1.3, and graph
//! semantics (topological order, lint, join evaluation) land in P1.4/P1.11.
//!
//! Serde posture: unknown fields are *tolerated* everywhere (no
//! `deny_unknown_fields`) — machine-checkable rejection of malformed
//! definitions is P1.3's published JSON Schema, enforced at the
//! import/create/update boundaries, not the serde layer. Each node's
//! `config` stays an opaque [`serde_json::Value`]: the per-type payload
//! schema belongs to the node handler (`config_schema()`, task P1.6).

use crate::domain::ids::{StepId, WorkflowId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The schema version this module models. `schema_version: 1` documents
/// (linear step lists) are auto-migrated by `workflow_migrate` (P1.2);
/// they never deserialize into these structs directly.
pub const WORKFLOW_SCHEMA_V2: u32 = 2;

/// A complete v2 workflow definition — the unit stored in
/// `workflow_versions.steps_json` once v2 lands, snapshotted immutably
/// into every run (decision 38 pins `features.workflow_version_id`).
///
/// Scheduling stays a workflow-level concern *outside* this document
/// (decision 41): `WorkflowSchedule` remains on [`super::Workflow`], so the
/// graph schema does not entrench cron and it can move to the Kanban layer
/// later without a schema break.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct WorkflowDefinitionV2 {
    /// Always [`WORKFLOW_SCHEMA_V2`] for documents this module writes.
    pub schema_version: u32,
    /// (Ids are `#[serde(transparent)]` newtypes — plain strings on the wire.)
    #[schemars(with = "String")]
    pub id: WorkflowId,
    pub name: String,
    pub nodes: Vec<NodeConfig>,
    pub edges: Vec<EdgeConfig>,
    /// Workflow-level fallbacks applied when a node doesn't set its own.
    #[serde(default, skip_serializing_if = "WorkflowDefaults::is_empty")]
    pub defaults: WorkflowDefaults,
}

/// Workflow-level defaults (PRD §5.1 `defaults` block).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct WorkflowDefaults {
    /// Default retry policy for nodes that don't declare their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
    /// Default join semantics for nodes with multiple incoming edges.
    /// Engine-wide default when unset here too: `all_success` (decision 39).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<JoinSemantics>,
}

impl WorkflowDefaults {
    pub fn is_empty(&self) -> bool {
        self.retry.is_none() && self.join.is_none()
    }
}

/// One node of the graph.
///
/// Node ids reuse the [`StepId`] vocabulary: the run tables
/// (`step_executions`, and `step_attempts` from P1.8) keep keying rows by
/// these ids, and the v1 → v2 migration maps each `StepConfig.id` onto its
/// node 1:1 — a parallel "NodeId" universe would only invite mix-ups the
/// newtype exists to prevent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct NodeConfig {
    #[schemars(with = "String")]
    pub id: StepId,
    /// Registry key (`agent`, `gate`, `sequence`, `sync`, `finalize`,
    /// `command`, …). Resolved against the `NodeTypeRegistry` (P1.6);
    /// an unknown type is a lint error, not a parse error.
    #[serde(rename = "type")]
    pub node_type: String,
    /// Per-node-type schema evolution without breaking old definitions
    /// (n8n's `typeVersion` pattern). Handlers bump this when their
    /// `config` payload shape changes.
    #[serde(default = "default_type_version")]
    pub type_version: u32,
    pub title: String,
    /// Per-type payload (prompt template, agent/model/effort, capability,
    /// outputs, …). Deliberately untyped here — the owning node handler
    /// publishes its schema via `config_schema()` (P1.6) and the editor
    /// renders/validates from that.
    #[serde(default)]
    pub config: serde_json::Value,
    /// Per-node retry policy; falls back to `defaults.retry`, then the
    /// engine defaults (P1.10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
    /// Join semantics over this node's incoming edges. Unset = workflow
    /// default, then `all_success` (decision 39). Per-node opt-in exists
    /// so e.g. a notification-ish node can fire on `all_done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<JoinSemantics>,
    /// Editor layout, co-persisted with the definition (PRD §5.1). The
    /// v1 migration synthesizes a simple vertical layout; `None` means
    /// "let the editor auto-layout".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
}

fn default_type_version() -> u32 {
    1
}

/// Editor canvas coordinates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// A forward dependency: `to` cannot become ready before `from` reaches a
/// terminal state satisfying `to`'s join semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct EdgeConfig {
    #[schemars(with = "String")]
    pub from: StepId,
    #[schemars(with = "String")]
    pub to: StepId,
    /// Optional guard in the sandboxed expression grammar (P1.5):
    /// `${{ nodes.<id>.outputs.<name> }}` plus comparison operators.
    /// A guard that evaluates false marks the edge unsatisfied and the
    /// downstream node `skipped(reason)` per its join semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

/// When a node with multiple incoming edges becomes ready (PRD §5.1).
/// Default everywhere: `all_success` — with the critic's `PASS_WITH_NOTES`
/// verdict mapping to *success* for join purposes (decision 39).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JoinSemantics {
    AllSuccess,
    AnySuccess,
    AllDone,
}

/// Coarse typed ports (PRD §5.1): checked at connect-time in the editor
/// and lint-time in the engine (P1.4). `Any` is the escape hatch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PortType {
    Text,
    File,
    TaskList,
    Verdict,
    Approval,
    Any,
}

impl PortType {
    /// Connect-time compatibility: equal types always connect, and `Any`
    /// connects with everything on either side.
    pub fn compatible_with(self, other: PortType) -> bool {
        self == PortType::Any || other == PortType::Any || self == other
    }
}

/// Declarative per-failure-class retry policy (PRD §5.4), unifying the
/// v1 scatter of `on_failure` / `max_iterations` / env one-shot retry /
/// engine default. A missing class falls back to `defaults.retry`, then
/// the engine defaults (P1.10 wires evaluation).
///
/// The four classes map 1:1 onto the existing `StepOutcome` /
/// `VerifierError` taxonomy — the engine already *classifies* failures;
/// this makes the *response* declarative.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct RetryPolicy {
    /// Infra/tooling failures (missing deps, transient env) — v1's
    /// one-shot env retry generalized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<RetryRule>,
    /// Verifier verdict failures (failing tests, BLOCKED verdicts) —
    /// v1's `on_failure` redirect loop generalized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<RetryRule>,
    /// The agent process itself failed (crash, non-zero exit, timeout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_failure: Option<RetryRule>,
    /// Failures the engine marks as not worth retrying; the only sane
    /// strategy is [`RetryStrategy::Fail`], but the class is explicit so
    /// a workflow can (say) attach `feedback: false` semantics later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_retryable: Option<RetryRule>,
}

/// What to do when a failure of the keyed class occurs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct RetryRule {
    pub strategy: RetryStrategy,
    /// Attempt budget for this class. `None` = engine default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    /// Delay before the next attempt. `None` = immediate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_secs: Option<u64>,
    /// Carry the failure context into the retried node's prompt — v1's
    /// `RetryContext` append behavior.
    #[serde(default)]
    pub feedback: bool,
    /// Target for [`RetryStrategy::Redirect`]; must be an *ancestor* of
    /// the failing node (lint rule, P1.4 — cycles stay impossible by
    /// construction). Accepts the PRD §5.4 short form `"to"` on input;
    /// serializes as `redirect_to`.
    #[serde(default, alias = "to", skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub redirect_to: Option<StepId>,
}

/// Response strategy for a failure class.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RetryStrategy {
    /// Re-run the same node in place.
    InPlace,
    /// Jump back to an ancestor node (v1 `on_failure` semantics).
    Redirect,
    /// Don't retry; the node (and per join semantics, the run) fails.
    Fail,
}

/// The published JSON Schema for [`WorkflowDefinitionV2`], generated from
/// the structs above so it can never drift from the code. The committed
/// copy lives at `docs-site/workflow-schema-v2.json` (kept in sync by the
/// `published_schema_is_current` test; regen with `UPDATE_SCHEMAS=1`).
pub fn workflow_v2_schema() -> serde_json::Value {
    let mut root = schemars::schema_for!(WorkflowDefinitionV2);
    root.insert(
        "$id".into(),
        "https://demeteo.dev/schemas/workflow-v2.json".into(),
    );
    root.to_value()
}

/// Validate a raw JSON document against the v2 schema. `Ok(())` means the
/// document is schema-valid (structural lint — cycles, dangling refs,
/// port types — is P1.4's `WorkflowGraph`, a separate pass). `Err` carries
/// every violation, one per line, each prefixed with its JSON-pointer
/// location, so boundary callers can surface it verbatim.
pub fn validate_workflow_v2(value: &serde_json::Value) -> Result<(), String> {
    use std::sync::LazyLock;
    static VALIDATOR: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
        jsonschema::validator_for(&workflow_v2_schema())
            .expect("generated workflow v2 schema compiles")
    });

    let errors: Vec<String> = VALIDATOR
        .iter_errors(value)
        .map(|e| {
            let at = e.instance_path.to_string();
            let at = if at.is_empty() { "/".to_string() } else { at };
            format!("at {at}: {e}")
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Whether `definition` and `steps` describe the same set of nodes.
///
/// A workflow version stores both representations, written together, so they
/// agree by construction. A hand-edited row can disagree — and the consequence
/// is not a failed load but a *silently different run*: the definition owns the
/// edges, so a definition that has lost agreement with `steps` schedules a
/// topology the user never drew, and a graph naming nodes the step list does
/// not have fails the whole run at schedule time.
///
/// The caller's fallback when this is `false` is to migrate `steps` instead:
/// `steps` is what the engine can actually execute, so it is the safer
/// authority for a pair that has already lost their agreement.
///
/// Ids only, both directions. The length check is not redundant with the
/// membership one — without it a definition holding a *subset* of the steps
/// (a node deleted from the document but not the list) passes, and the run
/// silently drops a step.
pub fn definition_matches_steps(
    definition: &WorkflowDefinitionV2,
    steps: &[super::workflow::StepConfig],
) -> bool {
    let ids: std::collections::HashSet<&str> = steps.iter().map(|s| s.id.as_str()).collect();
    definition.nodes.len() == steps.len()
        && definition.nodes.iter().all(|n| ids.contains(n.id.as_str()))
}

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow_v2/definition_match_tests.rs"]
mod definition_match_tests;

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow_v2/serde_tests.rs"]
mod serde_tests;

#[cfg(test)]
#[path = "../../../tests/domain/models/workflow_v2/schema_tests.rs"]
mod schema_tests;
