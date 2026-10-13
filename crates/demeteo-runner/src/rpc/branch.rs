//! `refresh_feature_branch(run_id, git_pat?)`: bring this runner's clone of a
//! settled run's branch up to what origin holds, after the desktop published a
//! sync to it. The decision is
//! [`demeteo_core::domain::runner_branch_refresh`]'s; this gathers the facts
//! and performs it.
//!
//! `git_pat` is used for the one fetch and dropped. It is never put in the
//! run's credential store: a settled run's PAT was wiped on purpose
//! (docs/REMOTE_EXECUTION.md §6.2), and storing it here would leave it resident
//! with no run end left to wipe it.

use crate::git_askpass::run_git;
use crate::services::RunnerServices;
use demeteo_core::adapters::step_executor::setup::fetch_default_settings;
use demeteo_core::domain::ids::{FeatureId, ProjectId};
use demeteo_core::domain::push_lease::{ref_query, ref_tip, tracking_ref};
use demeteo_core::domain::runner_branch_refresh::{
    checkout_holding, is_missing_remote_ref, plan_refresh, refusal_for_run, BranchFacts,
    BranchRefreshOutcome, Checkout, RefreshPlan,
};
use demeteo_core::paths;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

use super::ownership::require_owner;

#[derive(Debug, Deserialize)]
struct RefreshFeatureBranchParams {
    run_id: String,
    #[serde(default)]
    git_pat: Option<String>,
}

pub(super) async fn refresh_feature_branch(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<BranchRefreshOutcome, String> {
    let params: RefreshFeatureBranchParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let run = require_owner(svc, &params.run_id, client_id)?;
    // Before any git: an unsettled run is refused whatever the clone holds,
    // and a fetch for it would be wasted.
    if let Some(refusal) = refusal_for_run(&run.status) {
        return Ok(BranchRefreshOutcome::Refused {
            reason: refusal.to_string(),
        });
    }
    let (Some(project_id), Some(feature_id)) = (run.project_id.clone(), run.feature_id.clone())
    else {
        return Err(format!(
            "run {} never bootstrapped a feature, so it has no branch",
            run.run_id
        ));
    };
    let project_id = ProjectId::from(project_id);
    let repos = svc.ctx.projects.get_repositories_for(&project_id)?;
    let repo = repos
        .first()
        .ok_or_else(|| "project has no repositories configured".to_string())?;
    let settings = svc
        .ctx
        .projects
        .get_settings(&project_id)?
        .unwrap_or_else(fetch_default_settings);
    let repo_dir =
        paths::repo_target_dir_local(&svc.ctx.workspace_dir, project_id.as_str(), &repo.repo_path);
    let branch = crate::run::branch_to_push(
        svc.ctx.features.as_ref(),
        &FeatureId::from(feature_id),
        &settings.worktree_strategy.branch_prefix,
    )?;
    refresh_clone(
        &svc.askpass_path,
        &repo_dir.to_string_lossy(),
        &branch,
        &run.status,
        params.git_pat.as_deref(),
    )
    .await
}

/// The whole refresh against one clone, free of [`RunnerServices`] so a test
/// can drive it over a real repository.
pub(crate) async fn refresh_clone(
    askpass_path: &Path,
    repo_dir: &str,
    branch: &str,
    run_status: &str,
    pat: Option<&str>,
) -> Result<BranchRefreshOutcome, String> {
    // `--refmap=` keeps git from updating `origin/<branch>` opportunistically:
    // the tracking ref is what the push lease reads, and it may only move once
    // the branch has moved with it.
    let head = format!("refs/heads/{branch}");
    let fetch: Vec<String> = [
        "-C",
        repo_dir,
        "fetch",
        "--no-tags",
        "--refmap=",
        "origin",
        head.as_str(),
    ]
    .map(String::from)
    .to_vec();
    let origin_tip = match run_git(askpass_path, &fetch, pat).await {
        Ok(_) => Some(
            git(
                askpass_path,
                repo_dir,
                &["rev-parse", "FETCH_HEAD^{commit}"],
            )
            .await?
            .trim()
            .to_string(),
        ),
        Err(e) if is_missing_remote_ref(&e) => None,
        Err(e) => return Err(format!("could not fetch origin/{branch}: {e}")),
    };

    let listing = git(
        askpass_path,
        repo_dir,
        &ref_query(&head).each_ref().map(String::as_str),
    )
    .await?;
    let local_tip = ref_tip(&listing, &head);
    let local_in_origin = match (local_tip.as_deref(), origin_tip.as_deref()) {
        // `merge-base` exits non-zero for unrelated histories, which is a "no"
        // here; any other failure also reads as "no", the direction that
        // refuses rather than moves.
        (Some(local), Some(origin)) if local != origin => {
            git(askpass_path, repo_dir, &["merge-base", local, origin])
                .await
                .is_ok_and(|base| base.trim() == local)
        }
        _ => true,
    };
    let worktree = checkout_holding(
        &git(askpass_path, repo_dir, &["worktree", "list", "--porcelain"]).await?,
        branch,
    );
    let dirty = match worktree.as_deref() {
        Some(path) => !git(
            askpass_path,
            path,
            &["status", "--porcelain", "--untracked-files=no"],
        )
        .await?
        .trim()
        .is_empty(),
        None => false,
    };

    let plan = plan_refresh(
        run_status,
        &BranchFacts {
            local_tip: local_tip.as_deref(),
            origin_tip: origin_tip.as_deref(),
            local_in_origin,
            checkout: worktree
                .as_deref()
                .map(|worktree| Checkout { worktree, dirty }),
        },
    );
    let tracking = tracking_ref(branch);
    match plan {
        RefreshPlan::Refuse(refusal) => Ok(BranchRefreshOutcome::Refused {
            reason: refusal.to_string(),
        }),
        RefreshPlan::UpToDate { tip } => {
            git(askpass_path, repo_dir, &["update-ref", &tracking, &tip]).await?;
            Ok(BranchRefreshOutcome::UpToDate { tip })
        }
        RefreshPlan::FastForwardCheckout { worktree, from, to } => {
            git(askpass_path, &worktree, &["merge", "--ff-only", &to]).await?;
            git(askpass_path, repo_dir, &["update-ref", &tracking, &to]).await?;
            Ok(BranchRefreshOutcome::Updated { from, to })
        }
        RefreshPlan::MoveRef { from, to } => {
            git(askpass_path, repo_dir, &["update-ref", &head, &to, &from]).await?;
            git(askpass_path, repo_dir, &["update-ref", &tracking, &to]).await?;
            Ok(BranchRefreshOutcome::Updated { from, to })
        }
    }
}

/// A local `git` in `dir`, which never needs a credential.
async fn git(askpass_path: &Path, dir: &str, args: &[&str]) -> Result<String, String> {
    let mut argv = vec!["-C".to_string(), dir.to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    run_git(askpass_path, &argv, None).await
}

#[cfg(test)]
#[path = "branch_tests.rs"]
mod tests;
