//! Feature-branch sync with the upstream `base_branch`.
//!
//! Two Tauri commands surface this code path:
//!
//! - `feature_sync`: merges `origin/<base>` into the feature
//!   branch. If the merge is clean, returns a `SyncOutcomeView::Ok`.
//!   If there are conflicts, returns a `SyncOutcomeView::Conflict`
//!   with the parsed conflict list — the UI then offers a "Resolve
//!   with agent" button.
//!
//! - `feature_resolve_sync_conflicts`: spawns a fresh agent session
//!   in a temporary worktree on the conflicted feature branch and
//!   asks it to remove conflict markers. When the agent finishes
//!   (or its cost / time budget runs out), the resolution is
//!   committed, the worktree is merged back into the feature branch
//!   on the main repo, and the optional re-validate step is replayed.
//!
//! Both commands live in `commands/features.rs` (the thin IPC
//! layer); this module owns the orchestration. It reuses the existing
//! `GitOpsHelper` for git, `MergeExecutor` for the conflict
//! detection, and the `AgentRegistry` for spawning — no new ports.

use std::sync::Arc;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::steps::list_unmerged::list_unmerged_files;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::agent_event::AgentEvent;
use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::paths;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::{AppSettingsRepository, FeatureRepository};
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::DomainEvent;
use crate::ports::notification::NotificationPort;
use crate::ports::pricing::PricingTable;
use crate::ports::step_executor::{StepExecutor, SyncOutcomeView};

use super::sync_worktree::discard_sync_worktree;
use super::DagStepExecutor;

/// The branch a sync merges into the feature branch.
///
/// [`diff_base::resolve`](crate::domain::diff_base::resolve) and nothing
/// else: a run cut from `origin/release/2.0` that merged the project default
/// instead would pull the whole of trunk into a release branch's feature.
pub(crate) fn sync_base(
    feature: &crate::domain::models::Feature,
    settings: &crate::domain::models::ProjectSettings,
) -> Result<String, String> {
    crate::domain::diff_base::resolve(
        feature.diff_base_branch.as_deref(),
        &feature.origin,
        &settings.worktree_strategy.default_branch,
    )
    .map(str::to_string)
    .ok_or_else(|| {
        "This run names no base branch to sync from; set the project's default branch.".to_string()
    })
}

/// The thread-id suffix for the conflict-resolution agent. We use a
/// fresh id (not `feature_id`) so the resolution session is fully
/// independent from the step-execution agent session that drove the
/// implementation: the resolver gets a clean prompt and its own
/// `OPENCODE_PERMISSION` scope.
const SYNC_RESOLVER_THREAD_PREFIX: &str = "sync-resolver";

/// Unified sync conflict resolver helper. Drives the conflict resolution agent,
/// streams UI status events, monitors timeouts, verifies conflict markers,
/// commits the resolution, and pushes it to remote origin.
pub(crate) struct ResolveSyncContext<'a> {
    pub exec: &'a Arc<dyn ExecutionPort>,
    pub registry: &'a Arc<AgentRegistry>,
    pub notif: &'a Arc<dyn NotificationPort>,
    pub _features: &'a Arc<dyn FeatureRepository>,
    pub agent_exec: &'a Arc<dyn AgentExecutionPort>,
    pub app_settings: &'a Arc<dyn AppSettingsRepository>,
    pub git_ops: &'a GitOpsHelper,
    pub feature_id: &'a FeatureId,
    pub resolved_cwd: &'a str,
    pub machine_str: &'a str,
    pub feature_branch: &'a str,
    pub base_branch: &'a str,
    pub conflict_files: &'a [String],
    pub step_execution_id: &'a StepExecutionId,
    pub thread_id_prefix: &'a str,
    pub agent_kind: &'a str,
    pub override_model: Option<&'a str>,
    /// The run's resolved effort. Resolving a merge conflict is real
    /// reasoning work, so it inherits rather than being pinned like the
    /// verifier / triage / finalize turns.
    pub effort: crate::domain::models::EffortLevel,
    pub pricing: &'a Arc<dyn PricingTable>,
}

