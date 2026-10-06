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
pub mod edit;
pub mod launch;
pub mod release;
pub mod starting;

use std::collections::HashMap;

use serde::Serialize;

use crate::domain::ids::{DiscoveryId, MachineId, TicketId, LOCAL_MACHINE};
use crate::domain::models::{Discovery, Feature, Project, Ticket, TicketState};
use crate::domain::run_placement::{
    default_ticket_placement, placement_for, ticket_placement, ResolvedPlacement, RunPlacement,
};
use crate::domain::ticket_graph::{
    derive_board, TicketNode, TicketNodeState, TicketProgress, TicketStanding,
};
use crate::error::AppError;
use crate::ports::db::FeatureRepository;
use crate::ports::remote_run_mirror::RemoteRunMirror;
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
    /// Where a start with no override would run, resolved here so no surface
    /// re-spells [`crate::domain::run_placement`]'s precedence.
    pub placement: ResolvedPlacement,
}

/// What a started Ticket's current attempt contributes to a card: the verdict
/// the node renders and the link the user follows.
#[derive(Debug, Clone, Serialize)]
pub struct TicketFeatureView {
    pub id: String,
    pub status: String,
    pub mr_state: Option<String>,
    pub mr_url: Option<String>,
    /// Where this attempt was placed, as its start recorded it. `None` when
    /// that is unknown — an attempt predating the record, or a read of it
    /// that failed — and never inferred from [`Self::remote`] being absent:
    /// a detached run the laptop could not mirror has no mirror row either.
    pub placement: Option<RunPlacement>,
    /// The detached run behind this attempt, for display only: the lane is
    /// derived from `mr_state` above, which reconcile hydrates from the
    /// runner, so the board has one source.
    pub remote: Option<TicketRemoteRun>,
}

/// The mirror row of a detached attempt, as of its last reconcile.
#[derive(Debug, Clone, Serialize)]
pub struct TicketRemoteRun {
    pub machine_id: String,
    pub run_id: String,
    pub status: String,
}

