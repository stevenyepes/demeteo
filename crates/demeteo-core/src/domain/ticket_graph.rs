//! Everything a Discovery's ticket set derives from its edges: what satisfies
//! a dependency, what is startable, which lane a ticket sits in, what a
//! re-decomposition may change, and what a started ticket's agent must be told
//! about its prerequisites.
//!
//! Policy only — see this module's parent for why that boundary is drawn
//! where it is. The input is a projection the application layer builds
//! ([`TicketNode`]), never a persisted row: the projection is what keeps every
//! rule here reachable from a test with no port doubles, and it is why a new
//! column on the ticket table is not a change to this file.
//!
//! Nothing derived here is stored. `docs/PRD_DISCOVERY.md` §6.3 settled that
//! against a readiness column and against a column-plus-cache, because a cached
//! derivation drifts the moment its subject changes through a path the updater
//! never watched — a force start, or a PR merged outside Demeteo entirely.
//! Lanes fall out of [`derive_board`] beside readiness for the same reason
//! §9.2 gives: one computation cannot disagree with itself, and two of them
//! eventually would.

use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

/// A ticket's stored state — the entire vocabulary, which §11 pins at three.
///
/// Declared here rather than borrowed from the persistence model so this
/// module keeps compiling against a projection alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketNodeState {
    Unstarted,
    Started,
    Dropped,
}

/// The facts about one ticket that every rule below reads, and no others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketNode {
    pub id: String,
    pub state: TicketNodeState,
    pub blocked_by: Vec<String>,
    /// The current attempt's `Feature.mr_state`, verbatim: `none`, `draft`,
    /// `open`, `merged` or `closed`. `None` when the ticket has no Feature, or
    /// has one that has published nothing.
    ///
    /// Kept as the forge's own word rather than a parsed enum: the vocabulary
    /// is the publisher adapters', it already normalises GitLab's spelling to
    /// GitHub's, and a second enum here would be a place for the two to drift.
    pub mr_state: Option<String>,
    pub force_started: bool,
}

/// Does this ticket release the tickets that depend on it?
///
/// Forge state, never the run's own report of itself (§6.4): a run can finish
/// green without its work reaching the base branch, and a dependent cut from
/// that base would build on nothing. Git ancestry was the other candidate and
/// lost for needing a checkout to answer a question the forge already answers.
///
/// Closed-unmerged releases too, deliberately — the ticket was abandoned and
/// the plan moved on — as does an explicitly dropped ticket (§6.6), which is
/// the same judgement made before a PR ever existed rather than a second rule
/// beside this one. Both leave the dependent building on code that is not in
/// its base branch, which is a lie to the next agent unless it is told:
/// [`prerequisite_briefing`] is where it is told.
pub fn releases_dependents(node: &TicketNode) -> bool {
    match node.state {
        TicketNodeState::Dropped => true,
        _ => matches!(node.mr_state.as_deref(), Some("merged") | Some("closed")),
    }
}

/// Why one unsatisfied edge is still holding its ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerReason {
    /// The prerequisite exists and has neither merged, closed, nor been
    /// dropped. Includes the no-PR case §6.5 refuses to treat as satisfied.
    Outstanding,
    /// No ticket in the set carries this id.
    ///
    /// **A dangling edge blocks.** §6.2 closes the graph over one Discovery
    /// and [`validate_ticket_graph`] rejects an out-of-set edge while the
    /// author is still in context, so an unresolvable id at read time is
    /// drift — a row that went missing — not a plan. Resolving drift in the
    /// ticket's favour would start it on the strength of evidence that is
    /// absent, which is exactly the reading §6.5 rejected when it chose to
    /// keep a dependency with no PR blocked. Reported apart from
    /// [`BlockerReason::Outstanding`] so the surface can say *unknown
    /// prerequisite* rather than *waiting*, and so the force start stays the
    /// one hatch out of both.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Blocker {
    pub id: String,
    pub reason: BlockerReason,
}

