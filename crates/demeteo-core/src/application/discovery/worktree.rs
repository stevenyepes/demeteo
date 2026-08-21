//! The checkout an interview reads the repository in (§4.6, §12 #21 of
//! `docs/PRD_DISCOVERY.md`).
//!
//! Provisioned on the first turn that needs it and reclaimed when the
//! Discovery goes idle, rather than held for the Discovery's life. §8.3 keeps
//! a Discovery open indefinitely, and a tree pinned for that long is a tree
//! pinned forever; the interview writes nothing, so what a reclaim destroys is
//! a checkout of a commit, recreated on the next turn without the user seeing
//! it happen.
//!
//! What it leaves behind while it exists is one local branch. §4.7 asks for
//! nothing in the repository, and this is the whole of the exception: `git
//! worktree add` needs a ref to attach the tree to, the branch is never
//! pushed, and [`reclaim`] deletes it with the tree.

use std::path::PathBuf;

use crate::adapters::worktree::git_ops::scope::NONE_WRITABLE;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::models::Discovery;
use crate::ports::discovery::DiscoveryPatch;
use crate::state::AppContext;

/// Where a Discovery's turns run, and against which repository.
pub struct DiscoveryRepo {
    /// `None` for the desktop host, as every [`crate::ports::worktree_ops::WorktreeOpsPort`]
    /// method spells local.
    pub machine_id: Option<String>,
    /// The machine as the agent runtime names it, which spells local as an id
    /// rather than as an absence.
    pub machine_str: String,
    pub repo_dir: String,
    pub default_branch: String,
}

/// Resolve the project side of a Discovery, on the host it chose.
///
/// The repository is the project's first, matching
/// `application::worktree::resolve_feature_worktree`; a project with several
/// is a gap that predates Discovery and is not narrowed here.
///
/// The machine is [`Discovery::machine_id`], not the project's. §4.5 makes the
/// host part of the interviewer choice, and a stored choice the turn read past
/// would be the same discarded control one layer down. The repository path is
/// therefore derived for *that* host rather than through
/// `application::projects::resolve_target_dir`, which answers for the project's
/// own.
///
/// Which is why the checkout is probed before anything is provisioned in it: a
/// host with no clone of this project is the one way the choice can be wrong,
/// and `git worktree add` reports it as a repository that is not one.
pub async fn resolve(ctx: &AppContext, discovery: &Discovery) -> Result<DiscoveryRepo, String> {
    let repos = ctx.projects.get_repositories_for(&discovery.project_id)?;
    let repo = repos
        .first()
        .ok_or_else(|| "No repository configured for this project".to_string())?;

    let machine_id = if discovery.machine_id.is_local() {
        None
    } else {
        Some(discovery.machine_id.as_str().to_string())
    };
    let machine_str = discovery.machine_id.as_str().to_string();
    let repo_dir = repo_dir_on(ctx, &machine_str, discovery, &repo.repo_path).await?;
    if ctx
        .exec
        .get_metadata(&machine_str, &repo_dir)
        .await
        .is_err()
    {
        return Err(format!(
            "This project has no checkout on '{machine_str}' ({repo_dir}). Move the interview to \
             the host the project was cloned on, or clone it there first."
        ));
    }
    let default_branch = ctx
        .projects
        .get_settings(&discovery.project_id)?
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings)
        .worktree_strategy
        .default_branch;

    Ok(DiscoveryRepo {
        machine_id,
        machine_str,
        repo_dir,
        default_branch,
    })
}

