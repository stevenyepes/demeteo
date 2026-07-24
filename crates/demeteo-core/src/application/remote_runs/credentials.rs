use super::rpc::remote_rpc;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::ProjectId;
use crate::error::AppError;
use crate::ports::remote_run_mirror::RemoteRunMirror;
use crate::state::AppContext;

pub(super) async fn inject_pat_for_run(
    ctx: &AppContext,
    machine_id: &str,
    run_id: &str,
    row: &RemoteRunMirror,
) -> Result<(), AppError> {
    let project_id = row.project_id.clone().ok_or_else(|| {
        AppError::from("Run has no project on record; cannot resolve its git provider".to_string())
    })?;
    let pid = ProjectId::from(project_id);
    let repos = ctx
        .projects
        .get_repositories_for(&pid)
        .map_err(AppError::from)?;
    let repo = repos.first().ok_or_else(|| {
        AppError::from("Project has no repository configured; cannot resolve a PAT".to_string())
    })?;
    let provider = ctx
        .app_settings
        .get_provider_instances()
        .map_err(AppError::from)?
        .into_iter()
        .find(|provider| provider.id == repo.provider_id)
        .ok_or_else(|| {
            AppError::from("Repository's git provider instance is not configured".to_string())
        })?;
    let git_ops = GitOpsHelper::new(ctx.app_settings.clone(), ctx.exec.clone());
    let pat = git_ops
        .get_provider_pat(&provider.id.0)
        .map_err(AppError::from)?;

    remote_rpc(
        ctx,
        machine_id,
        "inject_credentials",
        serde_json::json!({ "run_id": run_id, "git_pat": pat }),
    )
    .await
    .map_err(AppError::from)?;
    Ok(())
}