pub(crate) async fn resolve_sync_conflicts_shared(
    sync_ctx: ResolveSyncContext<'_>,
) -> Result<String, String> {
    let ResolveSyncContext {
        exec,
        registry,
        notif,
        _features,
        agent_exec,
        app_settings,
        git_ops,
        feature_id,
        resolved_cwd,
        machine_str,
        feature_branch,
        base_branch,
        conflict_files,
        step_execution_id,
        thread_id_prefix,
        agent_kind,
        override_model,
        effort,
        pricing,
    } = sync_ctx;

    let fid = feature_id;

    // Safety check: is a merge actually active?
    let pre_unmerged = list_unmerged_files(&**exec, machine_str, resolved_cwd).await;
    let merge_in_progress = exec
        .run_command(
            machine_str,
            &format!(
                "git -C {} rev-parse --verify MERGE_HEAD",
                paths::shell_escape_posix(resolved_cwd)
            ),
        )
        .await
        .is_ok();

    if pre_unmerged.is_empty() && !merge_in_progress {
        return Err("No active merge in progress. Please run 'Sync with main' first.".to_string());
    }

    // Spawn a fresh agent session.
    let resolver_thread_id = format!("{}-{}", thread_id_prefix, paths::now_ms());
    // Every supported agent is a CLI runtime that takes its model via the
    // `--model` flag in `build_args` from `ctx.model` below.
    let agent_env = crate::ports::agent_runtime::agent_base_env(exec.as_ref(), machine_str).await;
    let platform =
        crate::ports::agent_runtime::resolve_agent_platform(exec.as_ref(), machine_str).await;

    let binary = registry
        .runtime_for(agent_kind)
        .map(|r| r.binary().to_string())
        .unwrap_or_else(|| agent_kind.to_string());
    let ctx = AgentContext {
        thread_id: resolver_thread_id.clone(),
        machine_id: machine_str.to_string(),
        binary,
        args: vec![],
        env: agent_env,
        cwd: resolved_cwd.to_string(),
        model: override_model.map(str::to_string),
        effort: Some(effort),
        title: Some("Sync conflict resolver".to_string()),
        platform,
        agent_exec: agent_exec.clone(),
        exec: exec.clone(),
        permissions: crate::domain::permission::PermissionProfile::all_allow(),
        bare_mode: true,
        tool_allowlist: None,
        max_turns: None,
        // Standalone resolver path (no driver budget in scope); uncapped like
        // its turn count.
        max_budget_usd: None,
    };

    let session = registry
        .get_or_spawn(&resolver_thread_id, agent_kind, ctx)
        .await
        .map_err(|e| format!("Failed to spawn resolver agent: {}", e))?;

    let prompt = build_resolver_prompt(feature_branch, base_branch, conflict_files);

    let timeouts = crate::application::timeouts::resolve_effective(app_settings.as_ref());

    let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
        &*session,
        &prompt,
        timeouts,
        None, // No cancel watch for resolver agent
        machine_str,
        &**exec,
        override_model.map(str::to_string),
        pricing.clone(),
        |event| {
            if let AgentEvent::Text { delta } = event {
                let _ = notif.emit(&DomainEvent::AgentStream {
                    feature_id: fid.clone(),
                    step_execution_id: step_execution_id.clone(),
                    content: delta.clone(),
                });
            }
        },
    )
    .await;

    match turn_res {
        crate::adapters::agent::event_stream::TurnResult::Interrupted => {
            let _ = registry.kill(&resolver_thread_id).await;
            return Err("Resolver execution interrupted".to_string());
        }
        crate::adapters::agent::event_stream::TurnResult::Failed(descriptive)
        | crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => {
            let _ = registry.kill(&resolver_thread_id).await;
            return Err(descriptive);
        }
        crate::adapters::agent::event_stream::TurnResult::Success(_) => {}
    }

    // The agent's worktree fence deliberately excludes the linked-worktree
    // index. Demeteo owns staging and committing after the agent resolves
    // the conflicted content.
    if let Err(reason) =
        ensure_conflict_markers_removed(&**exec, machine_str, resolved_cwd, &pre_unmerged).await
    {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(reason);
    }

    let conflict_paths = pre_unmerged
        .iter()
        .map(|file| paths::shell_escape_posix(&file.path))
        .collect::<Vec<_>>()
        .join(" ");
    if let Err(e) = exec
        .run_command(
            machine_str,
            &format!(
                "git -C {} add -- {}",
                paths::shell_escape_posix(resolved_cwd),
                conflict_paths,
            ),
        )
        .await
    {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(format!("Failed to stage conflict resolution: {}", e));
    }

    // Staging turns Git's unmerged index entries into the resolved files;
    // this is the authoritative completion check, independent of agent kind.
    let still_unmerged = list_unmerged_files(&**exec, machine_str, resolved_cwd).await;
    if !still_unmerged.is_empty() {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err("Resolver did not resolve every conflicted file.".to_string());
    }

    let message = format!("chore: resolve sync conflicts with origin/{}", base_branch);
    if let Err(rejection) = git_ops
        .validate_commit_message(
            if machine_str == crate::domain::ids::LOCAL_MACHINE {
                None
            } else {
                Some(machine_str)
            },
            resolved_cwd,
            &message,
        )
        .await
    {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(format!(
            "The repository's commit-msg hook rejected the sync-resolution commit: {}",
            rejection.hook_output
        ));
    }

    // The hook has already accepted this exact message above, matching the
    // finalize flow's validate-then-commit split. Avoid rerunning arbitrary
    // repository hooks after the merge has been staged.
    let commit_resolved = exec
        .run_command(
            machine_str,
            &format!(
                "{} -c user.email=demeteo@local -c user.name=demeteo commit -m {}",
                paths::git_no_hooks(resolved_cwd),
                paths::shell_escape_posix(&message),
            ),
        )
        .await;
    if let Err(e) = commit_resolved {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err(format!("Failed to commit resolution: {}", e));
    }

    // Push the resolution to origin remote.
    exec.run_command(
        machine_str,
        &format!(
            "git -C {} push origin {}",
            paths::shell_escape_posix(resolved_cwd),
            paths::shell_escape_posix(feature_branch),
        ),
    )
    .await
    .map_err(|e| {
        format!(
            "Resolution committed locally but push to origin/{} failed: {}. Push the feature branch manually.",
            feature_branch, e
        )
    })?;

    let _ = registry.kill(&resolver_thread_id).await;

    // Capture the new HEAD sha.
    let head_sha = exec
        .run_command(
            machine_str,
            &format!(
                "git -C {} rev-parse HEAD",
                paths::shell_escape_posix(resolved_cwd)
            ),
        )
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    Ok(head_sha)
}

