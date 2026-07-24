use super::attachments::{cleanup_attachment_spool, mark_placeholder_failed, spool_attachments};
use super::rpc::{json_str, remote_rpc};
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::application::attachments::StagedAttachmentInput;
use crate::domain::ids::{FeatureId, ProjectId, WorkflowId};
use crate::domain::models::{
    EffortLevel, Feature, ProviderInstance, Repository, StepOverride, WorkflowVersion,
};
use crate::domain::run_spec::{RunBudget, RunSpec, RunSpecProvider};
use crate::error::AppError;
use crate::state::AppContext;

pub struct SubmitInput {
    pub machine_id: String,
    pub project_id: String,
    pub workflow_id: String,
    pub title: String,
    pub description: String,
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<EffortLevel>,
    pub commit_artifacts: Option<bool>,
    pub loop_iterations: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub step_overrides: Option<Vec<StepOverride>>,
    pub staged_attachments: Option<Vec<StagedAttachmentInput>>,
    pub target_repo_id: Option<String>,
    pub unattended: bool,
    pub max_cost_usd: Option<f64>,
    pub max_wall_clock_secs: Option<u64>,
}

pub struct SubmitOutcome {
    pub run_id: String,
    pub machine_id: String,
    pub status: String,
    pub feature_id: String,
}

struct ResolvedWorkflow {
    id: WorkflowId,
    version: WorkflowVersion,
    json: serde_json::Value,
}

fn resolve_target_repo(
    ctx: &AppContext,
    project_id: &ProjectId,
    target_repo_id: Option<&str>,
) -> Result<Repository, AppError> {
    let repos = ctx
        .projects
        .get_repositories_for(project_id)
        .map_err(AppError::from)?;
    match target_repo_id {
        Some(id) => repos
            .into_iter()
            .find(|repo| repo.id.0 == id)
            .ok_or_else(|| {
                AppError::from(format!(
                    "Selected repository {id} is not attached to this project"
                ))
            }),
        None => repos.into_iter().next().ok_or_else(|| {
            AppError::from("Project has no repository configured; remote runs need one".to_string())
        }),
    }
}

fn resolve_target_provider(
    ctx: &AppContext,
    repo: &Repository,
) -> Result<ProviderInstance, AppError> {
    ctx.app_settings
        .get_provider_instances()
        .map_err(AppError::from)?
        .into_iter()
        .find(|provider| provider.id == repo.provider_id)
        .ok_or_else(|| {
            AppError::from("Repository's git provider instance is not configured".to_string())
        })
}

fn resolve_workflow_steps(
    ctx: &AppContext,
    workflow_id: &str,
) -> Result<ResolvedWorkflow, AppError> {
    let id = WorkflowId::from(workflow_id.to_string());
    let workflow = ctx
        .workflows
        .get(&id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found(format!("Workflow not found: {workflow_id}")))?;
    let version = ctx
        .workflows
        .latest_version(&id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::from("Workflow has no steps".to_string()))?;
    let steps: serde_json::Value = serde_json::from_str(&version.steps_json)
        .map_err(|error| AppError::from(error.to_string()))?;
    Ok(ResolvedWorkflow {
        id,
        version,
        json: serde_json::json!({
            "name": workflow.name,
            "description": workflow.description,
            "steps": steps,
        }),
    })
}

fn resolve_run_budget(input: &SubmitInput) -> Option<RunBudget> {
    if input.max_cost_usd.is_some() || input.max_wall_clock_secs.is_some() {
        Some(RunBudget {
            max_cost_usd: input.max_cost_usd,
            max_wall_clock_secs: input.max_wall_clock_secs,
        })
    } else {
        None
    }
}

fn make_run_id() -> String {
    format!("laptop-{}", crate::paths::new_id())
}

fn insert_shadow_feature(
    ctx: &AppContext,
    input: &SubmitInput,
    project_id: &ProjectId,
    workflow: &ResolvedWorkflow,
    feature_id: &str,
    step_overrides: &[StepOverride],
    now: i64,
) -> Result<(), String> {
    ctx.features.add(Feature {
        effort: input.effort,
        id: FeatureId::from(feature_id.to_string()),
        project_id: project_id.clone(),
        workflow_id: Some(workflow.id.clone()),
        workflow_version_id: Some(workflow.version.id.clone()),
        title: input.title.clone(),
        description: input.description.clone(),
        status: "pending".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        tokens: 0,
        created_at: now,
        agent_kind: input.agent_kind.clone(),
        model: input.model.clone(),
        mr_url: None,
        mr_state: Some("none".to_string()),
        pr_title: None,
        pr_body: None,
        commit_artifacts: input.commit_artifacts,
        loop_iterations: input.loop_iterations,
        max_budget_usd: input.max_budget_usd,
        step_overrides: step_overrides.to_vec(),
        attachments: Vec::new(),
    })
}

