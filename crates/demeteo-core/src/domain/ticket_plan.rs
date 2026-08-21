//! The ticket list a decomposition writes, and the arithmetic of applying a
//! chosen part of it (§5.2, §5.3 of `docs/PRD_DISCOVERY.md`).
//!
//! Policy only, synchronous and total — see [`crate::domain`] for why that
//! boundary is drawn where it is. The graph rules are
//! [`crate::domain::ticket_graph`]'s and are used from here, never restated:
//! this module owns the *shape* an agent authors and what a partial apply
//! would leave behind, and nothing else.
//!
//! [`ticket_plan_json_shape_example`] is the single source for that shape. The
//! prompt that asks for it and the message that refuses a malformed one both
//! call it, so the two cannot drift; `task_list_json_shape_example` in
//! `crates/demeteo-core/src/domain/sequence/tasks.rs` is the precedent and
//! carries why.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::domain::models::EffortLevel;
use crate::domain::ticket_graph::{
    validate_ticket_graph, CurrentTicket, ProposedTicket, TicketDiff,
};

/// One ticket exactly as a decomposition wrote it — before a workflow name is
/// a workflow id and before an effort word is an [`EffortLevel`].
///
/// Kept unresolved so the whole list survives a round trip through the review
/// modal: the proposal is not persisted anywhere (§5.3 asks for a view, not a
/// second table), so applying it means the surface handing it straight back,
/// and a payload that had already been half-resolved would need the resolution
/// undone to be sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedTicket {
    /// Unique in this plan, and the key every edge and every later
    /// re-decomposition is matched on. For a ticket that already exists this
    /// is the id it is stored under; for a new one it is whatever the agent
    /// authored, until [`apply`](crate::application::discovery::decompose)
    /// mints the stored one.
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    /// A workflow *name*, as the prompt listed it. The agent has no ids.
    #[serde(default)]
    pub workflow: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// Why this ticket is in *this* pass, addressed to the user reviewing it.
    ///
    /// Not a [`Ticket`](crate::domain::models::Ticket) field: §8.1 does not
    /// carry it, and it is about a proposal rather than about the work. It
    /// would be stale the moment a second pass reworded the same ticket for a
    /// different reason.
    #[serde(default)]
    pub why: Option<String>,
}

/// The declared artifact itself.
///
/// `tickets` is required rather than defaulted, which is what makes the
/// tolerant search in [`crate::domain::json_block`] able to tell this object
/// from any other one in the turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketPlan {
    pub tickets: Vec<PlannedTicket>,
}

/// The shape the decomposer is asked to emit, and the shape a refusal quotes
/// back at it.
pub fn ticket_plan_json_shape_example() -> String {
    let fields = [
        r#""id": "revoke-one-client""#,
        r#""title": "...""#,
        r#""description": "...""#,
        r#""acceptance": ["...", "..."]"#,
        r#""files": ["src/foo.rs"]"#,
        r#""test_command": null"#,
        r#""blocked_by": ["client-identity"]"#,
        r#""workflow": "Standard Feature""#,
        r#""agent": null"#,
        r#""model": null"#,
        r#""effort": null"#,
        r#""why": "...""#,
    ];
    format!("{{\"tickets\": [{{{}}}]}}", fields.join(", "))
}

/// Pull the plan out of a decompose turn, wherever in it the agent put it.
pub fn extract_ticket_plan(text: &str) -> Option<TicketPlan> {
    crate::domain::json_block::find_json_block(text, |_: &TicketPlan| true).map(|(_, plan)| plan)
}

