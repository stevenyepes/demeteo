//! Ask: a lightweight, project-scoped chat thread with an agent (V51,
//! `docs/PRD_DISCOVERY.md`-adjacent but not itself a Discovery — see
//! [`crate::domain::models::ask`]).
//!
//! Storage and lifecycle only: create, project list, load, rename, delete.
//! No turn execution, worktree allocation, or stream events — those are
//! `ask-turn-loop`'s and `ask-thread-ui`'s to add.

use serde::{Deserialize, Serialize};

use crate::domain::ids::{AskThreadId, MachineId, ProjectId};
use crate::domain::models::{AskMessage, AskStatus, AskThread, EffortLevel};
use crate::ports::ask::AskThreadPatch;
use crate::state::AppContext;

/// What creating an Ask thread needs: the interviewer choice, mirroring
/// [`NewDiscovery`](crate::application::discovery::NewDiscovery) minus the
/// decomposition/attachment surface Ask does not have.
#[derive(Debug, Clone, Deserialize)]
pub struct NewAskThread {
    pub project_id: String,
    pub title: String,
    pub agent_kind: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<EffortLevel>,
    /// Where the thread runs. `None` takes the project's own host, on the
    /// same terms as [`domain::discovery_host::interviewer_machine`](crate::domain::discovery_host::interviewer_machine).
    #[serde(default)]
    pub machine_id: Option<String>,
}

/// An Ask thread and its whole transcript, in the order it was said.
#[derive(Debug, Clone, Serialize)]
pub struct AskThreadDetail {
    pub thread: AskThread,
    pub messages: Vec<AskMessage>,
}

/// Open an Ask thread. No worktree and no agent process yet — both wait for
/// the first turn that needs them, the same deferral
/// [`discovery::create`](crate::application::discovery::create) makes.
pub fn create(ctx: &AppContext, new: NewAskThread) -> Result<AskThread, String> {
    let title = crate::domain::models::validate_title(&new.title)?;
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
    let thread = AskThread {
        id: AskThreadId::from(crate::shared::ids::new_id()),
        project_id,
        title,
        status: AskStatus::Open,
        agent_kind: new.agent_kind,
        model: new.model,
        effort: new.effort,
        machine_id,
        worktree_path: None,
        session_id: None,
        turn_count: 0,
        cost_usd: 0.0,
        tokens: 0,
        created_at: now,
        updated_at: now,
    };
    ctx.ask.create(&thread)?;
    Ok(thread)
}

/// A project's Ask threads, most recently touched first.
pub fn list_for_project(ctx: &AppContext, project_id: &str) -> Result<Vec<AskThread>, String> {
    ctx.ask
        .list_for_project(&ProjectId::from(project_id.to_string()))
}

/// An Ask thread and its whole transcript.
pub fn load(ctx: &AppContext, id: &AskThreadId) -> Result<AskThreadDetail, String> {
    let thread = get(ctx, id)?;
    let messages = ctx.ask.list_messages(id)?;
    Ok(AskThreadDetail { thread, messages })
}

/// Rename an Ask thread, advancing `updated_at`.
pub fn rename(ctx: &AppContext, id: &AskThreadId, title: &str) -> Result<AskThread, String> {
    get(ctx, id)?;
    let title = crate::domain::models::validate_title(title)?;
    ctx.ask.update(
        id,
        &AskThreadPatch {
            title: Some(title),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )?;
    get(ctx, id)
}

/// Delete an Ask thread and its transcript, via the declared foreign key.
pub fn delete(ctx: &AppContext, id: &AskThreadId) -> Result<(), String> {
    get(ctx, id)?;
    ctx.ask.delete(id)
}

/// Refuse a machine nothing is configured for, the same check
/// [`discovery::refuse_unknown_machine`](crate::application::discovery)
/// makes for the interviewer.
fn refuse_unknown_machine(ctx: &AppContext, machine_id: &MachineId) -> Result<(), String> {
    if machine_id.is_local() || ctx.machines.get_machine(machine_id)?.is_some() {
        return Ok(());
    }
    Err(format!(
        "No machine is configured as '{}'. Add it under Machines, or leave Ask on this \
         project's own host.",
        machine_id.as_str()
    ))
}

fn get(ctx: &AppContext, id: &AskThreadId) -> Result<AskThread, String> {
    ctx.ask
        .get(id)?
        .ok_or_else(|| format!("Ask thread not found: {}", id.as_str()))
}

#[cfg(test)]
#[path = "../../../tests/application/ask/mod.rs"]
mod tests;
