//! The text a Ticket's agent is given about the plan around it (§7.2), and the
//! same text the ticket editor previews before anything is started
//! (`docs/DISCOVERY_UI_SPEC.md` §5.8).
//!
//! [`prerequisite_briefing`] decides *what became of* each prerequisite and
//! deliberately stops there; the wording is here because it needs two things
//! that module cannot see — the ticket's own attachments (§9.3) and the number
//! a user says out loud ([`Ticket::seq`]).
//!
//! Synchronous and total over its inputs, so every sentence below is reachable
//! from a test with no port doubles.

use crate::application::agent_probe::model_supports_images_by_name;
use crate::domain::models::{Ticket, TicketState};
use crate::domain::ticket_graph::{
    prerequisite_briefing, PrerequisiteOutcome, TicketNode, TicketNodeState,
};

/// Compose the whole briefing for `ticket`.
///
/// `tickets` and `nodes` are the Discovery's entire set — §6.2 closes the
/// graph over the aggregate, so nothing outside them can be named. `nodes` is
/// the projection of `tickets` that [`crate::application::tickets`] builds;
/// they are not required to be parallel, only to describe the same set.
pub fn compose(ticket: &Ticket, tickets: &[Ticket], nodes: &[TicketNode]) -> String {
    let mut out = String::new();

    if ticket.state == TicketState::Dropped {
        out.push_str("Not started. This ticket was dropped.");
        if let Some(reason) = non_blank(ticket.drop_reason.as_deref()) {
            out.push_str(&format!("\nThe reason recorded was: \"{reason}\""));
        }
        return out;
    }

    let Some(node) = nodes.iter().find(|n| n.id == ticket.id.0) else {
        return "No prerequisites in this discovery.".to_string();
    };

    let notes = prerequisite_briefing(node, nodes);
    if notes.is_empty() {
        out.push_str("No prerequisites in this discovery.");
    } else {
        let lines: Vec<String> = notes
            .iter()
            .map(|note| prerequisite_line(&note.id, note.outcome, tickets, nodes))
            .collect();
        out.push_str(&lines.join("\n"));
    }

    if let Some(block) = attachment_block(ticket) {
        out.push_str("\n\n");
        out.push_str(&block);
    }

    if let Some(block) = bypass_block(ticket, &notes, tickets) {
        out.push_str("\n\n");
        out.push_str(&block);
    }

    out
}

/// How one prerequisite is described.
///
/// [`PrerequisiteOutcome::Outstanding`] splits in two here and only here: a
/// prerequisite with an open PR and one that never started are the same
/// judgement to the graph — neither released anything — but not the same
/// instruction to an agent, which can read an open PR's branch and cannot read
/// work that does not exist.
fn prerequisite_line(
    id: &str,
    outcome: PrerequisiteOutcome,
    tickets: &[Ticket],
    nodes: &[TicketNode],
) -> String {
    let label = label_for(id, tickets);
    match outcome {
        PrerequisiteOutcome::Merged => {
            format!("{label} landed. Its PR merged, so its code is in your base branch.")
        }
        PrerequisiteOutcome::ClosedUnmerged => format!(
            "{label} did not land. Its PR was closed without merging, so none of its work \
             reached your base branch."
        ),
        PrerequisiteOutcome::Dropped => {
            let reason = tickets
                .iter()
                .find(|t| t.id.0 == id)
                .and_then(|t| non_blank(t.drop_reason.as_deref()))
                .map(|r| format!(" The reason recorded was: \"{r}\""))
                .unwrap_or_default();
            format!(
                "{label} did not land. It was dropped from the plan, so none of its work \
                 exists.{reason}"
            )
        }
        PrerequisiteOutcome::Outstanding => {
            let has_open_pr = nodes
                .iter()
                .find(|n| n.id == id)
                .map(|n| n.state == TicketNodeState::Started)
                .unwrap_or(false);
            if has_open_pr {
                format!(
                    "{label} has not landed. It is still running, so none of its work has \
                     reached your base branch."
                )
            } else {
                format!("{label} has not landed. It has not started, so none of its work exists.")
            }
        }
        PrerequisiteOutcome::Unknown => format!(
            "{label} is listed as a prerequisite but is not in this plan. Treat its work as \
             absent."
        ),
    }
}

/// §9.3 routes a Ticket's attachments through the placeholder the agent
/// already understands rather than inventing a second channel for them.
fn attachment_block(ticket: &Ticket) -> Option<String> {
    if ticket.attachments.is_empty() {
        return None;
    }
    let names: Vec<String> = ticket
        .attachments
        .iter()
        .map(|a| format!("[attachment -- {}]", a.name))
        .collect();
    let mut block = format!("Attached: {}", names.join(", "));

    let has_image = ticket
        .attachments
        .iter()
        .any(|a| a.mime.starts_with("image/"));
    let blind = match (ticket.agent_kind.as_deref(), ticket.model.as_deref()) {
        (Some(kind), Some(model)) => !model_supports_images_by_name(kind, model),
        _ => false,
    };
    if has_image && blind {
        block.push_str("\nThe image rides as a path only — this model does not read images.");
    }
    Some(block)
}

/// §6.5's recorded reason, repeated to the agent.
///
/// The bypass is what makes the lines above survivable: every one of them may
/// say the prerequisite is still outstanding, and without this paragraph the
/// agent has no account of why it was started anyway.
fn bypass_block(
    ticket: &Ticket,
    notes: &[crate::domain::ticket_graph::PrerequisiteNote],
    tickets: &[Ticket],
) -> Option<String> {
    let reason = non_blank(ticket.force_start_reason.as_deref())?;
    let outstanding: Vec<String> = notes
        .iter()
        .filter(|n| {
            matches!(
                n.outcome,
                PrerequisiteOutcome::Outstanding | PrerequisiteOutcome::Unknown
            )
        })
        .map(|n| label_for(&n.id, tickets))
        .collect();
    let opening = if outstanding.is_empty() {
        "This ticket was started regardless of its edges, deliberately:".to_string()
    } else {
        format!(
            "This ticket was started before {} landed, deliberately:",
            outstanding.join(" and ")
        )
    };
    Some(format!("{opening}\n\"{reason}\""))
}

/// The number a user says out loud (§5.3), plus enough title to recognise it.
/// Falls back to the raw id for a prerequisite no row carries — the
/// [`PrerequisiteOutcome::Unknown`] case, which has nothing else to name.
fn label_for(id: &str, tickets: &[Ticket]) -> String {
    match tickets.iter().find(|t| t.id.0 == id) {
        Some(t) => format!("#{} \"{}\"", t.seq, t.title),
        None => format!("'{id}'"),
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

#[cfg(test)]
#[path = "../../../tests/application/tickets/briefing.rs"]
mod tests;
