//! Turning a Ticket into a run (§7.1), giving up on one (§6.6), and starting
//! one past its own edges (§6.5).
//!
//! Demeteo shows what is startable and starts nothing by itself (§7.1, §11).
//! Every function here is reached from a user's explicit act; there is no
//! scheduler above them and none is coming.

use crate::domain::ids::TicketId;
use crate::domain::models::{Feature, Ticket, TicketState};
use crate::domain::ticket_graph::{derive_board, BlockerReason, TicketStanding};
use crate::paths::now_ms;
use crate::ports::discovery::TicketPatch;
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

use super::{attachments, briefing, load, nodes_for};

/// Start a Ticket's current attempt.
///
/// The Feature takes the **Ticket's** workflow, agent, model and effort, never
/// the project's defaults: §5.4 lets a plan route a docs ticket and a UI
/// ticket to different harnesses, and inheriting here would quietly undo that.
/// `None` on agent/model/effort still means inherit, because a Ticket that
/// chose nothing has nothing else to fall back to; a missing workflow has no
/// such fallback and is refused.
pub async fn start(ctx: &AppContext, ticket_id: &TicketId) -> Result<Feature, String> {
    let ticket = load(ctx, ticket_id)?;
    let siblings = ctx.tickets.list_for_discovery(&ticket.discovery_id)?;
    let (nodes, _) = nodes_for(&siblings, &*ctx.features)?;
    let derived = derive_board(&nodes);
    let standing = derived
        .standings
        .iter()
        .find(|s| s.id == ticket.id.0)
        .ok_or_else(|| format!("ticket not in its own discovery: {}", ticket.id.0))?;
    if let Some(refusal) = start_refusal(&ticket, standing, &siblings) {
        return Err(refusal);
    }

    let discovery = ctx
        .discoveries
        .get(&ticket.discovery_id)?
        .ok_or_else(|| format!("discovery not found: {}", ticket.discovery_id.0))?;
    let workflow_id = ticket
        .workflow_id
        .as_ref()
        .map(|w| w.0.clone())
        .filter(|w| !w.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "ticket #{} has no workflow. Choose one in the ticket editor before starting it.",
                ticket.seq
            )
        })?;

    let feature = ctx
        .executor
        .feature_start(FeatureLaunch {
            project_id: discovery.project_id.0.clone(),
            workflow_id,
            title: ticket.title.clone(),
            description: launch_description(
                &ticket,
                &briefing::compose(&ticket, &siblings, &nodes),
            ),
            agent_kind: ticket.agent_kind.clone(),
            model: ticket.model.clone(),
            effort: ticket.effort,
            staged_attachments: attachments::staged_for_launch(ctx, &ticket)?,
            ..FeatureLaunch::default()
        })
        .await?;

    let now = now_ms();
    ctx.tickets.supersede_attempts(&ticket.id, now)?;
    ctx.tickets.record_attempt(&ticket.id, &feature.id, now)?;
    ctx.tickets.update(
        &ticket.id,
        &TicketPatch {
            state: Some(TicketState::Started),
            feature_id: Some(Some(feature.id.clone())),
            ..Default::default()
        },
        now,
    )?;
    Ok(feature)
}

/// Record why this Ticket is being started past its edges, then start it
/// (§6.5).
///
/// Per ticket, not per edge: in the case that needs the hatch most — a project
/// with no forge remote, where no dependency will ever have a PR to read —
/// per-edge waivers would have to be granted one at a time, every time.
///
/// The reason is written before the launch is attempted, so a launch that
/// fails for an unrelated cause (no workflow, a busy executor) leaves the
/// decision recorded and the retry a single click. The alternative — write it
/// only on success — loses the reason exactly when the user is most likely to
/// re-answer the prompt differently.
pub async fn force_start(
    ctx: &AppContext,
    ticket_id: &TicketId,
    reason: &str,
) -> Result<Feature, String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(
            "a force start needs a recorded reason: it is what keeps the bypass from \
                    being an unexplained one, for you and for the agent, which is told the same \
                    reason in its own prerequisite list."
                .to_string(),
        );
    }
    let ticket = load(ctx, ticket_id)?;
    if ticket.state != TicketState::Unstarted {
        return Err(format!(
            "ticket #{} is {}, so there is nothing to force.",
            ticket.seq,
            ticket.state.as_str()
        ));
    }
    ctx.tickets.update(
        ticket_id,
        &TicketPatch {
            force_start_reason: Some(Some(reason.to_string())),
            force_started_at: Some(Some(now_ms())),
            ..Default::default()
        },
        now_ms(),
    )?;
    start(ctx, ticket_id).await
}