/// Refuse a plan the rest of the system could not act on, while the agent that
/// wrote it is still in context (§5.2).
///
/// The graph rules run first and are [`validate_ticket_graph`]'s: ids, edges
/// inside the aggregate, and cycles. What is added here is the part that has
/// nothing to do with the graph — a ticket whose text does not describe work.
/// Both halves answer one reason at a time, so a plan with two faults takes
/// two re-asks; that is what bounds the loop in
/// [`crate::application::discovery::decompose`].
///
/// `description` and `acceptance` are required because of what a ticket
/// becomes: the agent that runs it reads those two fields and the repository,
/// and nothing else from the conversation that produced them. A ticket with no
/// acceptance criteria is one nothing can be held to, and the run that
/// implements it has no way to know it is finished.
pub fn validate_ticket_plan(plan: &TicketPlan) -> Option<String> {
    if plan.tickets.is_empty() {
        return Some(
            "the plan has no tickets. Decomposition has to emit at least one ticket; if the \
             conversation truly settled nothing, say which question is still open instead of \
             emitting an empty plan."
                .into(),
        );
    }
    let graph: Vec<ProposedTicket<()>> = plan
        .tickets
        .iter()
        .map(|t| ProposedTicket {
            id: t.id.clone(),
            blocked_by: t.blocked_by.clone(),
            body: (),
        })
        .collect();
    if let Some(reason) = validate_ticket_graph(&graph) {
        return Some(reason);
    }
    for ticket in &plan.tickets {
        let id = ticket.id.trim();
        if ticket.title.trim().is_empty() {
            return Some(format!("ticket '{id}' has no `title`."));
        }
        if ticket.description.trim().is_empty() {
            return Some(format!(
                "ticket '{id}' has no `description`. The agent that runs it reads the description \
                 and the acceptance criteria and nothing else of this conversation, so a decision \
                 left out here is one it will make again, differently."
            ));
        }
        if ticket.acceptance.iter().all(|c| c.trim().is_empty()) {
            return Some(format!(
                "ticket '{id}' has no `acceptance` criteria. They are what its run is held to and \
                 what tells it that it is finished; a ticket without them cannot be reviewed \
                 against anything."
            ));
        }
    }
    None
}

/// Every field of a ticket that a re-decomposition may change, resolved.
///
/// This is the opaque body [`ProposedTicket`] and [`CurrentTicket`] are
/// generic over, so a revision is exactly "the two bodies differ" and a new
/// editable field is a field added here rather than a rule added to
/// [`crate::domain::ticket_graph`]. Edges are not in it — the diff compares
/// those as a set on their own.
#[derive(Debug, Clone, PartialEq)]
pub struct TicketBody {
    pub title: String,
    pub description: String,
    pub acceptance: Vec<String>,
    pub files: Vec<String>,
    pub test_command: Option<String>,
    pub workflow_id: Option<String>,
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<EffortLevel>,
}

/// One field a revision would change, in the terms the review modal renders:
/// the old value struck through above the new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldChange {
    /// The field's name as a user reads it, not as the struct spells it.
    pub field: String,
    pub was: String,
    pub now: String,
}

/// Which fields differ between the stored ticket and the proposed one.
///
/// Rendered as text on both sides rather than as typed values: the modal shows
/// one row per field with two lines in it, and a caller that had to format
/// nine field types itself would format them differently from the next caller.
pub fn field_changes(
    was: &TicketBody,
    now: &TicketBody,
    was_edges: &[String],
    now_edges: &[String],
) -> Vec<FieldChange> {
    let mut out = Vec::new();
    let mut push = |field: &str, a: String, b: String| {
        if a != b {
            out.push(FieldChange {
                field: field.to_string(),
                was: a,
                now: b,
            });
        }
    };
    push("title", was.title.clone(), now.title.clone());
    push(
        "description",
        was.description.clone(),
        now.description.clone(),
    );
    push(
        "acceptance",
        was.acceptance.join(" · "),
        now.acceptance.join(" · "),
    );
    push("files", was.files.join(", "), now.files.join(", "));
    push(
        "test command",
        opt(&was.test_command),
        opt(&now.test_command),
    );
    push("workflow", opt(&was.workflow_id), opt(&now.workflow_id));
    push("agent", opt(&was.agent_kind), opt(&now.agent_kind));
    push("model", opt(&was.model), opt(&now.model));
    push(
        "effort",
        was.effort
            .map(|e| e.as_str().to_string())
            .unwrap_or_default(),
        now.effort
            .map(|e| e.as_str().to_string())
            .unwrap_or_default(),
    );
    let mut a: Vec<&str> = edge_set(was_edges).into_iter().collect();
    let mut b: Vec<&str> = edge_set(now_edges).into_iter().collect();
    a.sort_unstable();
    b.sort_unstable();
    if a != b {
        out.push(FieldChange {
            field: "blocked by".to_string(),
            was: a.join(", "),
            now: b.join(", "),
        });
    }
    out
}