/// The board lanes of §9.2, derived from the same pass as readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketLane {
    Blocked,
    Ready,
    InFlight,
    /// The prerequisite's PR merged. Merged only — §9.2 gives a node a check
    /// once its PR merged and a lock while it has not, and a lane that also
    /// held closed-unmerged would put a check on work that is not in the base
    /// branch.
    Landed,
    /// Dropped by the explicit act of §6.6 — **or started and closed
    /// unmerged**.
    ///
    /// This is the one place §6.4 and §9.2 have to be reconciled, and it is
    /// reconciled once, here, rather than per call site. A closed-unmerged
    /// ticket satisfies its dependents (§6.4) yet nothing of it reached the
    /// base branch, so neither of the two obvious homes works: `InFlight`
    /// claims work still in progress and would never empty, and `Landed`
    /// claims work that arrived. §6.4's own words for the case — "the ticket
    /// was abandoned and the plan moved on" — are what this lane means, and
    /// placing it here is what makes §9.2's counter honest: the lane is
    /// excluded from [`TicketProgress::live`], so an abandoned ticket stops
    /// being outstanding work without ever being counted as landed. The
    /// distinction survives where it matters, in
    /// [`PrerequisiteOutcome::ClosedUnmerged`].
    Dropped,
}

/// One ticket's derived position: its lane, whether it may be started now, and
/// what is holding it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TicketStanding {
    pub id: String,
    pub lane: TicketLane,
    /// Unstarted, and either every edge is satisfied or the ticket was
    /// force-started past them (§6.5).
    pub startable: bool,
    /// The unsatisfied prerequisites, listed whatever the ticket's own state —
    /// on a started ticket they are what a force start waived, which is the
    /// record §6.5 asks for and the input [`prerequisite_briefing`] renders.
    pub blockers: Vec<Blocker>,
}

/// Lane counts, plus the denominator §9.2 specifies for the progress bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TicketProgress {
    pub blocked: usize,
    pub ready: usize,
    pub in_flight: usize,
    pub landed: usize,
    pub dropped: usize,
    /// Tickets still expected to produce work: every lane but
    /// [`TicketLane::Dropped`].
    ///
    /// §9.2 counts landed against *live* tickets, since a dropped one is not
    /// work outstanding. Counting it would leave every bar permanently short
    /// of a total it can never reach, with nothing a user could do about it.
    pub live: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TicketBoard {
    /// In the order the nodes were given, so a caller that ordered them by
    /// `seq` keeps the number a user says out loud.
    pub standings: Vec<TicketStanding>,
    pub progress: TicketProgress,
}

/// Read the whole set once: readiness, lanes and the counter together.
///
/// The graph view and the board view are the same answer rendered twice
/// (§9.2), so they are computed once. A ticket that was done on the board and
/// blocked in the graph is not a bug this can have.
pub fn derive_board(nodes: &[TicketNode]) -> TicketBoard {
    let by_id: HashMap<&str, &TicketNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut progress = TicketProgress::default();
    let standings = nodes
        .iter()
        .map(|node| {
            let blockers = blockers_of(node, &by_id);
            let unblocked = blockers.is_empty() || node.force_started;
            let startable = node.state == TicketNodeState::Unstarted && unblocked;
            let lane = match node.state {
                TicketNodeState::Dropped => TicketLane::Dropped,
                TicketNodeState::Started => match node.mr_state.as_deref() {
                    Some("merged") => TicketLane::Landed,
                    Some("closed") => TicketLane::Dropped,
                    _ => TicketLane::InFlight,
                },
                TicketNodeState::Unstarted if startable => TicketLane::Ready,
                TicketNodeState::Unstarted => TicketLane::Blocked,
            };
            match lane {
                TicketLane::Blocked => progress.blocked += 1,
                TicketLane::Ready => progress.ready += 1,
                TicketLane::InFlight => progress.in_flight += 1,
                TicketLane::Landed => progress.landed += 1,
                TicketLane::Dropped => progress.dropped += 1,
            }
            TicketStanding {
                id: node.id.clone(),
                lane,
                startable,
                blockers,
            }
        })
        .collect();
    progress.live = nodes.len() - progress.dropped;
    TicketBoard {
        standings,
        progress,
    }
}

