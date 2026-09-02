//! Click-time resolution of one canvas node to an editor target.
//!
//! A turn's canvas nodes are verified once, against the worktree that turn
//! ran in (`turn::verify_canvas_paths`) — but a click can happen long after
//! that worktree was reclaimed by [`super::worktree::reclaim_idle`], on a
//! thread whose next turn (if any) has not run yet. [`super::worktree::ensure`]
//! is what closes that gap: it hands back the stored tree when it still
//! answers and provisions a fresh detached one from `origin/<default>` when
//! it does not, so a node stays openable after reclaim without this module
//! knowing which case it is in.
//!
//! **Not the project's own checkout**, which is what this did first. That
//! clone's working tree is a side effect of whatever feature run last used
//! it — in practice parked on some merged feature branch, hundreds of commits
//! from `origin/master`. Stating a path against it reported files as *moved*
//! that were never touched, and the times it did hit, "open in editor"
//! offered an unrelated commit, which is the worse half. Turn-time
//! verification uses the Ask worktree; resolution that used anything else
//! disagreed with the verdict it is gated on, by construction.

use serde::{Deserialize, Serialize};

use crate::domain::ids::AskThreadId;
use crate::state::AppContext;

/// Where a canvas node's path leads, as of the click rather than as of the
/// turn that cited it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeResolution {
    /// The path still exists in the thread's Ask worktree. `branch` and
    /// `default_branch` are deliberately the same value — that tree is
    /// detached at the commit Ask read, so it has no branch of its own to
    /// name, and it is fenced read-only (`NONE_WRITABLE`): Ask answers
    /// questions about a repository, it never edits one.
    Editor {
        machine_id: String,
        worktree_path: String,
        branch: String,
        default_branch: String,
        path: String,
    },
    /// The path is not in the tree the thread reads now, though it resolved
    /// at turn-time. `checked_commit_sha` is the message's own stored value,
    /// never re-derived.
    Moved { checked_commit_sha: String },
}

/// Resolve a canvas node's `path` against the thread's Ask worktree.
///
/// Requires a stored [`CanvasPathVerdict`](crate::domain::models::CanvasPathVerdict)
/// for `node_id` whose `resolved` is `true` — the frontend gates clicks on
/// that flag (AC-4), so reaching any other state here means a caller
/// violated the contract, not a normal "moved" case. A node with no `path`
/// has no verdict and is refused here; the surface must not ask about one.
///
/// May provision, because [`super::worktree::ensure`] may: a click on a
/// thread whose tree was reclaimed pays for one checkout, and writes the new
/// path back to the thread. That is the same tree the thread's next turn
/// would have built, so the cost is pulled forward rather than added.
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

    // Containment is settled against the repository, before [`ensure`] is
    // allowed to provision anything: whether a path escapes its root is a
    // property of that path's own components, so the root decides the value
    // and never the verdict. Checking it after would mean cutting a whole
    // checkout for `../../etc/hostname` on the way to refusing it.
    if super::path_containment::resolve_within_root(&repo.repo_dir, &verdict.path).is_some() {
        let worktree = super::worktree::ensure(ctx, &thread, &repo).await?;
        let full_path = super::path_containment::resolve_within_root(&worktree, &verdict.path)
            .ok_or_else(|| format!("Canvas node '{node_id}' names a path outside the worktree"))?;
        if ctx
            .exec
            .get_metadata(&repo.machine_str, &full_path.to_string_lossy())
            .await
            .is_ok()
        {
            return Ok(NodeResolution::Editor {
                machine_id: repo.machine_str,
                worktree_path: worktree,
                branch: repo.default_branch.clone(),
                default_branch: repo.default_branch,
                path: verdict.path.clone(),
            });
        }
    }

    let checked_commit_sha = message.checked_commit_sha.clone().ok_or_else(|| {
        "This message has no checked commit to report the node moved from".to_string()
    })?;
    Ok(NodeResolution::Moved { checked_commit_sha })
}

#[cfg(test)]
#[path = "../../../tests/application/ask/node.rs"]
mod tests;
