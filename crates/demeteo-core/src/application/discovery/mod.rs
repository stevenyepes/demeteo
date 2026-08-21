//! Discovery: the planning conversation a Feature can come out of
//! (`docs/PRD_DISCOVERY.md`).
//!
//! The interview is **application logic, not a Workflow** (§4.1): `sequence`,
//! `gate` and the rest form a DAG, and an interview runs an unknown number of
//! rounds, so expressing it there would mean teaching the graph validator to
//! accept a cycle — a structural cost in the engine to model something that is
//! not a pipeline. Only decomposition's *output* touches workflows.
//!
//! The submodules split by what they answer:
//! [`turn`] runs one round, [`context`] decides what the interviewer is told
//! about the project, [`attachments`] holds what the user handed it,
//! [`question`] holds the prompt, [`decompose`] ends the interview in tickets,
//! [`worktree`] owns the checkout it reads in, and [`events`] is what the
//! surface hears.

pub mod attachments;
pub mod context;
pub mod decompose;
pub mod events;
pub mod question;
pub mod turn;
pub mod worktree;

use crate::application::attachments::StagedAttachmentInput;
use crate::domain::discovery_question::{parse_interview_turn, InterviewTurn};
use crate::domain::ids::{DiscoveryId, MachineId, ProjectId};
use crate::domain::models::{
    Discovery, DiscoveryMessage, DiscoveryStatus, EffortLevel, MessageRole,
};
use crate::domain::ticket_graph::TicketProgress;
use crate::ports::discovery::DiscoveryPatch;
use crate::state::AppContext;
use serde::{Deserialize, Serialize};

/// What creating a Discovery needs: the interviewer choice, and whatever the
/// user dropped on the modal before there was a row to hang it on.
///
/// §4.5 puts agent, model, effort **and machine** on the Discovery rather than
/// inheriting the project default: interviewing and implementing want
/// different things from a model, and the same argument reaches the host.
#[derive(Debug, Clone, Deserialize)]
pub struct NewDiscovery {
    pub project_id: String,
    pub title: String,
    pub agent_kind: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<EffortLevel>,
    /// Where the interview runs. `None` takes the project's host, which is
    /// where its repository was cloned — the answer that is right until a user
    /// says otherwise.
    #[serde(default)]
    pub machine_id: Option<String>,
    /// Staged before the Discovery existed, on the terms
    /// [`attachments::stage_batch`] states.
    #[serde(default)]
    pub staged_attachments: Vec<StagedAttachmentInput>,
}

/// One message as a surface reads it: what was said, plus what the reader has
/// to be able to render.
///
/// The question is derived here rather than stored, so a turn and the
/// question it asked can never disagree about each other. Which one is *open*
/// is derived one level further out, by the reader: the last question with no
/// user message after it.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryMessageView {
    #[serde(flatten)]
    pub message: DiscoveryMessage,
    #[serde(flatten)]
    pub turn: InterviewTurn,
}

/// A Discovery and its whole transcript.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryDetail {
    pub discovery: Discovery,
    pub messages: Vec<DiscoveryMessageView>,
}

/// One Discovery as Project Home's list reads it (`DISCOVERY_UI_SPEC.md`
/// §1.5.2): the row, its turn count, and the progress bar over its tickets.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoverySummary {
    #[serde(flatten)]
    pub discovery: Discovery,
    pub message_count: i64,
    /// The same counter [`crate::application::tickets::board`] returns,
    /// derived by the same pass over the same rows.
    ///
    /// It is not a `COUNT(*)` beside the turn count and must never become one:
    /// §6.3 derives a lane from the edges plus the current forge state of each
    /// dependency, so a SQL-shaped second opinion would disagree with the card
    /// the user opens — which is the one thing §9.2 refuses to allow.
    pub progress: TicketProgress,
}

pub fn list_for_project(
    ctx: &AppContext,
    project_id: &str,
) -> Result<Vec<DiscoverySummary>, String> {
    ctx.discoveries
        .list_for_project(&ProjectId::from(project_id.to_string()))?
        .into_iter()
        .map(|row| {
            let tickets = ctx.tickets.list_for_discovery(&row.discovery.id)?;
            let (nodes, _) = crate::application::tickets::nodes_for(&tickets, &*ctx.features)?;
            Ok(DiscoverySummary {
                discovery: row.discovery,
                message_count: row.message_count,
                progress: crate::domain::ticket_graph::derive_board(&nodes).progress,
            })
        })
        .collect()
}

pub fn get(ctx: &AppContext, id: &DiscoveryId) -> Result<DiscoveryDetail, String> {
    let discovery = load(ctx, id)?;
    let messages = ctx
        .discoveries
        .list_messages(id)?
        .into_iter()
        .map(|message| DiscoveryMessageView {
            turn: match message.role {
                MessageRole::Assistant => parse_interview_turn(&message.content),
                MessageRole::User => InterviewTurn {
                    prose: message.content.clone(),
                    ..Default::default()
                },
            },
            message,
        })
        .collect();
    Ok(DiscoveryDetail {
        discovery,
        messages,
    })
}

