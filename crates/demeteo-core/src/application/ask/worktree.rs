//! The checkout an Ask turn reads the repository in, on the same terms as
//! [`crate::application::discovery::worktree`]: provisioned on the first turn
//! that needs it, detached, and fenced read-only.
//!
//! Ask writes nothing — it answers questions about a repository, it does not
//! change one — so the tree carries no branch to name, push, or merge, and
//! [`ensure`] never calls a branch-creating provisioner. Reclaiming it is
//! therefore never destructive: what is thrown away is a checkout of a commit,
//! recreated on the next turn without the user seeing it happen.

use std::path::PathBuf;

use crate::adapters::worktree::git_ops::scope::NONE_WRITABLE;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::models::AskThread;
use crate::ports::ask::AskThreadPatch;
use crate::state::AppContext;

/// Where an Ask thread's turns run, and against which repository.
pub struct AskRepo {
    /// `None` for the desktop host, as every [`crate::ports::worktree_ops::WorktreeOpsPort`]
    /// method spells local.
    pub machine_id: Option<String>,
    /// The machine as the agent runtime names it, which spells local as an id
    /// rather than as an absence.
    pub machine_str: String,
    pub repo_dir: String,
    pub default_branch: String,
}

/// Resolve the project side of an Ask thread, on the host it chose.
///
/// The repository is the project's first, matching
/// [`crate::application::discovery::worktree::resolve`]; a project with
/// several is a gap that predates Ask and is not narrowed here.
pub async fn resolve(ctx: &AppContext, thread: &AskThread) -> Result<AskRepo, String> {
    let repos = ctx.projects.get_repositories_for(&thread.project_id)?;
    let repo = repos
        .first()
        .ok_or_else(|| "No repository configured for this project".to_string())?;

    let machine_id = if thread.machine_id.is_local() {
        None
    } else {
        Some(thread.machine_id.as_str().to_string())
    };
    let machine_str = thread.machine_id.as_str().to_string();
    let repo_dir = repo_dir_on(ctx, &machine_str, thread, &repo.repo_path).await?;
    if ctx
        .exec
        .get_metadata(&machine_str, &repo_dir)
        .await
        .is_err()
    {
        return Err(format!(
            "This project has no checkout on '{machine_str}' ({repo_dir}). Move Ask to the host \
             the project was cloned on, or clone it there first."
        ));
    }
    let default_branch = ctx
        .projects
        .get_settings(&thread.project_id)?
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings)
        .worktree_strategy
        .default_branch;

    Ok(AskRepo {
        machine_id,
        machine_str,
        repo_dir,
        default_branch,
    })
}

/// Where this project's repository sits on one named host.
async fn repo_dir_on(
    ctx: &AppContext,
    machine_str: &str,
    thread: &AskThread,
    repo_path: &str,
) -> Result<String, String> {
    if thread.machine_id.is_local() {
        return Ok(crate::paths::repo_target_dir_local(
            &ctx.workspace_dir,
            thread.project_id.as_str(),
            repo_path,
        )
        .to_string_lossy()
        .to_string());
    }
    crate::paths::repo_target_dir_str(
        &ctx.exec,
        "remote",
        Some(machine_str),
        thread.project_id.as_str(),
        repo_path,
        None,
    )
    .await
}