fn blockers_of(node: &TicketNode, by_id: &HashMap<&str, &TicketNode>) -> Vec<Blocker> {
    let mut seen: HashSet<&str> = HashSet::new();
    node.blocked_by
        .iter()
        .filter_map(|dep| {
            let dep = dep.trim();
            if dep.is_empty() || !seen.insert(dep) {
                return None;
            }
            match by_id.get(dep) {
                Some(prerequisite) if releases_dependents(prerequisite) => None,
                Some(_) => Some(Blocker {
                    id: dep.to_string(),
                    reason: BlockerReason::Outstanding,
                }),
                None => Some(Blocker {
                    id: dep.to_string(),
                    reason: BlockerReason::Unknown,
                }),
            }
        })
        .collect()
}

/// What became of one prerequisite, in the terms §7.2 requires the ticket's
/// agent be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrerequisiteOutcome {
    /// Merged: the work is in the base branch, and only here.
    Merged,
    /// The PR was closed without merging. Satisfies (§6.4); delivered nothing.
    ClosedUnmerged,
    /// Dropped before a PR existed (§6.6). Satisfies; delivered nothing.
    Dropped,
    /// Neither landed nor abandoned — reachable only past a force start
    /// (§6.5), which is the case the recorded reason exists for.
    Outstanding,
    /// Names no ticket in the set. See [`BlockerReason::Unknown`].
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrerequisiteNote {
    pub id: String,
    pub outcome: PrerequisiteOutcome,
}

/// Per prerequisite, whether it landed or was abandoned.
///
/// Not polish. §6.4 releases a ticket when its prerequisite's PR was *closed*
/// and §6.6 when the prerequisite was *dropped*; in both cases the plan the
/// agent is reading describes code that its base branch does not contain, and
/// a competent agent told nothing will assume it is there and build on it.
///
/// Structured, not rendered: the prompt builder owns the wording, and §9.3
/// names the ticket's attachments on the same line, which this module cannot
/// see.
pub fn prerequisite_briefing(ticket: &TicketNode, nodes: &[TicketNode]) -> Vec<PrerequisiteNote> {
    let by_id: HashMap<&str, &TicketNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    ticket
        .blocked_by
        .iter()
        .filter_map(|dep| {
            let dep = dep.trim();
            if dep.is_empty() || !seen.insert(dep) {
                return None;
            }
            let outcome = match by_id.get(dep) {
                None => PrerequisiteOutcome::Unknown,
                Some(p) if p.state == TicketNodeState::Dropped => PrerequisiteOutcome::Dropped,
                Some(p) => match p.mr_state.as_deref() {
                    Some("merged") => PrerequisiteOutcome::Merged,
                    Some("closed") => PrerequisiteOutcome::ClosedUnmerged,
                    _ => PrerequisiteOutcome::Outstanding,
                },
            };
            Some(PrerequisiteNote {
                id: dep.to_string(),
                outcome,
            })
        })
        .collect()
}

/// One ticket as a decomposition proposed it.
///
/// `body` is every field the proposal can carry that this module does not
/// read — §5.4's title, description, acceptance, files, workflow, agent, model
/// and effort. It stays opaque and merely comparable on purpose: a revision is
/// "the caller's payload differs from the stored one", so a new editable field
/// is a change to the projection and not to this file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedTicket<B> {
    pub id: String,
    pub blocked_by: Vec<String>,
    pub body: B,
}

/// One ticket as it is stored, for the proposal to be diffed against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentTicket<B> {
    pub id: String,
    pub state: TicketNodeState,
    pub blocked_by: Vec<String>,
    pub body: B,
}