fn opt(v: &Option<String>) -> String {
    v.clone().unwrap_or_default()
}

fn edge_set(edges: &[String]) -> HashSet<&str> {
    edges
        .iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .collect()
}

/// What applying a chosen subset of a diff would write, delete, and leave.
#[derive(Debug, Clone, PartialEq)]
pub struct Application<B> {
    pub added: Vec<ProposedTicket<B>>,
    pub revised: Vec<ProposedTicket<B>>,
    pub removed: Vec<String>,
    /// The whole plan as it would stand afterwards — the accepted changes
    /// together with every stored ticket they did not touch.
    pub resulting: Vec<ProposedTicket<B>>,
}

/// Resolve a partial apply: which of the diff's changes were picked, and what
/// the plan looks like once only those land.
///
/// `resulting` is the reason this is not a loop at the call site. The review
/// modal lets each change be checked or unchecked independently
/// (`docs/DISCOVERY_UI_SPEC.md` §4.4), and a subset of a valid proposal is not
/// itself valid: declining a new ticket while accepting another that is
/// `blocked_by` it leaves an edge pointing at nothing, and declining a removal
/// while accepting the removal it depended on does the same from the other
/// side. Neither is a mistake the agent made, so neither is a re-ask — they
/// are refusals the user has to see, and they can only be found by running
/// [`validate_ticket_graph`] over the set this returns.
///
/// A change the caller did not accept leaves the stored row exactly as it is,
/// including a stored ticket the proposal never mentioned: absence is removal
/// only where [`diff_proposal`](crate::domain::ticket_graph::diff_proposal)
/// classified it as one, and a removal is still a change to be picked.
pub fn plan_application<B: Clone>(
    current: &[CurrentTicket<B>],
    proposed: &[ProposedTicket<B>],
    diff: &TicketDiff,
    accepted: &[String],
) -> Application<B> {
    let accepted: HashSet<&str> = accepted.iter().map(|id| id.trim()).collect();
    let revised: HashSet<&str> = diff.revised.iter().map(String::as_str).collect();
    let removed: HashSet<&str> = diff.removed.iter().map(String::as_str).collect();
    let added: HashSet<&str> = diff.added.iter().map(String::as_str).collect();
    let by_id: HashMap<&str, &ProposedTicket<B>> =
        proposed.iter().map(|t| (t.id.as_str(), t)).collect();

    let mut out = Application {
        added: Vec::new(),
        revised: Vec::new(),
        removed: Vec::new(),
        resulting: Vec::new(),
    };
    for stored in current {
        let id = stored.id.as_str();
        let picked = accepted.contains(id);
        if picked && removed.contains(id) {
            out.removed.push(stored.id.clone());
            continue;
        }
        if picked && revised.contains(id) {
            if let Some(ticket) = by_id.get(id) {
                out.revised.push((*ticket).clone());
                out.resulting.push((*ticket).clone());
                continue;
            }
        }
        out.resulting.push(ProposedTicket {
            id: stored.id.clone(),
            blocked_by: stored.blocked_by.clone(),
            body: stored.body.clone(),
        });
    }
    for ticket in proposed {
        let id = ticket.id.as_str();
        if added.contains(id) && accepted.contains(id) {
            out.added.push(ticket.clone());
            out.resulting.push(ticket.clone());
        }
    }
    out
}

#[cfg(test)]
#[path = "../../tests/domain/ticket_plan.rs"]
mod tests;