async fn ensure_conflict_markers_removed(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    worktree: &str,
    conflict_files: &[crate::domain::models::ConflictFile],
) -> Result<(), String> {
    for file in conflict_files {
        let path = worktree_file_path(machine_str, worktree, &file.path);
        let content = exec
            .read_file(machine_str, &path)
            .await
            .map_err(|e| format!("Failed to read resolved conflict file {}: {}", file.path, e))?;
        if has_conflict_marker(&content) {
            return Err(format!(
                "Resolver left merge conflict markers in {}.",
                file.path
            ));
        }
    }
    Ok(())
}

fn worktree_file_path(machine_str: &str, worktree: &str, relative_path: &str) -> String {
    if machine_str == crate::domain::ids::LOCAL_MACHINE {
        return std::path::Path::new(worktree)
            .join(relative_path)
            .to_string_lossy()
            .into_owned();
    }
    format!("{}/{}", worktree.trim_end_matches('/'), relative_path)
}

fn has_conflict_marker(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("<<<<<<<")
            || trimmed.starts_with("=======")
            || trimmed.starts_with(">>>>>>>")
            || trimmed.starts_with("|||||||")
    })
}

impl DagStepExecutor {
    /// Tauri entry point for the "Sync with main" command. Resolves
    /// the feature branch + project state, asks the merge executor to
    /// do the actual git work, and translates the result into a
    /// `SyncOutcomeView` for the UI.
    pub(crate) async fn feature_sync_impl(
        &self,
        feature_id: &str,
        _revalidate_step_execution_id: Option<&str>,
    ) -> Result<SyncOutcomeView, String> {
        let fid = FeatureId::from(feature_id.to_string());
        let feature = self
            .features
            .get(&fid)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;

        let settings = self
            .projects
            .get_settings(&feature.project_id)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        let base_branch = sync_base(&feature, &settings)?;
        let feature_branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);

