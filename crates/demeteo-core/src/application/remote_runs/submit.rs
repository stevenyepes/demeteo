use super::attachments::{cleanup_attachment_spool, mark_placeholder_failed, spool_attachments};
use super::compatibility::ensure_runner_compatible;
use super::rpc::{json_str, remote_rpc};
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::{FeatureId, MachineId, ProjectId, WorkflowId};
use crate::domain::models::{
    Feature, ProjectSettings, ProviderInstance, Repository, WorkflowVersion,
};
use crate::domain::run_placement::{detached_target_refusal, DetachedOptions};
use crate::domain::run_spec::{RunBudget, RunOrigin, RunSpec, RunSpecAttachment, RunSpecProvider};
use crate::error::AppError;
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

/// A detached launch is the same [`FeatureLaunch`] a local one starts from,
/// plus the options only a detached run reads, so the two field lists cannot
/// drift. `launch.feature_id`, when given, is the shadow Feature's id.
pub(crate) struct SubmitInput {
    pub machine_id: MachineId,
    pub launch: FeatureLaunch,
    pub detached: DetachedOptions,
}

/// Once `submit_run` is accepted the run exists and is spending, so nothing
/// after it may turn the submit into an `Err` (D8): a caller that saw one
/// would record no attempt and let the same run be submitted again. What
/// went wrong after acceptance is carried here instead.
pub struct SubmitOutcome {
    pub run_id: String,
    pub machine_id: String,
    pub status: String,
    pub feature_id: String,
    /// The shadow row as it was written, so the caller never has to read it
    /// back after acceptance.
    pub feature: Feature,
    /// Why the run is waiting for its PAT, when the runner accepted it but
    /// `inject_credentials` failed. `reinject_credentials` is the recovery.
    pub credentials_parked: Option<String>,
    /// Why the accepted run's mirror row is missing or was left unupdated.
    /// Reconcile hydrates only runs the mirror knows, so a missing row keeps
    /// the run out of the inbox for good.
    pub mirror_unrecorded: Option<String>,
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

fn resolve_run_budget(detached: &DetachedOptions) -> Option<RunBudget> {
    if detached.max_cost_usd.is_some() || detached.max_wall_clock_secs.is_some() {
        Some(RunBudget {
            max_cost_usd: detached.max_cost_usd,
            max_wall_clock_secs: detached.max_wall_clock_secs,
        })
    } else {
        None
    }
}

fn make_run_id() -> String {
    format!("laptop-{}", crate::paths::new_id())
}

/// Everything a submit resolved from its own database before composing the
/// run, bundled because the shadow row and the wire spec are built from the
/// same pieces and have to agree on them.
struct ResolvedSubmit {
    project_id: ProjectId,
    workflow: ResolvedWorkflow,
    provider: RunSpecProvider,
    repo_path: String,
    project_settings: Option<ProjectSettings>,
    attachments: Vec<RunSpecAttachment>,
    budget: Option<RunBudget>,
    feature_id: String,
    now: i64,
}

impl ResolvedSubmit {
    /// The eager placeholder the laptop inserts before the RPC, so the run is
    /// navigable from second zero and hydration updates it in place.
    ///
    /// It carries the run's origin because the laptop reads this row for its
    /// own answers — the review diff's base, the "view diff" link, the branch
    /// it offers to sync — for the whole window before the runner's real row
    /// is hydrated over it. A placeholder saying `DefaultBranch` for a run
    /// launched on a PR is wrong for that entire window, and wrong again
    /// whenever hydration cannot reach the runner.
    fn shadow_feature(&self, launch: &FeatureLaunch) -> Feature {
        Feature {
            effort: launch.effort,
            id: FeatureId::from(self.feature_id.clone()),
            project_id: self.project_id.clone(),
            workflow_id: Some(self.workflow.id.clone()),
            workflow_version_id: Some(self.workflow.version.id.clone()),
            title: launch.title.clone(),
            description: launch.description.clone(),
            status: "pending".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            created_at: self.now,
            agent_kind: launch.agent_kind.clone(),
            model: launch.model.clone(),
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: launch.commit_artifacts,
            loop_iterations: launch.loop_iterations,
            max_budget_usd: launch.max_budget_usd,
            step_overrides: launch.step_overrides.clone(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: launch.origin.clone(),
            diff_base_branch: launch.diff_base_branch.clone(),
            resolved_branch: None,
        }
    }

    /// The spec the runner executes. It is always unattended — see
    /// [`crate::domain::run_placement`].
    ///
    /// `origin` rides as [`RunOrigin::Supported`] — the arm a runner of any
    /// version either honours or refuses by name
    /// ([`RunSpec::origin_to_honour`]). Sending `None` instead is not a
    /// smaller version of the same thing: it tells the runner this launch
    /// chose nothing, and a run launched on a PR head would start from the
    /// default branch with nothing anywhere reporting a disagreement.
    fn run_spec(&self, launch: &FeatureLaunch) -> RunSpec {
        RunSpec {
            effort: launch.effort,
            feature_id: Some(self.feature_id.clone()),
            title: launch.title.clone(),
            description: launch.description.clone(),
            provider: self.provider.clone(),
            repo_path: self.repo_path.clone(),
            workflow_json: self.workflow.json.clone(),
            agent_kind: launch.agent_kind.clone(),
            model: launch.model.clone(),
            loop_iterations: launch.loop_iterations,
            max_budget_usd: launch.max_budget_usd,
            step_overrides: launch.step_overrides.clone(),
            commit_artifacts: launch.commit_artifacts,
            attachments: self.attachments.clone(),
            unattended: true,
            budget: self.budget.clone(),
            project_settings: self.project_settings.clone(),
            origin: Some(RunOrigin::Supported(launch.origin.clone())),
            diff_base_branch: launch.diff_base_branch.clone(),
        }
    }
}

/// Refuses, before anything else, a machine that cannot take a detached run
/// — this desktop, or an id with no `Machine` row — and then a runner that is
/// not this app's build ([`crate::state::AppVersion`]): a refused run must
/// leave no placeholder row and no spooled attachment, and must never have
/// been handed the PAT. Only this entry point is gated — status, retry and
/// reconcile of a run already on the runner are not, so a runner upgraded
/// mid-run does not strand it.
pub(crate) async fn submit_remote_run(
    ctx: &AppContext,
    input: SubmitInput,
) -> Result<SubmitOutcome, AppError> {
    let SubmitInput {
        machine_id,
        mut launch,
        detached,
    } = input;
    let machine = ctx
        .machines
        .get_machine(&machine_id)
        .map_err(AppError::from)?;
    if let Some(refusal) = detached_target_refusal(&machine_id, machine.as_ref()) {
        return Err(AppError::validation(refusal));
    }
    let machine_id = machine_id.as_str();
    ensure_runner_compatible(&*ctx.exec, machine_id, ctx.app_version.get()).await?;
    let project_id = ProjectId::from(launch.project_id.clone());
    let repo = resolve_target_repo(ctx, &project_id, detached.target_repo_id.as_deref())?;
    let provider = resolve_target_provider(ctx, &repo)?;
    let workflow = resolve_workflow_steps(ctx, &launch.workflow_id)?;
    let pat = GitOpsHelper::new(ctx.app_settings.clone(), ctx.exec.clone())
        .get_provider_pat(&provider.id.0)
        .map_err(AppError::from)?;
    let budget = resolve_run_budget(&detached);
    let run_id = make_run_id();
    let staged = std::mem::take(&mut launch.staged_attachments);
    let had_attachments = !staged.is_empty();
    let attachments = match spool_attachments(ctx, machine_id, &run_id, staged).await {
        Ok(attachments) => attachments,
        Err(error) => {
            if had_attachments {
                cleanup_attachment_spool(ctx, machine_id, &run_id).await;
            }
            return Err(AppError::from(error));
        }
    };

    let resolved = ResolvedSubmit {
        project_id: project_id.clone(),
        workflow,
        provider: RunSpecProvider {
            kind: provider.kind.clone(),
            host: provider.host.clone(),
        },
        repo_path: repo.repo_path.clone(),
        project_settings: ctx.projects.get_settings(&project_id).ok().flatten(),
        attachments,
        budget,
        feature_id: launch
            .feature_id
            .clone()
            .unwrap_or_else(|| format!("f-{}", crate::paths::new_id())),
        now: crate::paths::now_ms(),
    };
    let feature_id = resolved.feature_id.clone();
    let now = resolved.now;
    let shadow = resolved.shadow_feature(&launch);
    if let Err(error) = ctx.features.add(shadow.clone()) {
        if had_attachments {
            cleanup_attachment_spool(ctx, machine_id, &run_id).await;
        }
        return Err(AppError::from(error));
    }

    let spec = resolved.run_spec(&launch);
    let submitted = match remote_rpc(
        ctx,
        machine_id,
        "submit_run",
        serde_json::json!({ "run_id": run_id, "spec": spec }),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            mark_placeholder_failed(ctx, &feature_id);
            if had_attachments {
                cleanup_attachment_spool(ctx, machine_id, &run_id).await;
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
            cleanup_attachment_spool(ctx, machine_id, &run_id).await;
        }
        return Err(AppError::from(error));
    }

    let mirror_unrecorded = ctx
        .remote_run_mirror
        .upsert_submitted(
            machine_id,
            &run_id,
            Some(&launch.project_id),
            Some(&feature_id),
            &launch.title,
            now,
        )
        .and_then(|_| {
            ctx.remote_run_mirror
                .update_status(machine_id, &run_id, &status, None, None, None, None, 0, now)
        })
        .err();
    if let Some(error) = &mirror_unrecorded {
        tracing::warn!(machine = %machine_id, run = %run_id, %error, "remote run accepted but not mirrored");
    }
    let credentials_parked = remote_rpc(
        ctx,
        machine_id,
        "inject_credentials",
        serde_json::json!({ "run_id": run_id, "git_pat": pat }),
    )
    .await
    .err();

    Ok(SubmitOutcome {
        run_id,
        machine_id: machine_id.to_string(),
        status,
        feature_id,
        feature: shadow,
        credentials_parked,
        mirror_unrecorded,
    })
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/submit.rs"]
mod tests;