/// Open a Discovery. No worktree and no agent process yet — both wait for the
/// first turn that needs them (§4.6).
///
/// The chosen machine is checked here rather than at the first turn: it is the
/// last moment the user is still looking at the picker they set it with, and
/// `worktree::resolve` reaches it three screens later with nothing to say
/// except that a checkout was not found.
pub fn create(ctx: &AppContext, new: NewDiscovery) -> Result<Discovery, String> {
    let title = new.title.trim();
    if title.is_empty() {
        return Err("A discovery needs a title.".into());
    }
    let project_id = ProjectId::from(new.project_id);
    let project = ctx
        .projects
        .get_projects()?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("Project not found: {}", project_id.as_str()))?;
    let machine_id = crate::domain::discovery_host::interviewer_machine(
        new.machine_id.as_deref(),
        project.compute_type.eq_ignore_ascii_case("local"),
        project.remote_host.as_ref().map(MachineId::as_str),
    )?;
    refuse_unknown_machine(ctx, &machine_id)?;

    let now = crate::paths::now_ms();
    let discovery = Discovery {
        id: DiscoveryId::from(crate::shared::ids::new_id()),
        project_id,
        title: title.to_string(),
        status: DiscoveryStatus::Open,
        machine_id,
        agent_kind: new.agent_kind,
        model: new.model,
        effort: new.effort,
        resume_session_id: None,
        worktree_path: None,
        attachments: Vec::new(),
        total_cost: 0.0,
        tokens: 0,
        created_at: now,
        updated_at: now,
    };
    ctx.discoveries.create(&discovery)?;
    attachments::stage_batch(ctx, &discovery.id, new.staged_attachments)?;
    load(ctx, &discovery.id)
}

/// Refuse a machine nothing is configured for.
///
/// The desktop host is accepted without a lookup because by policy it has no
/// `machines` row (V38), so the absence of one says nothing about it.
fn refuse_unknown_machine(ctx: &AppContext, machine_id: &MachineId) -> Result<(), String> {
    if machine_id.is_local() || ctx.machines.get_machine(machine_id)?.is_some() {
        return Ok(());
    }
    Err(format!(
        "No machine is configured as '{}'. Add it under Machines, or leave the interviewer on \
         this project's own host.",
        machine_id.as_str()
    ))
}

/// End the interview without ending anything else (§8.4).
///
/// The checkout goes back here rather than waiting for the idle sweep: a
/// closed Discovery has no next turn to recreate it for.
pub async fn close(ctx: &AppContext, id: &DiscoveryId) -> Result<(), String> {
    let discovery = load(ctx, id)?;
    ctx.registry.kill(&turn::thread_id(id)).await;
    if let Err(e) = worktree::reclaim(ctx, &discovery).await {
        tracing::warn!(discovery = %id.as_str(), error = %e, "discovery: close could not reclaim the worktree");
    }
    ctx.discoveries.update(
        id,
        &DiscoveryPatch {
            status: Some(DiscoveryStatus::Closed),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )
}

/// Reopen a closed Discovery. §8.3 makes staying open the normal state, so
/// closing has to be undoable — nothing was destroyed to make it reversible.
pub fn reopen(ctx: &AppContext, id: &DiscoveryId) -> Result<(), String> {
    ctx.discoveries.update(
        id,
        &DiscoveryPatch {
            status: Some(DiscoveryStatus::Open),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )
}

/// Delete a Discovery, its transcript and its unstarted Tickets.
///
/// The refusal §8.4 asks for is
/// [`tickets::deletion_refusal`](crate::application::tickets::deletion_refusal),
/// which carries why. What this adds is everything a row delete cannot reach:
/// the live session, the checkout it was reading in, and the attachment bytes
/// — which live on disk keyed by owner id, so the `ON DELETE CASCADE` that
/// takes the tickets takes no file with it.
pub async fn delete(ctx: &AppContext, id: &DiscoveryId) -> Result<(), String> {
    let discovery = load(ctx, id)?;
    let tickets = ctx.tickets.list_for_discovery(id)?;
    if let Some(refusal) = crate::application::tickets::deletion_refusal(&tickets) {
        return Err(refusal);
    }
    ctx.registry.kill(&turn::thread_id(id)).await;
    if let Err(e) = worktree::reclaim(ctx, &discovery).await {
        tracing::warn!(discovery = %id.as_str(), error = %e, "discovery: delete could not reclaim the worktree");
    }
    for owner in std::iter::once(id.as_str()).chain(tickets.iter().map(|t| t.id.as_str())) {
        if let Err(e) = ctx.attachments.clear_feature(owner) {
            tracing::warn!(owner, error = %e, "discovery: delete could not drop the attachment bytes");
        }
    }
    ctx.discoveries.delete(id)
}

/// Stop the turn in flight.
///
/// The same shape as `commands::agent_lifecycle::agent_cancel`: the child is
/// killed and the stream ends on its own. What it spent is still billed and
/// whatever it managed to say is still stored, because both already happened.
pub async fn cancel_turn(ctx: &AppContext, id: &DiscoveryId) -> Result<(), String> {
    if let Some(session) = ctx.registry.session_handle_any(&turn::thread_id(id)).await {
        session.cancel()?;
    }
    Ok(())
}

pub(super) fn load(ctx: &AppContext, id: &DiscoveryId) -> Result<Discovery, String> {
    ctx.discoveries
        .get(id)?
        .ok_or_else(|| format!("Discovery not found: {}", id.as_str()))
}