/// A Discovery's tickets and the board they derive, from one pass.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryBoard {
    /// In [`Ticket::seq`] order, which the port guarantees.
    pub tickets: Vec<TicketView>,
    pub progress: TicketProgress,
    /// What a ticket left on Default runs on, shipped on its own because a
    /// ticket holding a stored choice cannot say it, and a plan where every
    /// ticket does would otherwise leave Default unnamed.
    pub discovery_default: RunPlacement,
    /// Where a [`RunPlacement::Local`] run executes, for a surface probing the
    /// agents that run will find: the project's compute, which is not the
    /// Discovery's machine when its interview ran elsewhere.
    pub local_host: MachineId,
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
    let (discovery, project) = placement_inputs(ctx, discovery_id).map_err(|e| e.to_string())?;
    let default = default_of(&discovery, project.as_ref());
    let local_host = local_host_of(&discovery, project.as_ref());
    let mut remotes = remote_runs_for(ctx, &attempts);
    let mut placements = attempt_placements(ctx, &tickets);

    let views = tickets
        .into_iter()
        .zip(derived.standings)
        .zip(attempts)
        .map(|((ticket, standing), feature)| TicketView {
            placement: ticket_placement(None, ticket.machine_id.as_ref(), default.clone()),
            ticket,
            standing,
            feature: feature.map(|f| TicketFeatureView {
                placement: placements.remove(f.id.as_str()),
                remote: remotes.remove(f.id.as_str()),
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
        discovery_default: default,
        local_host,
    })
}

/// Where one launch of `ticket` runs: `override_`, else its stored choice,
/// else its Discovery's default — the precedence and the default are
/// [`crate::domain::run_placement`]'s, and this only reads their inputs.
pub fn resolve_ticket_placement(
    ctx: &AppContext,
    ticket: &Ticket,
    override_: Option<&MachineId>,
) -> Result<ResolvedPlacement, AppError> {
    let default = discovery_default(ctx, &ticket.discovery_id)?;
    Ok(ticket_placement(
        override_,
        ticket.machine_id.as_ref(),
        default,
    ))
}

/// A launch places the run on the project's compute, so it refuses a
/// Discovery whose project row is gone; [`board`] instead renders with
/// [`default_of`] over no project.
fn discovery_default(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
) -> Result<RunPlacement, AppError> {
    let (discovery, project) = placement_inputs(ctx, discovery_id)?;
    let project = project.ok_or_else(|| {
        AppError::not_found(format!(
            "project not found: {}",
            discovery.project_id.as_str()
        ))
    })?;
    Ok(default_of(&discovery, Some(&project)))
}

fn placement_inputs(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
) -> Result<(Discovery, Option<Project>), AppError> {
    let discovery = ctx
        .discoveries
        .get(discovery_id)?
        .ok_or_else(|| AppError::not_found(format!("discovery not found: {}", discovery_id.0)))?;
    let project = ctx.projects.get_project(&discovery.project_id)?;
    Ok((discovery, project))
}

fn default_of(discovery: &Discovery, project: Option<&Project>) -> RunPlacement {
    default_ticket_placement(&discovery.machine_id, project.and_then(compute_host))
}

/// With no project row the Discovery's machine stands in, as it does for
/// [`default_of`]: that is where the repository was cloned.
fn local_host_of(discovery: &Discovery, project: Option<&Project>) -> MachineId {
    match project {
        Some(project) => compute_host(project)
            .cloned()
            .unwrap_or_else(|| MachineId::from(LOCAL_MACHINE.to_string())),
        None => discovery.machine_id.clone(),
    }
}

fn compute_host(project: &Project) -> Option<&MachineId> {
    if project.compute_type.eq_ignore_ascii_case("local") {
        return None;
    }
    project.remote_host.as_ref()
}

/// The mirror rows behind `attempts`, or none when they cannot be read.
///
/// Display-only ([`TicketFeatureView::remote`]), so a failed read degrades
/// the chips rather than the board: lanes derive from the Features, which
/// were already read.
fn remote_runs_for(
    ctx: &AppContext,
    attempts: &[Option<Feature>],
) -> HashMap<String, TicketRemoteRun> {
    let feature_ids: Vec<&str> = attempts.iter().flatten().map(|f| f.id.as_str()).collect();
    if feature_ids.is_empty() {
        return HashMap::new();
    }
    match ctx.remote_run_mirror.list_for_features(&feature_ids) {
        Ok(rows) => remote_runs_by_feature(rows),
        Err(error) => {
            tracing::warn!(%error, "board rendered without its remote-run mirror rows");
            HashMap::new()
        }
    }
}

/// Where each started ticket's current attempt was placed, by Feature id.
/// Degrades per ticket, on [`remote_runs_for`]'s terms.
fn attempt_placements(ctx: &AppContext, tickets: &[Ticket]) -> HashMap<String, RunPlacement> {
    let mut placements = HashMap::new();
    for ticket in tickets {
        let Some(feature_id) = &ticket.feature_id else {
            continue;
        };
        let attempts = match ctx.tickets.list_attempts(&ticket.id) {
            Ok(attempts) => attempts,
            Err(error) => {
                tracing::warn!(ticket = %ticket.id.0, %error, "board rendered without an attempt's placement");
                continue;
            }
        };
        let placed = attempts
            .into_iter()
            .find(|a| &a.feature_id == feature_id)
            .and_then(|a| a.machine_id);
        if let Some(machine_id) = placed {
            placements.insert(feature_id.0.clone(), placement_for(&machine_id));
        }
    }
    placements
}

/// `list_for_features` returns the most recently submitted run first, so a
/// feature submitted more than once keeps its latest submit — not whichever
/// run was reconciled last.
fn remote_runs_by_feature(mirror: Vec<RemoteRunMirror>) -> HashMap<String, TicketRemoteRun> {
    let mut by_feature = HashMap::new();
    for row in mirror {
        if let Some(feature_id) = row.feature_id {
            by_feature.entry(feature_id).or_insert(TicketRemoteRun {
                machine_id: row.machine_id,
                run_id: row.run_id,
                status: row.status,
            });
        }
    }
    by_feature
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
        .filter(|t| is_locked(t))
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

/// Whether this Ticket is closed to being changed — §5.3's line, drawn at
/// **has a Feature** and nowhere else.
///
/// That is what leaves a dropped Ticket open: it never got one, and
/// [`diff_proposal`](crate::domain::ticket_graph::diff_proposal) already lets
/// a re-decomposition revise it for the same reason. Refusing the user there
/// would leave hand-editing narrower than the interview it exists to save
/// (§12 #19), and the record §6.6 keeps is the drop reason, which is not an
/// editable field.
///
/// Both spellings of the fact are read and either one locks. They are written
/// together by [`launch::start`], so a row where they disagree is drift — and
/// `repos/ticket.rs` resolves a `state` this build cannot name to `started`
/// deliberately, which consulting `feature_id` alone would quietly undo.
pub fn is_locked(ticket: &Ticket) -> bool {
    ticket.feature_id.is_some() || ticket.state == TicketState::Started
}

fn load(ctx: &AppContext, ticket_id: &TicketId) -> Result<Ticket, String> {
    ctx.tickets
        .get(ticket_id)?
        .ok_or_else(|| format!("ticket not found: {}", ticket_id.0))
}

#[cfg(test)]
#[path = "../../../tests/application/tickets/mod.rs"]
mod tests;
