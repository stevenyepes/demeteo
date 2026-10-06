//! Turning a Ticket into a run (§7.1), giving up on one (§6.6), and starting
//! one past its own edges (§6.5).
//!
//! Demeteo shows what is startable and starts nothing by itself (§7.1, §11).
//! Every function here is reached from a user's explicit act; there is no
//! scheduler above them and none is coming.

use crate::application::launch::{launch_run, LaunchRequest, LaunchedRun};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{MachineId, TicketId};
use crate::domain::models::{Ticket, TicketState};
use crate::domain::run_placement::DetachedOptions;
use crate::domain::ticket_graph::{derive_board, BlockerReason, TicketStanding};
use crate::error::AppError;
use crate::paths::now_ms;
use crate::ports::discovery::TicketPatch;
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

use super::{attachments, briefing, load, nodes_for, resolve_ticket_placement};

/// Start a Ticket's current attempt.
///
/// The Feature takes the **Ticket's** workflow, agent, model and effort, never
/// the project's defaults: §5.4 lets a plan route a docs ticket and a UI
/// ticket to different harnesses, and inheriting here would quietly undo that.
/// `None` on agent/model/effort still means inherit, because a Ticket that
/// chose nothing has nothing else to fall back to; a missing workflow has no
/// such fallback and is refused.
///
/// `placement_override` applies to this launch only and is never written to
/// the ticket: a one-off "run this one on the runner" must not silently
/// become the ticket's choice for its next attempt. A detached run carries no
/// per-ticket caps — a ticket has no cap fields to read them from.
///
/// Nothing is recorded until the launch succeeds, so a refused one leaves the
/// ticket exactly as startable as it was. A detached run whose PAT was parked,
/// or that the laptop could not mirror, is still a launched run: it is
/// recorded like any other, and its notes go back to the caller on the
/// returned [`LaunchedRun`]. So is a run the ticket itself could not record:
/// once one exists, an `Err` here would read as "nothing started" and invite
/// a second, so the failure becomes
/// [`ticket_unrecorded`](LaunchedRun::ticket_unrecorded) instead.
///
/// The whole body runs under the ticket's [`starting`](super::starting) claim,
/// taken before the ticket is even read: a second start that loaded the row
/// first would judge it on a state the first start is about to overwrite.
pub async fn start(
    ctx: &AppContext,
    ticket_id: &TicketId,
    placement_override: Option<MachineId>,
) -> Result<LaunchedRun, AppError> {
    let Some(_claim) = ctx.ticket_starts.try_claim(ticket_id) else {
        let named = load(ctx, ticket_id)
            .map(|t| format!("#{}", t.seq))
            .unwrap_or_else(|_| ticket_id.0.clone());
        return Err(AppError::validation(format!(
            "ticket {named} is already being started — another start is under way. Wait for it \
             to finish; the board shows it once it has."
        )));
    };
    let ticket = load(ctx, ticket_id)?;
    let siblings = ctx.tickets.list_for_discovery(&ticket.discovery_id)?;
    let (nodes, _) = nodes_for(&siblings, &*ctx.features)?;
    let derived = derive_board(&nodes);
    let standing = derived
        .standings
        .iter()
        .find(|s| s.id == ticket.id.0)
        .ok_or_else(|| {
            AppError::internal(format!("ticket not in its own discovery: {}", ticket.id.0))
        })?;
    if let Some(refusal) = start_refusal(&ticket, standing, &siblings) {
        return Err(AppError::validation(refusal));
    }

    let discovery = ctx.discoveries.get(&ticket.discovery_id)?.ok_or_else(|| {
        AppError::not_found(format!("discovery not found: {}", ticket.discovery_id.0))
    })?;
    let workflow_id = ticket
        .workflow_id
        .as_ref()
        .map(|w| w.0.clone())
        .filter(|w| !w.trim().is_empty())
        .ok_or_else(|| {
            AppError::validation(format!(
                "ticket #{} has no workflow. Choose one in the ticket editor before starting it.",
                ticket.seq
            ))
        })?;
    let resolved = resolve_ticket_placement(ctx, &ticket, placement_override.as_ref())?;

    let launch = FeatureLaunch {
        project_id: discovery.project_id.0.clone(),
        workflow_id,
        title: ticket.title.clone(),
        description: launch_description(&ticket, &briefing::compose(&ticket, &siblings, &nodes)),
        agent_kind: ticket.agent_kind.clone(),
        model: ticket.model.clone(),
        effort: ticket.effort,
        staged_attachments: attachments::staged_for_launch(ctx, &ticket)?,
        origin: discovery
            .base_branch
            .clone()
            .map(|base| FeatureOrigin::Branch { base })
            .unwrap_or_default(),
        ..FeatureLaunch::default()
    };
    let mut launched = launch_run(
        ctx,
        LaunchRequest {
            launch,
            placement: resolved.placement.clone(),
            detached: DetachedOptions::default(),
        },
    )
    .await?;
    if let Err(error) = ctx.tickets.record_start(
        &ticket.id,
        &launched.feature.id,
        &resolved.placement,
        now_ms(),
    ) {
        let feature_id = &launched.feature.id.0;
        tracing::error!(ticket = %ticket.id.0, feature = %feature_id, %error, "ticket run launched but not recorded on the ticket");
        launched.ticket_unrecorded = Some(format!(
            "ticket #{} started Feature {feature_id}, but the ticket could not record it \
             ({error}), so the board still shows it unstarted. The run is going: do not start \
             the ticket again, which would launch a second run.",
            ticket.seq
        ));
    }
    Ok(launched)
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
    placement_override: Option<MachineId>,
) -> Result<LaunchedRun, AppError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(AppError::validation(
            "a force start needs a recorded reason: it is what keeps the bypass from \
                    being an unexplained one, for you and for the agent, which is told the same \
                    reason in its own prerequisite list.",
        ));
    }
    let ticket = load(ctx, ticket_id)?;
    if ticket.state != TicketState::Unstarted {
        return Err(AppError::validation(format!(
            "ticket #{} is {}, so there is nothing to force.",
            ticket.seq,
            ticket.state.as_str()
        )));
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
    start(ctx, ticket_id, placement_override).await
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
    if super::is_locked(&ticket) {
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
