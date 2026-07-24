//! Ready-set scheduler core (task P1.11, PRD §5.3) — **pure**.
//!
//! Given the graph (P1.4), the current per-node states, and a node-output
//! resolver for `when` guards (P1.5), compute which `pending` nodes become
//! `ready` and which become `skipped(reason)`. No I/O, no driver state, no
//! side effects: the driver (P1.12) owns persisting transitions and
//! dispatching work; this module owns *deciding* them.
//!
//! # Node state machine (PRD §5.3, formalized)
//!
//! ```text
//! pending → ready → running → { completed | failed | cancelled | skipped(reason) }
//!                   running → verifying     → { completed | failed }
//!                   running → awaiting_gate → { completed | failed | cancelled }
//!           any-active → interrupted   (watchdog, app restart)
//!           failed → awaiting_retry → ready   (policy-driven, P1.10)
//! ```
//!
//! # Edge / join semantics
//!
//! Each incoming edge is, at any instant, one of:
//!
//! - **satisfied** — source `completed` and the edge's `when` guard (if
//!   any) evaluates true;
//! - **unsatisfiable** — source reached a terminal state that isn't
//!   success (`failed` / `cancelled` / `interrupted` / `skipped`), or the
//!   source completed but the guard evaluated false (or errored);
//! - **undecided** — source not terminal yet.
//!
//! Join over incoming edges (per node, defaulting per decision 39):
//!
//! - `all_success` — ready when every edge is satisfied; skipped as soon
//!   as any edge is unsatisfiable (the classic propagation).
//! - `any_success` — ready as soon as one edge is satisfied (eager);
//!   skipped only when every edge is unsatisfiable.
//! - `all_done` — ready when every edge is decided either way; never
//!   skipped by its join.
//!
//! Roots (no incoming edges) are ready immediately. Skips cascade within
//! one evaluation (fixpoint), so a whole dead branch resolves in a single
//! call rather than one node per tick.
//!
//! # The deadlock invariant
//!
//! In an acyclic graph, if nothing is active and nothing new becomes
//! ready/skipped while `pending` nodes remain, the run can never advance
//! — a bug (the graph lint should have prevented it), not a wait state.
//! [`evaluate_ready_set`] reports it as
//! [`ScheduleError::Deadlock`] so the driver fails loudly (PRD §5.3
//! step 4) instead of idling forever.

use crate::domain::expr::{self, ExprValue};
use crate::domain::ids::StepId;
use crate::domain::models::workflow_v2::{JoinSemantics, WorkflowDefinitionV2};
use crate::domain::workflow_graph::WorkflowGraph;
use std::collections::HashMap;

/// Formalized node run-state (PRD §5.3). `skipped` carries its reason —
/// the UI renders it as the dim-with-tooltip state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeState {
    Pending,
    Ready,
    Running,
    Verifying,
    AwaitingGate,
    AwaitingRetry,
    Interrupted,
    Completed,
    Failed,
    Cancelled,
    Skipped { reason: String },
}

impl NodeState {
    /// Terminal: no further transitions without external intervention.
    /// (`interrupted` is terminal for scheduling — the watchdog's
    /// synthetic gate, not the ready set, revives it.)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            NodeState::Completed
                | NodeState::Failed
                | NodeState::Cancelled
                | NodeState::Interrupted
                | NodeState::Skipped { .. }
        )
    }

    /// Actively occupying the engine or waiting on a human/policy:
    /// the run is alive while any node is in one of these.
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            NodeState::Ready
                | NodeState::Running
                | NodeState::Verifying
                | NodeState::AwaitingGate
                | NodeState::AwaitingRetry
        )
    }

    /// The join's notion of success.
    pub fn is_success(&self) -> bool {
        matches!(self, NodeState::Completed)
    }
}