/// Give up on a Ticket (§6.6).
///
/// This releases everything downstream, exactly as a closed PR does — one rule
/// (§6.4), not a second one beside it. Deleting the row would release them
/// just as well and destroy the record that the option was considered and
/// rejected, which is the thing the interview existed to produce, so the row
/// stays and carries its reason.
///
/// A started Ticket is refused: its run already answers for it through
/// `Feature.mr_state`, and a stored `dropped` on top would hide a live PR
/// behind a lane that claims the plan moved on.
pub fn drop_ticket(ctx: &AppContext, ticket_id: &TicketId, reason: &str) -> Result<Ticket, String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(
            "dropping a ticket needs a reason: the record of the decision is the only \
                    thing that distinguishes it from deleting the ticket."
                .to_string(),
        );
    }
    let ticket = load(ctx, ticket_id)?;
    if ticket.state == TicketState::Started {
        return Err(format!(
            "ticket #{} has already been started. Close or merge its pull request instead — the \
             forge is what releases whatever waits on it.",
            ticket.seq
        ));
    }
    let now = now_ms();
    ctx.tickets.update(
        ticket_id,
        &TicketPatch {
            state: Some(TicketState::Dropped),
            drop_reason: Some(Some(reason.to_string())),
            ..Default::default()
        },
        now,
    )?;
    load(ctx, ticket_id)
}

/// Why this Ticket may not be started, or `None` when it may.
///
/// Reads [`TicketStanding::startable`] rather than re-deriving it — a second
/// opinion on readiness is the drift §6.3 removed the column to avoid.
pub fn start_refusal(
    ticket: &Ticket,
    standing: &TicketStanding,
    tickets: &[Ticket],
) -> Option<String> {
    if standing.startable {
        return None;
    }
    match ticket.state {
        TicketState::Dropped => Some(format!(
            "ticket #{} was dropped from the plan, so there is nothing to start.",
            ticket.seq
        )),
        TicketState::Started => Some(format!("ticket #{} has already been started.", ticket.seq)),
        TicketState::Unstarted => {
            let blockers: Vec<String> = standing
                .blockers
                .iter()
                .map(|b| match b.reason {
                    BlockerReason::Unknown => format!("'{}' (not a ticket in this plan)", b.id),
                    BlockerReason::Outstanding => label_for(&b.id, tickets),
                })
                .collect();
            Some(format!(
                "ticket #{} is blocked by {}. Nothing here starts on its own; force start it with \
                 a recorded reason if you mean to bypass that.",
                ticket.seq,
                blockers.join(", ")
            ))
        }
    }
}

/// The prompt body the run is launched with.
///
/// The briefing is part of it and not an afterthought: §7.2 makes the
/// landed-or-dropped line the difference between an agent that knows its base
/// branch lacks a prerequisite's code and one that assumes it is there.
pub fn launch_description(ticket: &Ticket, briefing: &str) -> String {
    let mut out = ticket.description.trim().to_string();
    if out.is_empty() {
        out.push_str(ticket.title.trim());
    }
    if !ticket.acceptance.is_empty() {
        out.push_str("\n\n## Acceptance criteria\n");
        for item in &ticket.acceptance {
            out.push_str(&format!("- {item}\n"));
        }
        out.truncate(out.trim_end().len());
    }
    if !ticket.files.is_empty() {
        out.push_str("\n\n## Files this is expected to touch\n");
        for item in &ticket.files {
            out.push_str(&format!("- `{item}`\n"));
        }
        out.truncate(out.trim_end().len());
    }
    if let Some(cmd) = ticket
        .test_command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        out.push_str(&format!("\n\n## Verification\n`{cmd}`"));
    }
    out.push_str("\n\n## Prerequisites in this plan\n");
    out.push_str(briefing);
    out
}

fn label_for(id: &str, tickets: &[Ticket]) -> String {
    match tickets.iter().find(|t| t.id.0 == id) {
        Some(t) => format!("#{} \"{}\"", t.seq, t.title),
        None => format!("'{id}'"),
    }
}

#[cfg(test)]
#[path = "../../../tests/application/tickets/launch.rs"]
mod tests;
