use crate::adapters::step_executor::setup::fetch_default_settings;
use crate::domain::ids::{FeatureId, ProjectId};
use crate::error::AppError;
use crate::state::AppContext;

fn build_diff_url(
    kind: &str,
    host: &str,
    repo_path: &str,
    base_branch: &str,
    branch: &str,
) -> String {
    let gitlab = kind.eq_ignore_ascii_case("gitlab");
    if base_branch.trim().is_empty() {
        return if gitlab {
            format!("https://{host}/{repo_path}/-/tree/{branch}")
        } else {
            format!("https://{host}/{repo_path}/tree/{branch}")
        };
    }
    if gitlab {
        format!("https://{host}/{repo_path}/-/compare/{base_branch}...{branch}")
    } else {
        format!("https://{host}/{repo_path}/compare/{base_branch}...{branch}")
    }
}

/// The provider's compare view for a run's pushed branch.
///
/// `feature_id` is the run's Feature — the mirror row carries it — and is what
/// makes the left side of the compare the branch the run declared itself
/// measured against rather than the project's default. `None` (a mirror from
/// before the id was recorded, or a feature this database never saw) falls back
/// to the project default, which is what the link always used to be.
pub fn resolve_run_diff_url(
    ctx: &AppContext,
    project_id: String,
    branch: String,
    feature_id: Option<String>,
) -> Result<Option<String>, AppError> {
    let pid = ProjectId::from(project_id);
    let Ok(repos) = ctx.projects.get_repositories_for(&pid) else {
        return Ok(None);
    };
    let Some(repo) = repos.first() else {
        return Ok(None);
    };
    let Ok(providers) = ctx.app_settings.get_provider_instances() else {
        return Ok(None);
    };
    let Some(provider) = providers
        .into_iter()
        .find(|provider| provider.id == repo.provider_id)
    else {
        return Ok(None);
    };
    let settings = ctx
        .projects
        .get_settings(&pid)
        .ok()
        .flatten()
        .unwrap_or_else(fetch_default_settings);
    let feature = feature_id
        .map(FeatureId::from)
        .and_then(|fid| ctx.features.get(&fid).ok().flatten());
    let base_branch = feature
        .as_ref()
        .and_then(|f| {
            crate::domain::diff_base::resolve(
                f.diff_base_branch.as_deref(),
                &f.origin,
                &settings.worktree_strategy.default_branch,
            )
        })
        .unwrap_or(&settings.worktree_strategy.default_branch);
    Ok(Some(build_diff_url(
        &provider.kind,
        &provider.host,
        &repo.repo_path,
        base_branch,
        &branch,
    )))
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/diff_url.rs"]
mod tests;
