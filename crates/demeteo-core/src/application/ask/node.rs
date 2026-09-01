//! Click-time resolution of one canvas node to an editor target.
//!
//! A turn's canvas nodes are verified once, against the worktree that turn
//! ran in (`turn::verify_canvas_paths`) — but a click can happen
//! long after that worktree was reclaimed by
//! [`super::worktree::reclaim_idle`], on a thread whose next turn (if any)
//! has not run yet. So [`resolve`] never reads
//! [`AskThread::worktree_path`](crate::domain::models::AskThread::worktree_path):
//! it re-resolves against the *project's* checkout, the same repository
//! [`super::worktree::resolve`] already derives for the turn loop, on the
//! thread's own chosen machine. That checkout outlives the turn worktree by
//! construction, so a node can still be opened after reclaim.

use serde::{Deserialize, Serialize};

use crate::domain::ids::AskThreadId;
use crate::state::AppContext;

/// Where a canvas node's path leads, as of the click rather than as of the
/// turn that cited it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeResolution {
    /// The path still exists in the project's checkout. `branch` and
    /// `default_branch` are deliberately the same value — a project-level
    /// checkout has no feature branch to name.
    Editor {
        machine_id: String,
        worktree_path: String,
        branch: String,
        default_branch: String,
        path: String,
    },
    /// The path no longer exists in the project's current checkout, though
    /// it resolved at turn-time. `checked_commit_sha` is the message's own
    /// stored value, never re-derived.
    Moved { checked_commit_sha: String },
}

/// Resolve a canvas node's `path` against the project's own checkout.
///
/// Requires a stored [`CanvasPathVerdict`](crate::domain::models::CanvasPathVerdict)
/// for `node_id` whose `resolved` is `true` — the frontend gates clicks on
/// that flag (AC-4), so reaching any other state here means a caller
/// violated the contract, not a normal "moved" case.
pub async fn resolve(
    ctx: &AppContext,
    thread_id: &AskThreadId,
    message_id: &str,
    node_id: &str,
) -> Result<NodeResolution, String> {
    let thread = ctx
        .ask
        .get(thread_id)?
        .ok_or_else(|| format!("Ask thread not found: {}", thread_id.as_str()))?;
    let message = ctx
        .ask
        .list_messages(thread_id)?
        .into_iter()
        .find(|m| m.id == message_id)
        .ok_or_else(|| format!("Ask message not found: {message_id}"))?;
    let verdict = message
        .canvas_paths
        .as_ref()
        .and_then(|verdicts| verdicts.iter().find(|v| v.node_id == node_id))
        .ok_or_else(|| format!("No verified path recorded for canvas node '{node_id}'"))?;
    if !verdict.resolved {
        return Err(format!(
            "Canvas node '{node_id}' was never resolved; the caller should not have offered it"
        ));
    }

    let repo = super::worktree::resolve(ctx, &thread).await?;
    let stat_ok = match super::path_containment::resolve_within_root(&repo.repo_dir, &verdict.path)
    {
        Some(full_path) => ctx
            .exec
            .get_metadata(&repo.machine_str, &full_path.to_string_lossy())
            .await
            .is_ok(),
        None => false,
    };

    if stat_ok {
        return Ok(NodeResolution::Editor {
            machine_id: repo.machine_str,
            worktree_path: repo.repo_dir,
            branch: repo.default_branch.clone(),
            default_branch: repo.default_branch,
            path: verdict.path.clone(),
        });
    }

    let checked_commit_sha = message.checked_commit_sha.clone().ok_or_else(|| {
        "This message has no checked commit to report the node moved from".to_string()
    })?;
    Ok(NodeResolution::Moved { checked_commit_sha })
}

#[cfg(test)]
#[path = "../../../tests/application/ask/node.rs"]
mod tests;
