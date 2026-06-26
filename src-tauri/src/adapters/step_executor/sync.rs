//! Feature-branch sync with the upstream `default_branch`.
//!
//! Two Tauri commands surface this code path:
//!
//! - `feature_sync`: merges `origin/<default>` into the feature
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
use crate::domain::agent_event::AgentEvent;
use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::ConflictFile;
use crate::paths;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::FeatureRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::DomainEvent;
use crate::ports::notification::NotificationPort;
use crate::ports::step_executor::{StepExecutor, SyncOutcomeView};

use super::DagStepExecutor;

/// The thread-id suffix for the conflict-resolution agent. We use a
/// fresh id (not `feature_id`) so the resolution session is fully
/// independent from the step-execution agent session that drove the
/// implementation: the resolver gets a clean prompt and its own
/// `OPENCODE_PERMISSION` scope.
const SYNC_RESOLVER_THREAD_PREFIX: &str = "sync-resolver";

/// Hard cap on the resolution agent's wall-clock time. Conflict
/// resolution is mechanical (remove markers, build, test) and rarely
/// needs more than a few minutes; the cap is generous to keep the
/// UI from hanging on truly stuck agents.
const RESOLVER_WALL_CAP_S: u64 = 600;
const RESOLVER_FAST_TIMEOUT_S: u64 = 180;
const RESOLVER_NORMAL_TIMEOUT_S: u64 = 180;

