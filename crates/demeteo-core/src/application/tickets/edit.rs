//! Hand-editing a Ticket (§5.4 of `docs/PRD_DISCOVERY.md`).
//!
//! §12 #19 chose this over an agent-authored-only plan, so the bar it has to
//! clear is the conversation it replaces: anything the user could get by
//! re-running the interview, they must be able to get here. That decides two
//! things below — a dropped ticket is editable, because a re-decomposition may
//! already revise one ([`crate::domain::ticket_graph::diff_proposal`]), and an
//! edited edge is validated by the same authority an authored one is.
//!
//! It does not widen what a plan may say. §5.2's rule that nothing invalid
//! reaches a stored row is about the row, not about who wrote it.

use serde::Deserialize;

use crate::domain::ids::{TicketId, WorkflowId};
use crate::domain::models::{EffortLevel, Ticket};
use crate::domain::ticket_graph::{validate_ticket_graph, ProposedTicket};
use crate::paths::now_ms;
use crate::ports::discovery::TicketPatch;
use crate::state::AppContext;

use super::{board, is_locked, load, DiscoveryBoard};

/// The whole editable set of one Ticket, as the editor drawer holds it
/// (`DISCOVERY_UI_SPEC.md` §5).
///
/// **Every field is required on the wire, and none of them means "leave this
/// one alone".** The drawer saves the ticket whole, and that is what keeps
/// [`TicketPatch`]'s `Option<Option<T>>` out of the wire: serde reads an
/// absent key and an explicit `null` as the same `None` unless a caller
/// installs a helper for it, so a partial shape would turn *clear the model*
/// into *keep the model* with nothing to show for it. Requiring the key makes
/// a caller that forgot one fail at the boundary instead.
///
/// `seq`, `state`, the drop and force-start reasons and the attachment
/// manifest are absent because none of them is a field of the work: §5.3
/// forbids renumbering, and the rest are written by the acts that record them.
#[derive(Debug, Clone, Deserialize)]
pub struct TicketEdit {
    pub title: String,
    pub description: String,
    pub acceptance: Vec<String>,
    pub files: Vec<String>,
    pub blocked_by: Vec<String>,
    pub test_command: Option<String>,
    pub workflow_id: Option<String>,
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<EffortLevel>,
}

impl TicketEdit {
    /// The edit as it will be stored: trimmed, with the blanks a form leaves
    /// behind removed.
    ///
    /// A field the user cleared arrives as `""`, and an empty string in a
    /// nullable column is a value that reads as a choice — `launch::start` has
    /// to filter a blank workflow id out again before it can refuse a ticket
    /// that has none. Emptied means unset, once, here.
    ///
    /// Edges are deduplicated because `blocked_by` is a set everywhere it is
    /// read: [`crate::domain::ticket_graph::diff_proposal`] compares edges as
    /// one, and a repeat would otherwise show as a revision on the next pass.
    pub fn normalized(&self) -> TicketEdit {
        TicketEdit {
            title: self.title.trim().to_string(),
            description: self.description.trim().to_string(),
            acceptance: entries(&self.acceptance),
            files: entries(&self.files),
            blocked_by: dedup(entries(&self.blocked_by)),
            test_command: chosen(&self.test_command),
            workflow_id: chosen(&self.workflow_id),
            agent_kind: chosen(&self.agent_kind),
            model: chosen(&self.model),
            effort: self.effort,
        }
    }

    /// The patch that writes this edit.
    ///
    /// Every field is `Some`, including the nullable ones: a hand edit states
    /// the whole set, so clearing a column is `Some(None)` and the outer
    /// `None` that [`TicketPatch`] reserves for *leave alone* is never reached
    /// from here.
    fn patch(&self) -> TicketPatch {
        TicketPatch {
            title: Some(self.title.clone()),
            description: Some(self.description.clone()),
            acceptance: Some(self.acceptance.clone()),
            files: Some(self.files.clone()),
            blocked_by: Some(
                self.blocked_by
                    .iter()
                    .cloned()
                    .map(TicketId::from)
                    .collect(),
            ),
            test_command: Some(self.test_command.clone()),
            workflow_id: Some(self.workflow_id.clone().map(WorkflowId::from)),
            agent_kind: Some(self.agent_kind.clone()),
            model: Some(self.model.clone()),
            effort: Some(self.effort),
            ..Default::default()
        }
    }
}

