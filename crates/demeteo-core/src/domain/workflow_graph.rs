//! Graph utilities + structural lint for schema-v2 workflows (task P1.4).
//!
//! Shared by the engine (ready-set scheduler, P1.11) and the editor
//! (live lint badges, P3.3). Pure module: no I/O, no engine state.
//!
//! Two layers:
//!
//! - [`WorkflowGraph::build`] — adjacency, topological order, ancestor /
//!   descendant queries. Construction *rejects* structurally unusable
//!   input (duplicate node ids, edges to unknown nodes, cycles) — the
//!   scheduler must never receive a graph these hold for. The rejection
//!   is expressed as [`LintFinding`]s so the editor renders the same
//!   message the engine refuses on.
//! - [`lint_workflow_v2`] — the full rule set: everything `build`
//!   rejects, plus sink shape (finalize count, dead-end branches),
//!   redirect-target-must-be-ancestor, typed-port compatibility,
//!   join/guard interaction, unknown node types, missing prompts. Save is
//!   blocked only by [`LintSeverity::Error`] findings (PRD §6.3); warnings
//!   surface but don't block.
//!
//! One rule is deliberately *not* here: port compatibility for nodes that
//! declare no ports of their own falls back to the node type's defaults,
//! which live on the registry. That half runs in
//! `adapters::step_executor::node_lint`, which can see it; this module owns
//! only what a node's own config states.
//!
//! On the "deadlock detection" rule (PRD §5.3 step 4): in this model the
//! definition graph is acyclic by construction, and an acyclic join can
//! always be *evaluated* — every static deadlock is a cycle, which is the
//! `cycle` error below. The adjacent runtime hazard is the skip cascade:
//! an `all_success` join fed by a guarded edge silently skips whenever the
//! guard fails — flagged as the `guarded-all-success-join` warning so the
//! author chooses `any_success`/`all_done` deliberately.
//!
//! The v1 semantic rules that live on step *config* (verify-capability
//! needs a verifier, a looping judge must attach its spec — see
//! `workflow::lint_workflow_steps`) migrate to the per-node-type
//! `NodeHandler::lint` seam in P1.6, where the config payload has an
//! owner; they are deliberately not duplicated here.