/// Transitions one evaluation decided: promote `ready` (dispatch order =
/// topological order), mark `skip` with reasons. The driver persists
/// these before acting on them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReadySet {
    pub ready: Vec<StepId>,
    pub skip: Vec<(StepId, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScheduleError {
    /// A node in `states` isn't in the graph or vice versa — driver
    /// bookkeeping bug; refuse to schedule rather than guess.
    UnknownNode(String),
    /// Nothing active, nothing newly ready/skipped, pending nodes remain
    /// (see module docs). Carries the stuck node ids.
    Deadlock(Vec<StepId>),
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::UnknownNode(id) => {
                write!(f, "scheduler state references unknown node '{id}'")
            }
            ScheduleError::Deadlock(nodes) => write!(
                f,
                "empty ready set with non-terminal nodes remaining ({}) — the workflow can \
                 never advance",
                nodes
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// How one incoming edge currently contributes to its target's join.
enum EdgeStanding {
    Satisfied,
    Unsatisfiable(String),
    Undecided,
}

/// Compute the transitions the current instant allows. Pure: `states` is
/// read-only; the skip cascade runs on an internal copy.
///
/// `resolve` supplies `nodes.<id>.outputs.<name>` values for `when`
/// guards; a guard that errors (unknown output, type mismatch) makes its
/// edge unsatisfiable with the error text as the reason — a guard that
/// cannot be evaluated must not silently pass (PRD §5.1).
pub fn evaluate_ready_set(
    def: &WorkflowDefinitionV2,
    graph: &WorkflowGraph,
    states: &HashMap<StepId, NodeState>,
    resolve: &dyn Fn(&str, &str) -> Option<ExprValue>,
) -> Result<ReadySet, ScheduleError> {
    for id in states.keys() {
        if !graph.contains(id) {
            return Err(ScheduleError::UnknownNode(id.to_string()));
        }
    }

    let mut working: HashMap<StepId, NodeState> = states.clone();
    let mut result = ReadySet::default();

    // Fixpoint: a skip decided this pass can decide more next pass.
    // Ready promotions can't cascade (a ready node isn't completed), so
    // only skips loop.
    loop {
        let mut changed = false;

        for node_id in graph.topological_order() {
            // A graph node without a state row hasn't been materialized
            // by the driver yet: not promotable, and (via edge_standing)
            // its outgoing edges stay undecided. If that lag never
            // resolves, the deadlock invariant below reports it.
            let Some(state) = working.get(node_id) else {
                continue;
            };
            if *state != NodeState::Pending {
                continue;
            }

            let node = def
                .nodes
                .iter()
                .find(|n| n.id == *node_id)
                .ok_or_else(|| ScheduleError::UnknownNode(node_id.to_string()))?;
            let join = node
                .join
                .or(def.defaults.join)
                .unwrap_or(JoinSemantics::AllSuccess);

            let incoming: Vec<&crate::domain::models::workflow_v2::EdgeConfig> =
                def.edges.iter().filter(|e| e.to == *node_id).collect();

            let standings: Vec<EdgeStanding> = incoming
                .iter()
                .map(|edge| edge_standing(edge, &working, resolve))
                .collect();

            let satisfied = standings
                .iter()
                .filter(|s| matches!(s, EdgeStanding::Satisfied))
                .count();
            let unsatisfiable: Vec<&String> = standings
                .iter()
                .filter_map(|s| match s {
                    EdgeStanding::Unsatisfiable(reason) => Some(reason),
                    _ => None,
                })
                .collect();
            let decided = satisfied + unsatisfiable.len();

            let decision: Option<Result<(), String>> = match join {
                _ if incoming.is_empty() => Some(Ok(())), // root: ready now
                JoinSemantics::AllSuccess => {
                    if let Some(reason) = unsatisfiable.first() {
                        Some(Err((*reason).clone()))
                    } else if satisfied == incoming.len() {
                        Some(Ok(()))
                    } else {
                        None
                    }
                }
                JoinSemantics::AnySuccess => {
                    if satisfied > 0 {
                        Some(Ok(()))
                    } else if unsatisfiable.len() == incoming.len() {
                        Some(Err(format!(
                            "no dependency succeeded: {}",
                            unsatisfiable
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join("; ")
                        )))
                    } else {
                        None
                    }
                }
                JoinSemantics::AllDone => {
                    if decided == incoming.len() {
                        Some(Ok(()))
                    } else {
                        None
                    }
                }
            };

            match decision {
                Some(Ok(())) => {
                    working.insert(node_id.clone(), NodeState::Ready);
                    result.ready.push(node_id.clone());
                    changed = true;
                }
                Some(Err(reason)) => {
                    working.insert(
                        node_id.clone(),
                        NodeState::Skipped {
                            reason: reason.clone(),
                        },
                    );
                    result.skip.push((node_id.clone(), reason));
                    changed = true;
                }
                None => {}
            }
        }

        if !changed {
            break;
        }
    }

    // Deadlock invariant: nothing active, nothing decided, pending left.
    let pending: Vec<StepId> = working
        .iter()
        .filter(|(_, s)| **s == NodeState::Pending)
        .map(|(id, _)| id.clone())
        .collect();
    let anything_active = working.values().any(NodeState::is_active);
    if !pending.is_empty() && !anything_active && result.ready.is_empty() && result.skip.is_empty()
    {
        let mut stuck = pending;
        stuck.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        return Err(ScheduleError::Deadlock(stuck));
    }

    Ok(result)
}

fn edge_standing(
    edge: &crate::domain::models::workflow_v2::EdgeConfig,
    states: &HashMap<StepId, NodeState>,
    resolve: &dyn Fn(&str, &str) -> Option<ExprValue>,
) -> EdgeStanding {
    let Some(source_state) = states.get(&edge.from) else {
        // Graph construction guarantees edge endpoints exist; a missing
        // state row is driver bookkeeping lag — treat as undecided.
        return EdgeStanding::Undecided;
    };

    if !source_state.is_terminal() {
        return EdgeStanding::Undecided;
    }
    if !source_state.is_success() {
        let why = match source_state {
            NodeState::Failed => format!("dependency '{}' failed", edge.from),
            NodeState::Cancelled => format!("dependency '{}' was cancelled", edge.from),
            NodeState::Interrupted => format!("dependency '{}' was interrupted", edge.from),
            NodeState::Skipped { reason } => {
                format!("dependency '{}' was skipped ({reason})", edge.from)
            }
            _ => unreachable!("terminal non-success states covered above"),
        };
        return EdgeStanding::Unsatisfiable(why);
    }

    match &edge.when {
        None => EdgeStanding::Satisfied,
        Some(guard) => match expr::evaluate(guard, resolve) {
            Ok(true) => EdgeStanding::Satisfied,
            Ok(false) => EdgeStanding::Unsatisfiable(format!(
                "guard on edge '{}' → '{}' evaluated false: {guard}",
                edge.from, edge.to
            )),
            Err(e) => EdgeStanding::Unsatisfiable(format!(
                "guard on edge '{}' → '{}' could not be evaluated: {e}",
                edge.from, edge.to
            )),
        },
    }
}

#[cfg(test)]
#[path = "../../../tests/adapters/step_executor/scheduler_tests.rs"]
mod scheduler_tests;
