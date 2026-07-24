use crate::adapters::step_executor::setup::fetch_default_settings;
use crate::domain::ids::ProjectId;
use crate::error::AppError;
use crate::state::AppContext;

fn build_diff_url(
    kind: &str,
    host: &str,
    repo_path: &str,
    default_branch: &str,
    branch: &str,
) -> String {
    let gitlab = kind.eq_ignore_ascii_case("gitlab");
    if default_branch.trim().is_empty() {
        return if gitlab {
            format!("https://{host}/{repo_path}/-/tree/{branch}")
        } else {
            format!("https://{host}/{repo_path}/tree/{branch}")
        };
    }
    if gitlab {
        format!("https://{host}/{repo_path}/-/compare/{default_branch}...{branch}")
    } else {
        format!("https://{host}/{repo_path}/compare/{default_branch}...{branch}")
    }
}

pub fn resolve_run_diff_url(
    ctx: &AppContext,
    project_id: String,
    branch: String,
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
    Ok(Some(build_diff_url(
        &provider.kind,
        &provider.host,
        &repo.repo_path,
        &settings.worktree_strategy.default_branch,
        &branch,
    )))
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/diff_url.rs"]
mod tests;
