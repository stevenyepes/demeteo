//! Resolve a feature's on-disk working directory + branch, shared by the
//! desktop `feature_get_worktree` command and the runner's `get_worktree`
//! control RPC (Variant A of the detached-run "Browse Code" fix).
//!
//! Both need the *same* answer to "where is feature F checked out, and on
//! what branch", computed from the feature's project/repo/settings — so it
//! lives here once rather than in two copies that drift. The desktop calls
//! it against its own `AppContext`; the runner calls it against its own,
//! keyed through the run's runner-owned feature. Because the two contexts
//! carry different `workspace_dir`s and project ids, each side naturally
//! yields the path on *its* machine — which is exactly what a detached run
//! needs (the runner's real worktree path, not the laptop's computed one).

use serde::Serialize;

use crate::domain::ids::FeatureId;
use crate::state::AppContext;

/// Where a feature's working tree lives and the branch it holds.
///
/// `machine_id` is `"local"` for a local-compute project, otherwise the
/// project's remote host. For a runner-owned feature this is always
/// `"local"` (the runner is `LocalOnly` — it *is* the machine); the laptop
/// caller substitutes the mirror's real machine id, since the path is a
/// path *on the runner's box*, reachable over the SSH the laptop already
/// holds to it.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureWorktreeInfo {
    pub machine_id: String,
    pub worktree_path: String,
    pub branch: String,
    /// The branch the in-app editor diffs `branch` against — this run's own
    /// base ([`diff_base::resolve`](crate::domain::diff_base::resolve)), which
    /// is the project's default branch only for a run that started there.
    ///
    /// Named for what it held before V41 because the name is on the wire: the
    /// laptop reads this field off the runner's `get_worktree` reply, and a
    /// rename would answer an older runner with nothing.
    pub default_branch: String,
}

/// Resolve `feature_id`'s worktree path + branch from its project, repo, and
/// saved worktree strategy. Mirrors the exact computation the executor uses
/// to place a feature's checkout, so browsing the result matches where the
/// run actually worked.
pub async fn resolve_feature_worktree(
    ctx: &AppContext,
    feature_id: &FeatureId,
) -> Result<FeatureWorktreeInfo, String> {
    let feature = ctx
        .features
        .get(feature_id)?
        .ok_or_else(|| "Feature not found".to_string())?;

    let project_id = feature.project_id.clone();
    let project = ctx
        .projects
        .get_projects()?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| "Project not found".to_string())?;

    let repos = ctx.projects.get_repositories_for(&project_id)?;
    let repo = repos
        .first()
        .ok_or_else(|| "No repository configured for this project".to_string())?;

    let settings = ctx
        .projects
        .get_settings(&project_id)?
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);

    let is_local = project.compute_type.eq_ignore_ascii_case("local");
    let machine_id = if is_local {
        crate::domain::ids::LOCAL_MACHINE.to_string()
    } else {
        project
            .remote_host
            .as_deref()
            .unwrap_or(crate::domain::ids::LOCAL_MACHINE)
            .to_string()
    };

    let worktree_path = if is_local {
        crate::paths::repo_target_dir_local(&ctx.workspace_dir, &project_id.0, &repo.repo_path)
            .to_string_lossy()
            .to_string()
    } else {
        crate::paths::repo_target_dir_str(
            &ctx.exec,
            &project.compute_type,
            project.remote_host.as_deref(),
            &project_id.0,
            &repo.repo_path,
            None,
        )
        .await?
    };

    let branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);
    let default_branch = crate::domain::diff_base::resolve(
        feature.diff_base_branch.as_deref(),
        &feature.origin,
        &settings.worktree_strategy.default_branch,
    )
    .unwrap_or(&settings.worktree_strategy.default_branch)
    .to_string();

    Ok(FeatureWorktreeInfo {
        machine_id,
        worktree_path,
        branch,
        default_branch,
    })
}
