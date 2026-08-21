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
//! about the project, [`question`] holds the prompt, [`decompose`] ends the
//! interview in tickets, [`worktree`] owns the checkout it reads in, and
//! [`events`] is what the surface hears.

pub mod context;
pub mod decompose;
pub mod events;
pub mod question;
pub mod turn;
pub mod worktree;

use crate::domain::discovery_question::{parse_interview_turn, InterviewTurn};
use crate::domain::ids::{DiscoveryId, MachineId, ProjectId};
use crate::domain::models::{
    Discovery, DiscoveryMessage, DiscoveryStatus, EffortLevel, MessageRole,
};
use crate::ports::discovery::DiscoveryPatch;
use crate::state::AppContext;
use serde::{Deserialize, Serialize};

/// What creating a Discovery needs, which is the interviewer choice and
/// nothing else. §4.5 puts agent, model, effort and machine on the Discovery
/// rather than inheriting the project default: interviewing and implementing
/// want different things from a model.
#[derive(Debug, Clone, Deserialize)]
pub struct NewDiscovery {
    pub project_id: String,
    pub title: String,
    pub agent_kind: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<EffortLevel>,
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

pub fn list_for_project(ctx: &AppContext, project_id: &str) -> Result<Vec<Discovery>, String> {
    ctx.discoveries
        .list_for_project(&ProjectId::from(project_id.to_string()))
}

pub fn get(ctx: &AppContext, id: &DiscoveryId) -> Result<DiscoveryDetail, String> {
    let discovery = ctx
        .discoveries
        .get(id)?
        .ok_or_else(|| format!("Discovery not found: {}", id.as_str()))?;
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
/// The machine is the project's, not a choice: the repository this interview
/// reads exists on exactly one host. `worktree::resolve` carries the rest.
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
    let machine_id = if project.compute_type.eq_ignore_ascii_case("local") {
        MachineId::from(crate::domain::ids::LOCAL_MACHINE.to_string())
    } else {
        project
            .remote_host
            .clone()
            .ok_or_else(|| "Remote project has no configured machine".to_string())?
    };

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
        total_cost: 0.0,
        tokens: 0,
        created_at: now,
        updated_at: now,
    };
    ctx.discoveries.create(&discovery)?;
    Ok(discovery)
}

/// End the interview without ending anything else (§8.4).
///
/// The checkout goes back here rather than waiting for the idle sweep: a
/// closed Discovery has no next turn to recreate it for.
pub async fn close(ctx: &AppContext, id: &DiscoveryId) -> Result<(), String> {
    let discovery = ctx
        .discoveries
        .get(id)?
        .ok_or_else(|| format!("Discovery not found: {}", id.as_str()))?;
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
/// which carries why. What this adds is the teardown a plain row delete would
/// leave behind: the live session and the checkout it was reading in.
pub async fn delete(ctx: &AppContext, id: &DiscoveryId) -> Result<(), String> {
    let discovery = ctx
        .discoveries
        .get(id)?
        .ok_or_else(|| format!("Discovery not found: {}", id.as_str()))?;
    let tickets = ctx.tickets.list_for_discovery(id)?;
    if let Some(refusal) = crate::application::tickets::deletion_refusal(&tickets) {
        return Err(refusal);
    }
    ctx.registry.kill(&turn::thread_id(id)).await;
    if let Err(e) = worktree::reclaim(ctx, &discovery).await {
        tracing::warn!(discovery = %id.as_str(), error = %e, "discovery: delete could not reclaim the worktree");
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