pub async fn submit_remote_run(
    ctx: &AppContext,
    mut input: SubmitInput,
) -> Result<SubmitOutcome, AppError> {
    let project_id = ProjectId::from(input.project_id.clone());
    let repo = resolve_target_repo(ctx, &project_id, input.target_repo_id.as_deref())?;
    let provider = resolve_target_provider(ctx, &repo)?;
    let workflow = resolve_workflow_steps(ctx, &input.workflow_id)?;
    let pat = GitOpsHelper::new(ctx.app_settings.clone(), ctx.exec.clone())
        .get_provider_pat(&provider.id.0)
        .map_err(AppError::from)?;
    let budget = resolve_run_budget(&input);
    let run_id = make_run_id();
    let staged = input.staged_attachments.take().unwrap_or_default();
    let had_attachments = !staged.is_empty();
    let attachments = match spool_attachments(ctx, &input.machine_id, &run_id, staged).await {
        Ok(attachments) => attachments,
        Err(error) => {
            if had_attachments {
                cleanup_attachment_spool(ctx, &input.machine_id, &run_id).await;
            }
            return Err(AppError::from(error));
        }
    };

    let now = crate::paths::now_ms();
    let feature_id = format!("f-{}", crate::paths::new_id());
    let step_overrides = input.step_overrides.take().unwrap_or_default();
    if let Err(error) = insert_shadow_feature(
        ctx,
        &input,
        &project_id,
        &workflow,
        &feature_id,
        &step_overrides,
        now,
    ) {
        if had_attachments {
            cleanup_attachment_spool(ctx, &input.machine_id, &run_id).await;
        }
        return Err(AppError::from(error));
    }

    let project_settings = ctx.projects.get_settings(&project_id).ok().flatten();
    let spec = RunSpec {
        effort: input.effort,
        feature_id: Some(feature_id.clone()),
        title: input.title.clone(),
        description: input.description.clone(),
        provider: RunSpecProvider {
            kind: provider.kind.clone(),
            host: provider.host.clone(),
        },
        repo_path: repo.repo_path.clone(),
        workflow_json: workflow.json,
        agent_kind: input.agent_kind.clone(),
        model: input.model.clone(),
        loop_iterations: input.loop_iterations,
        max_budget_usd: input.max_budget_usd,
        step_overrides,
        commit_artifacts: input.commit_artifacts,
        attachments,
        unattended: input.unattended,
        budget,
        project_settings,
    };
    let submitted = match remote_rpc(
        ctx,
        &input.machine_id,
        "submit_run",
        serde_json::json!({ "run_id": run_id, "spec": spec }),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            mark_placeholder_failed(ctx, &feature_id);
            if had_attachments {
                cleanup_attachment_spool(ctx, &input.machine_id, &run_id).await;
            }
            return Err(AppError::from(error));
        }
    };
    let status = json_str(&submitted, "status").unwrap_or_else(|| "pending".to_string());
    if status == "failed" {
        let error = json_str(&submitted, "error")
            .unwrap_or_else(|| "the runner rejected this run".to_string());
        mark_placeholder_failed(ctx, &feature_id);
        if had_attachments {
            cleanup_attachment_spool(ctx, &input.machine_id, &run_id).await;
        }
        return Err(AppError::from(error));
    }

    ctx.remote_run_mirror
        .upsert_submitted(
            &input.machine_id,
            &run_id,
            Some(&input.project_id),
            Some(&feature_id),
            &input.title,
            now,
        )
        .map_err(AppError::from)?;
    ctx.remote_run_mirror
        .update_status(
            &input.machine_id,
            &run_id,
            &status,
            None,
            None,
            None,
            None,
            0,
            now,
        )
        .map_err(AppError::from)?;
    remote_rpc(
        ctx,
        &input.machine_id,
        "inject_credentials",
        serde_json::json!({ "run_id": run_id, "git_pat": pat }),
    )
    .await
    .map_err(AppError::from)?;

    Ok(SubmitOutcome {
        run_id,
        machine_id: input.machine_id,
        status,
        feature_id,
    })
}
