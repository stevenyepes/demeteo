//! The Ticket half of a Discovery: what a screen reads, what a start writes,
//! and what a merge releases (`docs/PRD_DISCOVERY.md` §6, §7).
//!
//! Everything here is a projection or an act. The rules themselves live in
//! [`crate::domain::ticket_graph`], which is synchronous, total, and reads a
//! [`TicketNode`] rather than a row — so this module's whole job on the read
//! side is [`node_of`]. A rule that appears to need writing twice is a rule
//! that belongs there instead.
//!
//! The one thing a persisted Ticket cannot answer on its own is whether its
//! prerequisites landed: that is `Feature.mr_state`, read from the forge and
//! never from the run's own account of itself (§6.4). [`nodes_for`] is where
//! the two rows meet, and it is the only place they do.

pub mod attachments;
pub mod briefing;
pub mod launch;
pub mod release;

use serde::Serialize;

use crate::domain::ids::{DiscoveryId, TicketId};
use crate::domain::models::{Feature, Ticket, TicketState};
use crate::domain::ticket_graph::{
    derive_board, TicketNode, TicketNodeState, TicketProgress, TicketStanding,
};
use crate::ports::db::FeatureRepository;
use crate::state::AppContext;

/// One Ticket as every surface reads it: the row, its derived position, and
/// the forge state the position was derived from.
///
/// The three travel together because §9.2 refuses to let the graph and the
/// board disagree, and two commands returning two halves of one computation is
/// exactly how they would.
#[derive(Debug, Clone, Serialize)]
pub struct TicketView {
    pub ticket: Ticket,
    pub standing: TicketStanding,
    /// `None` when the Ticket has never been started, or when its current
    /// attempt's row has since gone.
    pub feature: Option<TicketFeatureView>,
}

/// What a started Ticket's current attempt contributes to a card: the verdict
/// the node renders and the link the user follows.
#[derive(Debug, Clone, Serialize)]
pub struct TicketFeatureView {
    pub id: String,
    pub status: String,
    pub mr_state: Option<String>,
    pub mr_url: Option<String>,
}

/// A Discovery's tickets and the board they derive, from one pass.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryBoard {
    /// In [`Ticket::seq`] order, which the port guarantees.
    pub tickets: Vec<TicketView>,
    pub progress: TicketProgress,
}

/// Project one stored Ticket onto the node the derived layer reads.
///
/// `mr_state` is the current attempt's, verbatim. `force_started` is derived
/// from the recorded reason rather than stored beside it: §6.5 makes the
/// reason the thing that stops a bypass being unexplained, so a bypass with no
/// reason is not one this can honour.
pub fn node_of(ticket: &Ticket, mr_state: Option<&str>) -> TicketNode {
    TicketNode {
        id: ticket.id.0.clone(),
        state: match ticket.state {
            TicketState::Unstarted => TicketNodeState::Unstarted,
            TicketState::Started => TicketNodeState::Started,
            TicketState::Dropped => TicketNodeState::Dropped,
        },
        blocked_by: ticket.blocked_by.iter().map(|id| id.0.clone()).collect(),
        mr_state: mr_state.map(str::to_string),
        force_started: ticket
            .force_start_reason
            .as_deref()
            .map(str::trim)
            .is_some_and(|reason| !reason.is_empty()),
    }
}

/// Read each Ticket's current attempt and project the whole set.
///
/// Returns the Features alongside, positionally aligned with `tickets`, so a
/// caller rendering a card does not read the same rows again.
pub fn nodes_for(
    tickets: &[Ticket],
    features: &dyn FeatureRepository,
) -> Result<(Vec<TicketNode>, Vec<Option<Feature>>), String> {
    let mut nodes = Vec::with_capacity(tickets.len());
    let mut attempts = Vec::with_capacity(tickets.len());
    for ticket in tickets {
        let feature = match &ticket.feature_id {
            Some(id) => features.get(id)?,
            None => None,
        };
        nodes.push(node_of(
            ticket,
            feature.as_ref().and_then(|f| f.mr_state.as_deref()),
        ));
        attempts.push(feature);
    }
    Ok((nodes, attempts))
}

/// A Discovery's tickets with their derived board (§9.2).
pub fn board(ctx: &AppContext, discovery_id: &DiscoveryId) -> Result<DiscoveryBoard, String> {
    let tickets = ctx.tickets.list_for_discovery(discovery_id)?;
    let (nodes, attempts) = nodes_for(&tickets, &*ctx.features)?;
    let derived = derive_board(&nodes);

    let views = tickets
        .into_iter()
        .zip(derived.standings)
        .zip(attempts)
        .map(|((ticket, standing), feature)| TicketView {
            ticket,
            standing,
            feature: feature.map(|f| TicketFeatureView {
                id: f.id.0,
                status: f.status,
                mr_state: f.mr_state,
                mr_url: f.mr_url,
            }),
        })
        .collect();

    Ok(DiscoveryBoard {
        tickets: views,
        progress: derived.progress,
    })
}

/// The §7.2 briefing for one Ticket, composed against its Discovery's live
/// graph — what the ticket editor previews and what a start would send.
pub fn briefing_for(ctx: &AppContext, ticket_id: &TicketId) -> Result<String, String> {
    let ticket = load(ctx, ticket_id)?;
    let siblings = ctx.tickets.list_for_discovery(&ticket.discovery_id)?;
    let (nodes, _) = nodes_for(&siblings, &*ctx.features)?;
    Ok(briefing::compose(&ticket, &siblings, &nodes))
}

/// Why §8.4 will not delete this Discovery, or `None` when it may go.
///
/// A started Ticket's Feature owns a branch, a worktree and a PR that outlive
/// the plan; cascade-and-detach was rejected because it leaves those branches
/// with no surviving explanation. Kept synchronous and over a slice so the
/// refusal is testable without a database, as `ports/discovery.rs` asks.
pub fn deletion_refusal(tickets: &[Ticket]) -> Option<String> {
    let started: Vec<String> = tickets
        .iter()
        .filter(|t| t.feature_id.is_some())
        .map(|t| format!("#{}", t.seq))
        .collect();
    if started.is_empty() {
        return None;
    }
    Some(format!(
        "this discovery cannot be deleted: {} ({}) {} already been started, and the runs own \
         branches, worktrees and pull requests that outlive the plan. Drop the tickets you have \
         given up on instead.",
        if started.len() == 1 {
            "ticket"
        } else {
            "tickets"
        },
        started.join(", "),
        if started.len() == 1 { "has" } else { "have" },
    ))
}

/// Delete a Discovery, its transcript and its unstarted Tickets (§8.4).
pub fn delete_discovery(ctx: &AppContext, discovery_id: &DiscoveryId) -> Result<(), String> {
    let tickets = ctx.tickets.list_for_discovery(discovery_id)?;
    if let Some(refusal) = deletion_refusal(&tickets) {
        return Err(refusal);
    }
    ctx.discoveries.delete(discovery_id)
}

fn load(ctx: &AppContext, ticket_id: &TicketId) -> Result<Ticket, String> {
    ctx.tickets
        .get(ticket_id)?
        .ok_or_else(|| format!("ticket not found: {}", ticket_id.0))
}

#[cfg(test)]
#[path = "../../../tests/application/tickets/mod.rs"]
mod tests;