use crate::domain::ids::StepId;
use crate::domain::models::workflow_v2::{
    JoinSemantics, NodeConfig, PortType, RetryStrategy, WorkflowDefinitionV2,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

/// Node types the engine dispatches today. The authority is the
/// `NodeTypeRegistry` (P1.6) — `node_lint::lint_definition` feeds it in
/// from there, and a `registry.rs` test keeps this constant in lockstep;
/// it survives as the pure domain module's default for callers that have
/// no registry handy (tests, boundary code).
pub const CORE_NODE_TYPES: [&str; 6] = ["agent", "gate", "sequence", "sync", "finalize", "command"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LintSeverity {
    /// Structurally broken — save/schedule must refuse.
    Error,
    /// Legal but suspicious — surface, don't block.
    Warning,
}

/// One lint result, addressable to a node or an edge so the editor can
/// badge the offender and the engine can log a precise refusal.
///
/// `Serialize` only, deliberately: `code` is a `&'static str` drawn from
/// this module's fixed vocabulary, so a finding can travel *out* to the
/// builder (the `workflow_lint` command, P3.3) but can never be minted by
/// something outside the crate and handed back as if it were ours. The
/// edge anchor serializes as a `[from, to]` pair.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LintFinding {
    pub severity: LintSeverity,
    /// Stable machine code (`cycle`, `redirect-not-ancestor`, …).
    pub code: &'static str,
    /// The node this finding is anchored to, when node-shaped.
    pub node: Option<StepId>,
    /// The edge this finding is anchored to, when edge-shaped.
    pub edge: Option<(StepId, StepId)>,
    pub message: String,
}

impl LintFinding {
    /// A finding about the definition as a whole, anchored to neither a node
    /// nor an edge — used when the payload can't even be read as a v2
    /// definition, so no rule below can run.
    pub fn workflow_error(code: &'static str, message: String) -> Self {
        Self {
            severity: LintSeverity::Error,
            code,
            node: None,
            edge: None,
            message,
        }
    }

    /// `pub(crate)` so a [`NodeHandler`](crate::adapters::step_executor::registry::NodeHandler)
    /// can anchor its own per-type findings to a node (the `lint` seam,
    /// PRD §5.2) without re-implementing the shape. Still crate-private:
    /// `code` is a `&'static str` from this module's fixed vocabulary.
    pub(crate) fn node_error(code: &'static str, node: &StepId, message: String) -> Self {
        Self {
            severity: LintSeverity::Error,
            code,
            node: Some(node.clone()),
            edge: None,
            message,
        }
    }

    /// Peer of [`node_error`](Self::node_error) for advisory findings —
    /// warnings never block a save (PRD §6.3).
    pub(crate) fn node_warning(code: &'static str, node: &StepId, message: String) -> Self {
        Self {
            severity: LintSeverity::Warning,
            code,
            node: Some(node.clone()),
            edge: None,
            message,
        }
    }

    /// `pub(crate)` for the registry-aware port pass in `node_lint`, which
    /// emits the same `port-type-mismatch` code for the edges this module
    /// cannot judge without the node-type catalog.
    pub(crate) fn edge_error(
        code: &'static str,
        from: &StepId,
        to: &StepId,
        message: String,
    ) -> Self {
        Self {
            severity: LintSeverity::Error,
            code,
            node: None,
            edge: Some((from.clone(), to.clone())),
            message,
        }
    }
}

/// Immutable adjacency + order over a v2 definition. Holds ids only —
/// callers keep the [`WorkflowDefinitionV2`] for node payloads.
#[derive(Debug, Clone)]
pub struct WorkflowGraph {
    ids: Vec<StepId>,
    index: HashMap<StepId, usize>,
    out: Vec<Vec<usize>>,
    inc: Vec<Vec<usize>>,
    /// Indices in a valid topological order (stable: ties broken by
    /// definition order, so linear chains keep their authored order).
    topo: Vec<usize>,
}

impl WorkflowGraph {
    /// Build the graph, rejecting input the scheduler could never walk:
    /// duplicate node ids, edges naming unknown nodes, and cycles
    /// (including self-edges). All violations are returned, not just the
    /// first.
    pub fn build(def: &WorkflowDefinitionV2) -> Result<Self, Vec<LintFinding>> {
        let mut findings = Vec::new();

        let mut index: HashMap<StepId, usize> = HashMap::new();
        for (i, node) in def.nodes.iter().enumerate() {
            if index.insert(node.id.clone(), i).is_some() {
                findings.push(LintFinding::node_error(
                    "duplicate-node-id",
                    &node.id,
                    format!("duplicate node id '{}'", node.id),
                ));
            }
        }

        let n = def.nodes.len();
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut inc: Vec<Vec<usize>> = vec![Vec::new(); n];
        for edge in &def.edges {
            let from = index.get(&edge.from).copied();
            let to = index.get(&edge.to).copied();
            for (id, resolved) in [(&edge.from, from), (&edge.to, to)] {
                if resolved.is_none() {
                    findings.push(LintFinding::edge_error(
                        "edge-unknown-node",
                        &edge.from,
                        &edge.to,
                        format!("edge references unknown node '{id}'"),
                    ));
                }
            }
            if let (Some(f), Some(t)) = (from, to) {
                out[f].push(t);
                inc[t].push(f);
            }
        }

        // Kahn's algorithm; whatever survives with in-degree > 0 is on a
        // cycle (a self-edge is the 1-node case).
        let mut in_degree: Vec<usize> = inc.iter().map(Vec::len).collect();
        let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut topo = Vec::with_capacity(n);
        while let Some(i) = queue.pop_front() {
            topo.push(i);
            for &j in &out[i] {
                in_degree[j] -= 1;
                if in_degree[j] == 0 {
                    queue.push_back(j);
                }
            }
        }
        if topo.len() < n {
            let cyclic: Vec<String> = (0..n)
                .filter(|&i| in_degree[i] > 0)
                .map(|i| def.nodes[i].id.to_string())
                .collect();
            for name in &cyclic {
                findings.push(LintFinding::node_error(
                    "cycle",
                    &StepId::from(name.as_str()),
                    format!(
                        "node '{}' is on a dependency cycle (involving: {}) — the ready set \
                         could never schedule it",
                        name,
                        cyclic.join(", ")
                    ),
                ));
            }
        }

        if !findings.is_empty() {
            return Err(findings);
        }

        Ok(Self {
            ids: def.nodes.iter().map(|s| s.id.clone()).collect(),
            index,
            out,
            inc,
            topo,
        })
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &StepId) -> bool {
        self.index.contains_key(id)
    }

    /// Node ids in a valid topological order (definition order for ties).
    pub fn topological_order(&self) -> Vec<&StepId> {
        self.topo.iter().map(|&i| &self.ids[i]).collect()
    }

    /// Direct successors.
    pub fn successors(&self, id: &StepId) -> Option<Vec<&StepId>> {
        let &i = self.index.get(id)?;
        Some(self.out[i].iter().map(|&j| &self.ids[j]).collect())
    }

    /// Direct predecessors.
    pub fn predecessors(&self, id: &StepId) -> Option<Vec<&StepId>> {
        let &i = self.index.get(id)?;
        Some(self.inc[i].iter().map(|&j| &self.ids[j]).collect())
    }

    /// Every transitive predecessor (excluding `id` itself).
    pub fn ancestors(&self, id: &StepId) -> Option<HashSet<&StepId>> {
        self.closure(id, &self.inc)
    }

    /// Every transitive successor (excluding `id` itself).
    pub fn descendants(&self, id: &StepId) -> Option<HashSet<&StepId>> {
        self.closure(id, &self.out)
    }

    /// True when `ancestor` is a strict ancestor of `of` — the invariant
    /// behind redirect targets and the graph-aware predecessor guard.
    pub fn is_ancestor(&self, ancestor: &StepId, of: &StepId) -> bool {
        self.ancestors(of)
            .is_some_and(|set| set.iter().any(|a| *a == ancestor))
    }

    fn closure(&self, id: &StepId, adj: &[Vec<usize>]) -> Option<HashSet<&StepId>> {
        let &start = self.index.get(id)?;
        let mut seen: HashSet<usize> = HashSet::new();
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            for &j in &adj[i] {
                if seen.insert(j) {
                    stack.push(j);
                }
            }
        }
        seen.remove(&start);
        Some(seen.iter().map(|&i| &self.ids[i]).collect())
    }
}