        match self
            .merge_executor
            .sync_feature_with_upstream(&fid, &feature_branch, &base_branch)
            .await
        {
            Ok(outcome) => Ok(SyncOutcomeView::Ok {
                merge_commit_sha: outcome.merge_commit_sha,
                changed: outcome.changed,
            }),
            Err(failure) => Ok(SyncOutcomeView::Conflict {
                conflict_files: failure.report.files,
                raw_error: failure.report.raw_error,
            }),
        }
    }

    /// Tauri entry point for the "Resolve with agent" button. Spawns
    /// a fresh agent session dedicated to the conflict, waits for it
    /// to commit a resolution, and (optionally) replays the named
    /// step so the workflow's validation re-runs on the merged tree.
    pub(crate) async fn feature_resolve_sync_conflicts_impl(
        &self,
        feature_id: &str,
        conflict_files: &[String],
        revalidate_step_execution_id: Option<&str>,
    ) -> Result<SyncOutcomeView, String> {
        let fid = FeatureId::from(feature_id.to_string());
        let feature = self
            .features
            .get(&fid)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;

        let settings = self
            .projects
            .get_settings(&feature.project_id)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        let base_branch = sync_base(&feature, &settings)?;
        let feature_branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);

        // Resolve the project / machine / repo dir for the agent's cwd.
        let (machine_id_opt, repo_dir) = self
            .resolve_repo_dir(&fid)
            .await
            .map_err(|e| format!("Failed to resolve repo dir: {}", e))?;
        let machine_str = machine_id_opt
            .clone()
            .unwrap_or_else(|| crate::domain::ids::LOCAL_MACHINE.to_string());

        // The merge executor's `sync_feature_with_upstream` left
        // the feature in a conflicted state. The conflict lives in
        // a sync worktree (if one was provisioned) or, as fallback,
        // in the main repo's checkout.
        //
        // If we're using the main repo, ensure it's on the correct
        // branch so the merge state is accessible.
        // Try to retrieve the worktree path from the last sync conflict report.
        let resolved_cwd = match self.merge_executor.get_last_sync_worktree_path(&fid).await {
            Ok(Some(wt_path)) => {
                let path_exists = self.exec.get_metadata(&machine_str, &wt_path).await.is_ok();
                if path_exists {
                    wt_path
                } else {
                    let _ = self
                        .exec
                        .run_command(
                            &machine_str,
                            &format!(
                                "git -C {} checkout {}",
                                paths::shell_escape_posix(&repo_dir),
                                paths::shell_escape_posix(&feature_branch)
                            ),
                        )
                        .await;
                    repo_dir.clone()
                }
            }
            _ => {
                let _ = self
                    .exec
                    .run_command(
                        &machine_str,
                        &format!(
                            "git -C {} checkout {}",
                            paths::shell_escape_posix(&repo_dir),
                            paths::shell_escape_posix(&feature_branch)
                        ),
                    )
                    .await;
                repo_dir.clone()
            }
        };

        let agent_kind = feature
            .agent_kind
            .clone()
            .unwrap_or_else(|| "opencode".to_string());
        let override_model = feature.model.clone();
        // No driver is running here (this is the "Resolve with agent" button),
        // so walk what the feature row + project settings know: the run
        // override, then the project default, then the built-in high.
        let effort = feature
            .effort
            .or(settings.default_effort)
            .unwrap_or(crate::domain::models::EffortLevel::DEFAULT);

        let step_exec_id = StepExecutionId::from(format!("se-sync-{}", paths::now_ms()));
        match resolve_sync_conflicts_shared(ResolveSyncContext {
            exec: &self.exec,
            registry: &self.registry,
            notif: &self.notif,
            _features: &self.features,
            agent_exec: &self.agent_exec,
            app_settings: &self.app_settings,
            git_ops: &self.git_ops,
            feature_id: &fid,
            resolved_cwd: &resolved_cwd,
            machine_str: &machine_str,
            feature_branch: &feature_branch,
            base_branch: &base_branch,
            conflict_files,
            step_execution_id: &step_exec_id,
            thread_id_prefix: SYNC_RESOLVER_THREAD_PREFIX,
            agent_kind: &agent_kind,
            override_model: override_model.as_deref(),
            effort,
            pricing: &self.pricing,
        })
        .await
        {
            Ok(head_sha) => {
                discard_sync_worktree(&*self.exec, &machine_str, &repo_dir, &resolved_cwd).await;

                // After a successful resolution, replay the validation step
                if let Some(se_id) = revalidate_step_execution_id {
                    if let Err(e) = self.replay_from_step(se_id, None, None, None).await {
                        return Err(format!(
                            "Resolution succeeded but re-validate failed: {}",
                            e
                        ));
                    }
                }

                Ok(SyncOutcomeView::Resolved {
                    merge_commit_sha: head_sha,
                    revalidated_step_id: revalidate_step_execution_id.map(|s| s.to_string()),
                })
            }
            Err(reason) => {
                let conflict_list =
                    list_unmerged_files(&*self.exec, &machine_str, &resolved_cwd).await;
                Ok(SyncOutcomeView::ResolutionFailed {
                    reason,
                    conflict_files: conflict_list,
                })
            }
        }
    }

    /// Resolve the absolute local repo dir + machine for a feature.
    ///
    /// The `repositories.repo_path` column holds the provider-side
    /// slug (e.g. `"gitops/terraform-dev-containers"`) — that is
    /// not a path on disk. We have to translate it through
    /// [`crate::paths::repo_target_dir_str`] which knows the local
    /// home + projects + repos layout. Skipping that translation
    /// (which is what the old version of this method did) made
    /// `git -C <path>` fail with `cannot change to ...` whenever
    /// the resolver tried to provision a worktree.
    async fn resolve_repo_dir(
        &self,
        feature_id: &FeatureId,
    ) -> Result<(Option<String>, String), String> {
        let feature = self
            .features
            .get(feature_id)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id.0))?;
        let project = self
            .projects
            .get_projects()?
            .into_iter()
            .find(|p| p.id == feature.project_id)
            .ok_or_else(|| format!("Project not found for feature: {}", feature_id.0))?;
        let repo = self
            .projects
            .get_repositories_for(&project.id)?
            .first()
            .cloned()
            .ok_or_else(|| "Project has no repositories configured.".to_string())?;
        let machine = if project.compute_type.to_lowercase() == "local" {
            None
        } else {
            project.remote_host.as_ref().map(|m| m.0.clone())
        };
        let target_dir = if project.compute_type.to_lowercase() == "local" {
            crate::paths::repo_target_dir_local(
                &self.workspace_dir,
                project.id.0.as_str(),
                &repo.repo_path,
            )
            .to_string_lossy()
            .to_string()
        } else {
            crate::paths::repo_target_dir_str(
                &self.exec,
                &project.compute_type,
                project.remote_host.as_ref().map(|m| m.as_str()),
                project.id.0.as_str(),
                &repo.repo_path,
                None,
            )
            .await?
        };
        Ok((machine, target_dir))
    }
}