/// Reject a proposed graph that could never be executed, while the agent that
/// wrote it is still in context and can be asked to fix it (§5.2).
///
/// The list crossed a trust boundary on its way out of the agent and into a
/// declared artifact; the same boundary `domain/sequence/tasks.rs` describes
/// for its own validator, and this is that validator's sibling over a general
/// DAG rather than one restricted to list order. Each rule maps to a concrete
/// failure:
///
/// * **empty or duplicate ids** — the id is the edge target *and* the key
///   [`diff_proposal`] matches a re-decomposition on, so §5.3's promise that a
///   started ticket is never renumbered rests on it being unique and stable.
/// * **an edge naming a ticket outside the set** — §6.2 closes the graph over
///   one Discovery, which is what gives the aggregate one ownership rule, one
///   deletion rule, and a bounded set to diff against. Cross-aggregate edges
///   are deferred (§11), not refused on principle.
/// * **a self-edge** — the one-ticket cycle, named separately because the
///   author's mistake is a different one.
/// * **a cycle** — nothing in it can ever start, and every ticket in it is
///   named so the fix does not require re-deriving the loop by hand.
///
/// Returns the reason to hand back to the author, or `None` when the graph is
/// executable. The message is prose the agent can act on and carries no
/// section references: the agent has not read the PRD.
pub fn validate_ticket_graph<B>(proposed: &[ProposedTicket<B>]) -> Option<String> {
    let mut ids: HashSet<&str> = HashSet::with_capacity(proposed.len());
    for (i, ticket) in proposed.iter().enumerate() {
        let id = ticket.id.trim();
        if id.is_empty() {
            return Some(format!(
                "ticket at position {} has an empty `id`. Every ticket needs a unique, stable, \
                 kebab-case id — the dependency edges and every later re-decomposition are keyed \
                 by it.",
                i + 1
            ));
        }
        if !ids.insert(id) {
            return Some(format!(
                "ticket id '{id}' appears more than once. An edge naming it could not say which \
                 of the two it meant."
            ));
        }
    }

    for ticket in proposed {
        let id = ticket.id.trim();
        for dep in &ticket.blocked_by {
            let dep = dep.trim();
            if dep.is_empty() {
                continue;
            }
            if dep == id {
                return Some(format!(
                    "ticket '{id}' lists itself in `blocked_by`. A ticket cannot wait on itself."
                ));
            }
            if !ids.contains(dep) {
                return Some(format!(
                    "ticket '{id}' is blocked_by '{dep}', which is not a ticket in this plan. \
                     Edges may only point at tickets in the same plan; work that depends on \
                     something outside it belongs in the ticket's own description."
                ));
            }
        }
    }

    cycle_reason(proposed)
}

fn cycle_reason<B>(proposed: &[ProposedTicket<B>]) -> Option<String> {
    let index: HashMap<&str, usize> = proposed
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.trim(), i))
        .collect();
    let n = proposed.len();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut waiting_on = vec![0usize; n];
    for (i, ticket) in proposed.iter().enumerate() {
        let mut seen: HashSet<&str> = HashSet::new();
        for dep in &ticket.blocked_by {
            let dep = dep.trim();
            if dep.is_empty() || !seen.insert(dep) {
                continue;
            }
            if let Some(&j) = index.get(dep) {
                dependents[j].push(i);
                waiting_on[i] += 1;
            }
        }
    }

    let mut queue: VecDeque<usize> = (0..n).filter(|&i| waiting_on[i] == 0).collect();
    let mut settled = 0usize;
    while let Some(i) = queue.pop_front() {
        settled += 1;
        for &j in &dependents[i] {
            waiting_on[j] -= 1;
            if waiting_on[j] == 0 {
                queue.push_back(j);
            }
        }
    }
    if settled == n {
        return None;
    }

    let cyclic: Vec<&str> = (0..n)
        .filter(|&i| waiting_on[i] > 0)
        .map(|i| proposed[i].id.trim())
        .collect();
    Some(format!(
        "ticket '{}' is on a dependency cycle (involving: {}). None of them could ever start, \
         since each one waits on another in the same loop.",
        cyclic[0],
        cyclic.join(", ")
    ))
}

/// How a proposal would have changed a ticket it is not allowed to touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImmutableChange {
    Revised,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImmutableViolation {
    pub id: String,
    pub change: ImmutableChange,
    pub reason: String,
}