/// Run the full structural rule set over a definition. Returns every
/// finding; an empty vec means clean. `known_types` is the set of node
/// types the engine can dispatch — pass [`CORE_NODE_TYPES`] until the
/// registry (P1.6) becomes the authority.
pub fn lint_workflow_v2(def: &WorkflowDefinitionV2, known_types: &[&str]) -> Vec<LintFinding> {
    let mut findings = Vec::new();

    // Node-local rules don't need the graph.
    for node in &def.nodes {
        if !known_types.contains(&node.node_type.as_str()) {
            findings.push(LintFinding::node_error(
                "unknown-node-type",
                &node.id,
                format!(
                    "node '{}' has unknown type '{}' (known: {})",
                    node.id,
                    node.node_type,
                    known_types.join(", ")
                ),
            ));
        }

        if node.node_type == "agent" {
            let prompt = node
                .config
                .get("prompt_template")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            if prompt.trim().is_empty() {
                findings.push(LintFinding::node_error(
                    "missing-prompt",
                    &node.id,
                    format!("agent node '{}' has no prompt_template", node.id),
                ));
            }
        }

        for (class, rule) in [
            (
                "environment",
                &node.retry.as_ref().and_then(|r| r.environment.clone()),
            ),
            (
                "verdict",
                &node.retry.as_ref().and_then(|r| r.verdict.clone()),
            ),
            (
                "agent_failure",
                &node.retry.as_ref().and_then(|r| r.agent_failure.clone()),
            ),
            (
                "non_retryable",
                &node.retry.as_ref().and_then(|r| r.non_retryable.clone()),
            ),
        ] {
            let Some(rule) = rule else { continue };
            if rule.strategy == RetryStrategy::Redirect && rule.redirect_to.is_none() {
                findings.push(LintFinding::node_error(
                    "redirect-missing-target",
                    &node.id,
                    format!(
                        "node '{}' retry.{class} uses strategy 'redirect' but names no \
                         redirect_to target",
                        node.id
                    ),
                ));
            }
        }
    }

    let graph = match WorkflowGraph::build(def) {
        Ok(g) => g,
        Err(mut build_findings) => {
            // Without a graph the remaining rules can't run; report what
            // we have.
            findings.append(&mut build_findings);
            return findings;
        }
    };

    // Redirect targets must exist and be strict ancestors — iteration
    // lives in run state; the definition graph stays acyclic.
    for node in &def.nodes {
        let Some(retry) = &node.retry else { continue };
        for (class, rule) in [
            ("environment", &retry.environment),
            ("verdict", &retry.verdict),
            ("agent_failure", &retry.agent_failure),
            ("non_retryable", &retry.non_retryable),
        ] {
            let Some(target) = rule.as_ref().and_then(|r| r.redirect_to.as_ref()) else {
                continue;
            };
            if !graph.contains(target) {
                findings.push(LintFinding::node_error(
                    "redirect-unknown-target",
                    &node.id,
                    format!(
                        "node '{}' retry.{class} redirects to unknown node '{target}'",
                        node.id
                    ),
                ));
            } else if !graph.is_ancestor(target, &node.id) {
                findings.push(LintFinding::node_error(
                    "redirect-not-ancestor",
                    &node.id,
                    format!(
                        "node '{}' retry.{class} redirects to '{target}', which is not an \
                         ancestor — a redirect must re-enter work the node depends on",
                        node.id
                    ),
                ));
            }
        }
    }

    // Finalize sink shape (PRD §5.2: "exactly one sink of type finalize").
    let finalize: Vec<&NodeConfig> = def
        .nodes
        .iter()
        .filter(|n| n.node_type == "finalize")
        .collect();
    match finalize.len() {
        0 => {
            if let Some(first) = def.nodes.first() {
                findings.push(LintFinding::node_warning(
                    "no-finalize",
                    &first.id,
                    "workflow has no finalize node — the run will complete without \
                     squashing/publishing a branch"
                        .to_string(),
                ));
            }
        }
        1 => {
            let f = finalize[0];
            if graph.successors(&f.id).is_some_and(|s| !s.is_empty()) {
                findings.push(LintFinding::node_error(
                    "finalize-not-sink",
                    &f.id,
                    format!(
                        "finalize node '{}' has outgoing edges — nothing may run after the \
                         branch has been squashed and published",
                        f.id
                    ),
                ));
            }
        }
        _ => {
            for f in &finalize {
                findings.push(LintFinding::node_error(
                    "multiple-finalize",
                    &f.id,
                    format!(
                        "workflow has {} finalize nodes; exactly one is allowed (the second \
                         squash would collapse the first and overwrite its summary)",
                        finalize.len()
                    ),
                ));
            }
        }
    }

    // Dead ends: a non-finalize sink, when a finalize exists, ends a
    // branch whose work never reaches the publish path.
    if finalize.len() == 1 {
        for node in &def.nodes {
            if node.node_type != "finalize"
                && graph.successors(&node.id).is_some_and(|s| s.is_empty())
            {
                findings.push(LintFinding::node_warning(
                    "dead-end",
                    &node.id,
                    format!(
                        "node '{}' is a sink but not the finalize node — its results never \
                         flow into the published branch",
                        node.id
                    ),
                ));
            }
        }
    }

    // Typed-port compatibility, checked only where both sides declare
    // ports (`config.outputs` / `config.inputs`); handlers formalize
    // these payloads in P1.6.
    let ports: HashMap<&StepId, (Vec<PortType>, Vec<PortType>)> = def
        .nodes
        .iter()
        .map(|n| {
            (
                &n.id,
                (declared_ports(n, "outputs"), declared_ports(n, "inputs")),
            )
        })
        .collect();
    for edge in &def.edges {
        let (Some((from_outputs, _)), Some((_, to_inputs))) =
            (ports.get(&edge.from), ports.get(&edge.to))
        else {
            continue;
        };
        if from_outputs.is_empty() || to_inputs.is_empty() {
            continue;
        }
        let compatible = from_outputs
            .iter()
            .any(|o| to_inputs.iter().any(|i| o.compatible_with(*i)));
        if !compatible {
            findings.push(LintFinding::edge_error(
                "port-type-mismatch",
                &edge.from,
                &edge.to,
                format!(
                    "edge '{}' → '{}' connects no compatible ports (outputs {:?} vs \
                     inputs {:?})",
                    edge.from, edge.to, from_outputs, to_inputs
                ),
            ));
        }
    }

    // Join/guard interaction: an all_success join fed by a guarded edge
    // silently skips the node whenever the guard fails. Legal — that is
    // the skip-propagation contract — but worth a deliberate choice.
    for node in &def.nodes {
        let effective_join = node
            .join
            .or(def.defaults.join)
            .unwrap_or(JoinSemantics::AllSuccess);
        if effective_join != JoinSemantics::AllSuccess {
            continue;
        }
        let incoming: Vec<_> = def.edges.iter().filter(|e| e.to == node.id).collect();
        if incoming.len() > 1 && incoming.iter().any(|e| e.when.is_some()) {
            findings.push(LintFinding::node_warning(
                "guarded-all-success-join",
                &node.id,
                format!(
                    "node '{}' joins multiple edges with all_success, and at least one edge \
                     is guarded — any failing guard skips this node; use any_success or \
                     all_done if partial paths should still run it",
                    node.id
                ),
            ));
        }
    }

    findings
}