/// Where this project's repository sits on one named host.
///
/// `compute_type` is read by [`crate::paths::repo_target_dir_str`] only as
/// local-or-not, and the machine already answers that — so it is spelled from
/// the machine rather than taken from the project, whose own host this may not
/// be.
async fn repo_dir_on(
    ctx: &AppContext,
    machine_str: &str,
    discovery: &Discovery,
    repo_path: &str,
) -> Result<String, String> {
    if discovery.machine_id.is_local() {
        return Ok(crate::paths::repo_target_dir_local(
            &ctx.workspace_dir,
            discovery.project_id.as_str(),
            repo_path,
        )
        .to_string_lossy()
        .to_string());
    }
    crate::paths::repo_target_dir_str(
        &ctx.exec,
        "remote",
        Some(machine_str),
        discovery.project_id.as_str(),
        repo_path,
        None,
    )
    .await
}

/// The worktree this Discovery's next turn runs in, provisioning one if the
/// stored path no longer answers.
///
/// A path that has gone missing and a Discovery that never had one are the
/// same case: nothing in the tree is authoritative, so neither is worth
/// telling apart from the other.
pub async fn ensure(
    ctx: &AppContext,
    discovery: &Discovery,
    repo: &DiscoveryRepo,
) -> Result<String, String> {
    if let Some(path) = discovery.worktree_path.as_deref().filter(|p| !p.is_empty()) {
        if ctx.exec.get_metadata(&repo.machine_str, path).await.is_ok() {
            return Ok(path.to_string());
        }
    }

    let path = ctx
        .worktree_ops
        .provision_subtask_worktree(
            repo.machine_id.as_deref(),
            &repo.repo_dir,
            &repo.default_branch,
            &subtask_id(discovery),
        )
        .await?;

    GitOpsHelper::new(ctx.app_settings.clone(), ctx.exec.clone())
        .apply_artifact_scope(
            repo.machine_id.as_deref(),
            &path,
            &[PathBuf::from(NONE_WRITABLE)],
        )
        .await?;

    ctx.discoveries.update(
        &discovery.id,
        &DiscoveryPatch {
            worktree_path: Some(Some(path.clone())),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )?;
    Ok(path)
}

/// Give the tree and its branch back, and forget the path.
///
/// The path is cleared even when teardown reported a failure: a stored path
/// whose tree is half-removed is what makes the *next* turn fail too, and
/// [`ensure`] treats a missing one as nothing to explain.
pub async fn reclaim(ctx: &AppContext, discovery: &Discovery) -> Result<(), String> {
    if discovery
        .worktree_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .is_none()
    {
        return Ok(());
    }
    let repo = resolve(ctx, discovery).await?;
    let outcome = ctx
        .worktree_ops
        .cleanup_subtask_worktree(
            repo.machine_id.as_deref(),
            &repo.repo_dir,
            &repo.default_branch,
            &subtask_id(discovery),
        )
        .await;
    ctx.discoveries.update(
        &discovery.id,
        &DiscoveryPatch {
            worktree_path: Some(None),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )?;
    outcome
}

/// Reclaim every open Discovery of a project whose last turn is older than
/// `idle_after_ms`.
///
/// Returns the ids it reclaimed. One Discovery's failure does not stop the
/// sweep — a tree that resists teardown is a reason to log, not a reason to
/// leave the others pinned.
pub async fn reclaim_idle(
    ctx: &AppContext,
    project_id: &crate::domain::ids::ProjectId,
    idle_after_ms: i64,
) -> Result<Vec<String>, String> {
    let cutoff = crate::paths::now_ms() - idle_after_ms;
    let mut reclaimed = Vec::new();
    for row in ctx.discoveries.list_for_project(project_id)? {
        let discovery = row.discovery;
        if discovery.updated_at > cutoff || discovery.worktree_path.is_none() {
            continue;
        }
        match reclaim(ctx, &discovery).await {
            Ok(()) => reclaimed.push(discovery.id.as_str().to_string()),
            Err(e) => tracing::warn!(
                discovery = %discovery.id.as_str(),
                error = %e,
                "discovery: idle worktree reclaim failed"
            ),
        }
    }
    Ok(reclaimed)
}

fn subtask_id(discovery: &Discovery) -> String {
    format!("discovery-{}", discovery.id.as_str())
}