/// Which tickets a re-decomposition would add, revise, remove, or leave alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TicketDiff {
    pub added: Vec<String>,
    pub revised: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
}

/// Classify a proposal against the stored set, holding started tickets
/// immutable (§5.3).
///
/// Run [`validate_ticket_graph`] over the proposal first — this function
/// answers what would change, not whether the result is executable.
///
/// Three decisions the code cannot recover:
///
/// * **The proposal is the whole plan, so absence is removal.** A
///   re-decomposition emits the set it believes in; treating an unmentioned
///   ticket as merely unmentioned would leave no way to express "drop this"
///   at all, and §5.3 explicitly allows removing an unstarted ticket. The
///   consequence is that a delta-shaped proposal reads as a mass removal,
///   which is why it is a rejection to be shown rather than an application to
///   be performed.
/// * **A started ticket reissued verbatim is `unchanged`, not a violation.**
///   Only a proposal that would actually revise or remove one is refused — a
///   re-decomposition that restates the plan it inherited is the normal path,
///   and refusing it would make §5.3's additive re-run impossible.
/// * **A dropped ticket is treated as an unstarted one.** §5.3 draws
///   immutability at "has a Feature", and a dropped ticket has none; what §6.6
///   preserves is the reason recorded on a ticket that still exists, and
///   whether a proposal may delete it is the user's call in the
///   proposed-changes view, not a rule this module can make for them.
///
/// Edge order is not compared. `blocked_by` is a set — reordering it changes
/// nothing about the graph — so a reordering is not a revision and must not
/// show up in the view as one.
pub fn diff_proposal<B: PartialEq>(
    current: &[CurrentTicket<B>],
    proposed: &[ProposedTicket<B>],
) -> Result<TicketDiff, Vec<ImmutableViolation>> {
    let stored: HashMap<&str, &CurrentTicket<B>> =
        current.iter().map(|t| (t.id.as_str(), t)).collect();
    let offered: HashSet<&str> = proposed.iter().map(|t| t.id.as_str()).collect();

    let mut diff = TicketDiff::default();
    let mut violations = Vec::new();

    for ticket in proposed {
        let Some(stored) = stored.get(ticket.id.as_str()) else {
            diff.added.push(ticket.id.clone());
            continue;
        };
        if stored.body == ticket.body && same_edges(&stored.blocked_by, &ticket.blocked_by) {
            diff.unchanged.push(ticket.id.clone());
        } else if stored.state == TicketNodeState::Started {
            violations.push(ImmutableViolation {
                id: ticket.id.clone(),
                change: ImmutableChange::Revised,
                reason: format!(
                    "ticket '{}' has already been started, so it cannot be revised. Its Feature \
                     is running against the plan as it stands; propose a follow-up ticket for the \
                     change instead.",
                    ticket.id
                ),
            });
        } else {
            diff.revised.push(ticket.id.clone());
        }
    }

    for ticket in current {
        if offered.contains(ticket.id.as_str()) {
            continue;
        }
        if ticket.state == TicketNodeState::Started {
            violations.push(ImmutableViolation {
                id: ticket.id.clone(),
                change: ImmutableChange::Removed,
                reason: format!(
                    "ticket '{}' has already been started, so it cannot be removed. Work exists \
                     against it; leave it in the plan and drop it explicitly if the plan has \
                     moved on.",
                    ticket.id
                ),
            });
            continue;
        }
        diff.removed.push(ticket.id.clone());
    }

    if violations.is_empty() {
        Ok(diff)
    } else {
        Err(violations)
    }
}

fn same_edges(a: &[String], b: &[String]) -> bool {
    fn set(edges: &[String]) -> HashSet<&str> {
        edges
            .iter()
            .map(|e| e.trim())
            .filter(|e| !e.is_empty())
            .collect()
    }
    set(a) == set(b)
}

#[cfg(test)]
#[path = "../../tests/domain/ticket_graph.rs"]
mod tests;