/// Unified sync conflict resolver helper. Drives the conflict resolution agent,
/// streams UI status events, monitors timeouts, verifies conflict markers,
/// commits the resolution, and pushes it to remote origin.
pub(crate) struct ResolveSyncContext<'a> {
    pub exec: &'a Arc<dyn ExecutionPort>,
    pub registry: &'a Arc<AgentRegistry>,
    pub notif: &'a Arc<dyn NotificationPort>,
    pub _features: &'a Arc<dyn FeatureRepository>,
    pub agent_exec: &'a Arc<dyn AgentExecutionPort>,
    pub feature_id: &'a FeatureId,
    pub resolved_cwd: &'a str,
    pub machine_str: &'a str,
    pub feature_branch: &'a str,
    pub default_branch: &'a str,
    pub conflict_files: &'a [String],
    pub step_execution_id: &'a StepExecutionId,
    pub thread_id_prefix: &'a str,
    pub agent_kind: &'a str,
    pub override_model: &'a Option<String>,
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
        feature_id,
        resolved_cwd,
        machine_str,
        feature_branch,
        default_branch,
        conflict_files,
        step_execution_id,
        thread_id_prefix,
        agent_kind,
        override_model,
    } = sync_ctx;

    let fid = feature_id;

    // Safety check: is a merge actually active?
    let pre_unmerged = list_unmerged(&**exec, machine_str, resolved_cwd).await;
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
    let mut agent_env = crate::ports::agent_runtime::agent_base_env();
    if let Some(ref m) = override_model {
        if agent_kind != "opencode"
            && agent_kind != "hermes"
            && agent_kind != "claude-code"
            && agent_kind != "antigravity"
        {
            let config = format!(
                r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                m
            );
            agent_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
        }
    }

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
        model: override_model.clone(),
        title: Some("Sync conflict resolver".to_string()),
        agent_exec: agent_exec.clone(),
        exec: exec.clone(),
    };

    let session = registry
        .get_or_spawn(&resolver_thread_id, agent_kind, ctx)
        .await
        .map_err(|e| format!("Failed to spawn resolver agent: {}", e))?;

    let prompt = build_resolver_prompt(feature_branch, default_branch, conflict_files);

    let timeouts = crate::adapters::agent::event_stream::Timeouts {
        fast_timeout_s: RESOLVER_FAST_TIMEOUT_S,
        normal_timeout_s: RESOLVER_NORMAL_TIMEOUT_S,
        wall_cap_s: RESOLVER_WALL_CAP_S,
    };

    let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
        &*session,
        &prompt,
        timeouts,
        None, // No cancel watch for resolver agent
        machine_str,
        &**exec,
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
        crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
            let _ = registry.kill(&resolver_thread_id).await;
            return Err(descriptive);
        }
        crate::adapters::agent::event_stream::TurnResult::Success(_) => {}
    }

    // Verify the agent actually removed the conflict markers.
    let still_unmerged = list_unmerged(&**exec, machine_str, resolved_cwd).await;
    if !still_unmerged.is_empty() {
        let _ = registry.kill(&resolver_thread_id).await;
        return Err("Resolver did not remove all conflict markers.".to_string());
    }

    // Commit the resolution.
    let commit_resolved = exec
        .run_command(
            machine_str,
            &format!(
                "git -C {} add -A && git -C {} commit -m \"Resolve sync conflicts with origin/{}\"",
                paths::shell_escape_posix(resolved_cwd),
                paths::shell_escape_posix(resolved_cwd),
                default_branch
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
        let default_branch = settings.worktree_strategy.default_branch.clone();
        let branch_prefix = settings.worktree_strategy.branch_prefix.clone();
        let feature_branch = format!("{}{}", branch_prefix, fid.as_str());

        match self
            .merge_executor
            .sync_feature_with_upstream(&fid, &feature_branch, &default_branch)
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
        let default_branch = settings.worktree_strategy.default_branch.clone();
        let branch_prefix = settings.worktree_strategy.branch_prefix.clone();
        let feature_branch = format!("{}{}", branch_prefix, fid.as_str());

        // Resolve the project / machine / repo dir for the agent's cwd.
        let (machine_id_opt, repo_dir) = self
            .resolve_repo_dir(&fid)
            .await
            .map_err(|e| format!("Failed to resolve repo dir: {}", e))?;
        let machine_str = machine_id_opt
            .clone()
            .unwrap_or_else(|| "local".to_string());

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

        let step_exec_id = StepExecutionId::from(format!("se-sync-{}", paths::now_ms()));
        match resolve_sync_conflicts_shared(ResolveSyncContext {
            exec: &self.exec,
            registry: &self.registry,
            notif: &self.notif,
            _features: &self.features,
            agent_exec: &self.agent_exec,
            feature_id: &fid,
            resolved_cwd: &resolved_cwd,
            machine_str: &machine_str,
            feature_branch: &feature_branch,
            default_branch: &default_branch,
            conflict_files,
            step_execution_id: &step_exec_id,
            thread_id_prefix: SYNC_RESOLVER_THREAD_PREFIX,
            agent_kind: &agent_kind,
            override_model: &override_model,
        })
        .await
        {
            Ok(head_sha) => {
                // Cleanup the sync worktree if one was used.
                if resolved_cwd != repo_dir {
                    let _ = self
                        .exec
                        .run_command(
                            &machine_str,
                            &format!(
                                "git -C {} worktree remove --force {}",
                                paths::shell_escape_posix(&repo_dir),
                                paths::shell_escape_posix(&resolved_cwd)
                            ),
                        )
                        .await;
                    let _ = self
                        .exec
                        .run_command(
                            &machine_str,
                            &format!("rm -rf {}", paths::shell_escape_posix(&resolved_cwd)),
                        )
                        .await;
                    let _ = self
                        .exec
                        .run_command(
                            &machine_str,
                            &format!(
                                "git -C {} worktree prune",
                                paths::shell_escape_posix(&repo_dir)
                            ),
                        )
                        .await;
                }

                // After a successful resolution, replay the validation step
                if let Some(se_id) = revalidate_step_execution_id {
                    if let Err(e) = self.replay_from_step(se_id, None, None).await {
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
                let conflict_list = list_unmerged(&*self.exec, &machine_str, &resolved_cwd).await;
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
    default_branch: &str,
    conflict_files: &[String],
) -> String {
    let files_list = conflict_files
        .iter()
        .map(|f| format!("- {}", f))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "We just merged origin/{default} into {feature}. A merge conflict was detected.\n\
         Please resolve the conflicts in the following files:\n\
         {files}\n\n\
         For each file:\n\
         - Read the conflict markers (<<<<<<<, =======, >>>>>>>).\n\
         - Integrate the changes from both sides correctly.\n\
         - Remove all conflict markers.\n\
         - Do NOT modify any other file or any other part of the listed files.\n\
         - When done, run the project's build / test suite to confirm nothing is broken.\n\
         - Stage your resolution (`git add -A`). Do NOT commit — the tool will commit for you.\n\
         - Report back with a one-line summary when you're done.",
        default = default_branch,
        feature = feature_branch,
        files = files_list,
    )
}

/// Walk `git status --porcelain` and pull out the unmerged paths.
async fn list_unmerged(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<ConflictFile> {
    let raw = match exec
        .run_command(
            machine_id,
            &format!(
                "git -C {} status --porcelain --untracked-files=no",
                paths::shell_escape_posix(repo_dir)
            ),
        )
        .await
    {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    raw.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
            let xy = &line[..2];
            let path = line[3..].trim().to_string();
            let kind = match xy {
                "UU" | "AA" | "DD" => "both-modified".to_string(),
                "UA" => "added-by-them".to_string(),
                "AU" => "added-by-us".to_string(),
                "UD" => "deleted-by-them".to_string(),
                "DU" => "deleted-by-us".to_string(),
                _ => return None,
            };
            Some(ConflictFile { path, kind })
        })
        .collect()
}