/// Build the constrained prompt for the conflict-resolution agent.
/// The agent is told exactly which files to edit and explicitly
/// forbidden from touching anything else — keeps the cost low and
/// the resolution deterministic.
fn build_resolver_prompt(
    feature_branch: &str,
    base_branch: &str,
    conflict_files: &[String],
) -> String {
    let files_list = conflict_files
        .iter()
        .map(|f| format!("- {}", f))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "We just merged origin/{base} into {feature}. A merge conflict was detected.\n\
         Please resolve the conflicts in the following files:\n\
         {files}\n\n\
         For each file:\n\
         - Read the conflict markers (<<<<<<<, =======, >>>>>>>).\n\
         - Integrate the changes from both sides correctly.\n\
         - Remove all conflict markers.\n\
         - Do NOT modify any other file or any other part of the listed files.\n\
         - When done, run the project's build / test suite to confirm nothing is broken.\n\
         - Do NOT stage or commit — Demeteo validates, stages, and commits the resolution.\n\
         - Report back with a one-line summary when you're done.",
        base = base_branch,
        feature = feature_branch,
        files = files_list,
    )
}

#[cfg(test)]
mod tests {
    use super::has_conflict_marker;

    #[test]
    fn merge_markers_are_rejected_before_demeteo_stages_the_resolution() {
        assert!(has_conflict_marker(
            "const value = 1;\n<<<<<<< HEAD\nconst branch = 'feature';\n=======\nconst branch = 'main';\n>>>>>>> origin/master\n"
        ));
        assert!(!has_conflict_marker("const value = 1;\n"));
    }
}