/// Does this finding set block a save? PRD §6.3: *"Save is blocked only by
/// errors, not warnings."* One predicate, shared by the editor's save button
/// (via the `workflow_lint` command) and the write-path guard in
/// `commands::workflows`, so the two can't drift into disagreeing about what
/// "invalid" means.
pub fn has_errors(findings: &[LintFinding]) -> bool {
    findings.iter().any(|f| f.severity == LintSeverity::Error)
}

/// Parse the declared port types under `config.outputs` / `config.inputs`
/// (`[{ "name": …, "type": … }]`). Undeclared or unparseable → empty, and
/// the port check stays silent for that side.
///
/// `pub(crate)` so the registry-aware pass (`node_lint`) reads a node's
/// declaration through *this* function rather than its own copy: the two
/// halves of the port rule have to agree on what "declared" means, or an edge
/// could be judged by both or by neither.
pub(crate) fn declared_ports(node: &NodeConfig, key: &str) -> Vec<PortType> {
    let Some(entries) = node.config.get(key).and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|e| e.get("type"))
        .filter_map(|t| serde_json::from_value::<PortType>(t.clone()).ok())
        .collect()
}

#[cfg(test)]
#[path = "../../tests/domain/workflow_graph/graph_tests.rs"]
mod graph_tests;