/// Save a hand edit, and hand back the board it leaves behind.
///
/// The board rather than the row: an edited edge moves the standing of every
/// ticket under it, so a caller given the row alone would hold a board that
/// disagrees with it — the disagreement §9.2 exists to prevent, and the reason
/// `discovery_apply_decomposition` returns the same thing.
pub fn update(
    ctx: &AppContext,
    ticket_id: &TicketId,
    edit: &TicketEdit,
) -> Result<DiscoveryBoard, String> {
    let ticket = load(ctx, ticket_id)?;
    let siblings = ctx.tickets.list_for_discovery(&ticket.discovery_id)?;
    let edit = edit.normalized();
    if let Some(refusal) = refusal(&ticket, &siblings, &edit) {
        return Err(refusal);
    }
    ctx.tickets.update(ticket_id, &edit.patch(), now_ms())?;
    board(ctx, &ticket.discovery_id)
}

/// Why this edit may not be saved, or `None` when it may.
///
/// Synchronous and over the rows, so every rule here is reachable from a test
/// with no port doubles — the same terms [`super::deletion_refusal`] and
/// [`super::launch::start_refusal`] are stated in.
///
/// The title is the one field held to a bar of its own. It is what
/// `FeatureLaunch` carries as the run's name and what every list identifies
/// the ticket by, and it is the only editable field with no fallback:
/// `launch::launch_description` already stands in for an empty description.
/// The rest of `validate_ticket_plan`'s content rules are deliberately not
/// applied — they hold an *agent* to writing a ticket worth running, and
/// enforcing them here would refuse a user's save with a message addressed to
/// someone else.
pub fn refusal(ticket: &Ticket, siblings: &[Ticket], edit: &TicketEdit) -> Option<String> {
    if is_locked(ticket) {
        return Some(format!(
            "ticket #{} has already been started, so it can no longer be edited. Its run is \
             working against the plan as it stands; add a follow-up ticket for the change \
             instead.",
            ticket.seq
        ));
    }
    if edit.title.trim().is_empty() {
        return Some(format!(
            "ticket #{} needs a title. It is the name its run is started under and the only way \
             to tell it apart in the plan.",
            ticket.seq
        ));
    }
    graph_refusal(ticket, siblings, &edit.blocked_by)
}

/// Validate the graph the edit would leave, not the edge it changed.
///
/// Adding one edge can complete a cycle that neither end shows on its own, so
/// the whole set goes through [`validate_ticket_graph`] exactly as
/// `decompose::apply` sends the resulting plan through it. Nothing here
/// re-implements any part of that: §6.2's edge scope and §5.2's cycle rule
/// have one authority, and a second reading of them is a second answer waiting
/// to drift.
///
/// The validator is keyed on `#seq` rather than on stored ids. Ids are opaque
/// to it — it only matches edges against them — and its refusals are quoted
/// verbatim to a user, who has never seen a stored id and reads the number on
/// the card. An edge pointing outside the Discovery has no number, so it keeps
/// the raw id and is named as the stranger it is.
fn graph_refusal(ticket: &Ticket, siblings: &[Ticket], edges: &[String]) -> Option<String> {
    let label = |id: &str| match siblings.iter().find(|s| s.id.0 == id) {
        Some(sibling) => format!("#{}", sibling.seq),
        None => id.to_string(),
    };
    if !siblings.iter().any(|sibling| sibling.id.0 == ticket.id.0) {
        return Some(format!("ticket not in its own discovery: {}", ticket.id.0));
    }
    let resulting: Vec<ProposedTicket<()>> = siblings
        .iter()
        .map(|sibling| ProposedTicket {
            id: format!("#{}", sibling.seq),
            blocked_by: if sibling.id.0 == ticket.id.0 {
                edges.iter().map(|id| label(id)).collect()
            } else {
                sibling.blocked_by.iter().map(|id| label(&id.0)).collect()
            },
            body: (),
        })
        .collect();
    validate_ticket_graph(&resulting).map(|reason| format!("this edit cannot be saved: {reason}"))
}

fn entries(items: &[String]) -> Vec<String> {
    items
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn dedup(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

fn chosen(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "../../../tests/application/tickets/edit.rs"]
mod tests;