/// The worktree this Ask thread's next turn runs in, provisioning one if the
/// stored path no longer answers.
///
/// **Provisioned from `origin/<default>` rather than the local branch of that
/// name**, for the reason recorded on
/// [`crate::application::discovery::worktree::ensure`]: an Ask turn reading a
/// stale local clone answers with the same confidence as about the present,
/// which is the failure mode. [`GitOpsHelper::refreshed_start_point`] fetches
/// first and falls back to the local ref only when origin cannot be reached.
///
/// The tree is provisioned detached and fenced `NONE_WRITABLE` — never
/// through a branch-creating provisioner — because an Ask turn has nothing to
/// commit.
pub async fn ensure(
    ctx: &AppContext,
    thread: &AskThread,
    repo: &AskRepo,
) -> Result<String, String> {
    if let Some(path) = thread.worktree_path.as_deref().filter(|p| !p.is_empty()) {
        if ctx.exec.get_metadata(&repo.machine_str, path).await.is_ok() {
            return Ok(path.to_string());
        }
    }

    let git = GitOpsHelper::new(ctx.app_settings.clone(), ctx.exec.clone());
    let start_point = git
        .refreshed_start_point(
            &repo.machine_str,
            &repo.repo_dir,
            Some(&repo.default_branch),
        )
        .await
        .map_err(|e| format!("Ask could not find a commit to read: {e}"))?;

    let path = git
        .provision_detached_worktree(
            repo.machine_id.as_deref(),
            &repo.repo_dir,
            &start_point,
            &subtask_id(thread),
            Some(&crate::paths::feature_cache_dir(
                &repo.repo_dir,
                &repo.default_branch,
            )),
        )
        .await?;

    git.apply_artifact_scope(
        repo.machine_id.as_deref(),
        &path,
        &[PathBuf::from(NONE_WRITABLE)],
    )
    .await?;

    ctx.ask.update(
        &thread.id,
        &AskThreadPatch {
            worktree_path: Some(Some(path.clone())),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )?;
    Ok(path)
}

/// Give the tree back, and forget the path.
///
/// Clears the stored path even on a teardown failure, uses
/// `cleanup_subtask_worktree` rather than the detached counterpart, and picks
/// up `resolve`'s repository-lookup errors — all on the same terms
/// [`crate::application::discovery::worktree::reclaim`] documents.
pub async fn reclaim(ctx: &AppContext, thread: &AskThread) -> Result<(), String> {
    if thread
        .worktree_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .is_none()
    {
        return Ok(());
    }
    let repo = resolve(ctx, thread).await?;
    let outcome = ctx
        .worktree_ops
        .cleanup_subtask_worktree(
            repo.machine_id.as_deref(),
            &repo.repo_dir,
            &repo.default_branch,
            &subtask_id(thread),
        )
        .await;
    ctx.ask.update(
        &thread.id,
        &AskThreadPatch {
            worktree_path: Some(None),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )?;
    outcome
}

/// Reclaim every Ask thread, across every project, whose worktree has sat
/// idle since before `cutoff` (an absolute `now_ms`-scale timestamp).
///
/// Returns the ids it reclaimed. One thread's failure does not stop the
/// sweep — a tree that resists teardown is a reason to log, not a reason to
/// leave the others pinned.
pub async fn reclaim_idle(ctx: &AppContext, cutoff: i64) -> Result<Vec<String>, String> {
    let mut reclaimed = Vec::new();
    for project in ctx.projects.get_projects()? {
        for thread in ctx.ask.list_for_project(&project.id)? {
            if thread.updated_at > cutoff || thread.worktree_path.is_none() {
                continue;
            }
            match reclaim(ctx, &thread).await {
                Ok(()) => reclaimed.push(thread.id.as_str().to_string()),
                Err(e) => tracing::warn!(
                    ask_thread = %thread.id.as_str(),
                    error = %e,
                    "ask: idle worktree reclaim failed"
                ),
            }
        }
    }
    Ok(reclaimed)
}

fn subtask_id(thread: &AskThread) -> String {
    format!("ask-{}", thread.id.as_str())
}

/// The commit an Ask worktree currently sits at, for the path-verification
/// ticket to check a canvas node's cited paths against.
pub async fn commit_sha(
    ctx: &AppContext,
    machine_str: &str,
    worktree_path: &str,
) -> Result<String, String> {
    let safe = crate::paths::shell_escape_posix(worktree_path);
    let output = ctx
        .exec
        .run_command(machine_str, &format!("git -C {safe} rev-parse HEAD"))
        .await?;
    Ok(output.trim().to_string())
}

#[cfg(test)]
#[path = "../../../tests/application/ask/worktree.rs"]
mod tests;
