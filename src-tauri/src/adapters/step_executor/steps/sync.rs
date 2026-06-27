//! The `sync` step kind. A workflow node that:
//!
//! 1. Fetches the latest `origin/<default_branch>`.
//! 2. Merges it into the feature branch.
//! 3. On a clean merge, completes (invisible cost when nothing changed).
//! 4. On a conflict, spawns a fresh agent to resolve, then redirects
//!    to the configured validation step (via `on_failure`) so the
//!    workflow re-runs validation on the freshly-merged tree.
//!
//! The step is opt-in: workflows that don't include a `sync` node
//! behave exactly as before. The `on_failure` redirect is what makes
//! the re-validate loop work — it points at the step that should be
//! replayed after a successful resolution.

use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

use super::StepOutcome;

impl ExecutionDriver {
    /// Handle a `kind == "sync"` step.
    ///
    /// Returns:
    /// - `StepOutcome::Completed` when the merge was clean (or there
    ///   was nothing to merge).
    /// - `StepOutcome::Failed(msg)` when the merge produced conflicts
    ///   that the resolution agent could not clean up. The driver
    ///   will route this through `on_failure` if the step declared
    ///   one (so the workflow can redirect to re-validate).
    /// - `StepOutcome::RedirectTo(idx)` when the resolution succeeded
    ///   and the workflow should jump to a different step (the
    ///   validation step declared via `on_failure`).
    pub(crate) async fn handle_sync_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        step_start: Instant,
    ) -> StepOutcome {
        // Persist the step as running.
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                iteration_count: None,
                status: Some("running".to_string()),
                cost_usd: Some(Some(*accumulated_cost)),
                tokens: None,
                wall_clock_secs: Some(Some(0)),
                artifact_path: None,
                artifact_paths: None,
                error_message: Some(None),
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "running".into(),
            cost_usd: Some(*accumulated_cost),
            tokens: None,
            wall_clock_secs: Some(0),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });

        // Resolve the project settings so we know which branch is the
        // default. We can't reach `ProjectSettings` from the driver
        // directly, so we look it up via the executor's project
        // repository. The driver is created by the executor which
        // already has the project settings cached, so this is cheap.
        let feature = match self.features.get(&self.f_id) {
            Ok(Some(f)) => f,
            _ => return StepOutcome::Failed("Feature not found for sync step".to_string()),
        };
        let settings = match self.projects.get_settings(&feature.project_id) {
            Ok(Some(s)) => s,
            _ => {
                return StepOutcome::Failed("Project settings not found for sync step".to_string())
            }
        };
        let default_branch = settings.worktree_strategy.default_branch.clone();
        let branch_prefix = settings.worktree_strategy.branch_prefix.clone();
        let feature_branch = format!("{}{}", branch_prefix, self.f_id.as_str());

        // Run the merge. A clean merge is the trivial path; conflicts
        // are routed to the resolution agent.
        match self
            .merge_executor
            .sync_feature_with_upstream(&self.f_id, &feature_branch, &default_branch)
            .await
        {
            Ok(outcome) => {
                let wall = step_start.elapsed().as_secs();
                let _ = self.features.step_update(
                    &step_exec.id,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some("completed".to_string()),
                        cost_usd: Some(Some(*accumulated_cost)),
                        tokens: None,
                        wall_clock_secs: Some(Some(wall)),
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: Some(None),
                    },
                );
                let _ = self.notif.emit(&DomainEvent::StepProgress {
                    feature_id: self.f_id.clone(),
                    step_id: step_exec.step_id.0.clone(),
                    status: "completed".into(),
                    cost_usd: Some(*accumulated_cost),
                    tokens: None,
                    wall_clock_secs: Some(wall),
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                });
                let _ = outcome.merge_commit_sha;
                StepOutcome::Completed
            }
            Err(failure) => {
                // Conflicts. Spawn the resolution agent in the sync
                // worktree (or the main repo as fallback). If the
                // agent succeeds, redirect to the validation step.
                let _ = accumulated_cost;
                self.resolve_sync_conflicts_in_step(
                    step_exec,
                    step_conf,
                    &failure.report.files,
                    &feature_branch,
                    &default_branch,
                    failure.worktree_path.as_deref(),
                    step_start,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn resolve_sync_conflicts_in_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        conflict_files: &[crate::domain::models::ConflictFile],
        feature_branch: &str,
        default_branch: &str,
        worktree_path: Option<&str>,
        step_start: Instant,
    ) -> StepOutcome {
        let machine_str = self.machine_id_opt.as_deref().unwrap_or("local");
        let repo_dir = &self.target_dir;
        let resolved_cwd = worktree_path.unwrap_or(repo_dir);

        let feature = match self.features.get(&self.f_id) {
            Ok(Some(f)) => f,
            _ => return StepOutcome::Failed("Feature not found for sync step".to_string()),
        };

        let agent_kind = step_conf
            .agent_kind
            .clone()
            .or_else(|| feature.agent_kind.clone())
            .unwrap_or_else(|| "opencode".to_string());
        let override_model = feature.model.clone();

        let conflict_paths: Vec<String> = conflict_files.iter().map(|f| f.path.clone()).collect();

        match crate::adapters::step_executor::sync::resolve_sync_conflicts_shared(
            crate::adapters::step_executor::sync::ResolveSyncContext {
                exec: &self.exec,
                registry: &self.registry,
                notif: &self.notif,
                _features: &self.features,
                agent_exec: &self.agent_exec,
                feature_id: &self.f_id,
                resolved_cwd,
                machine_str,
                feature_branch,
                default_branch,
                conflict_files: &conflict_paths,
                step_execution_id: &step_exec.id,
                thread_id_prefix: "sync-step-resolver",
                agent_kind: &agent_kind,
                override_model: &override_model,
                pricing: &self.pricing,
            },
        )
        .await
        {
            Ok(_head_sha) => {
                // Cleanup the sync worktree if one was used.
                if resolved_cwd != repo_dir {
                    let _ = self
                        .exec
                        .run_command(
                            machine_str,
                            &format!(
                                "git -C {} worktree remove --force {}",
                                paths::shell_escape_posix(repo_dir),
                                paths::shell_escape_posix(resolved_cwd)
                            ),
                        )
                        .await;
                    let _ = self
                        .exec
                        .run_command(
                            machine_str,
                            &format!("rm -rf {}", paths::shell_escape_posix(resolved_cwd)),
                        )
                        .await;
                    let _ = self
                        .exec
                        .run_command(
                            machine_str,
                            &format!(
                                "git -C {} worktree prune",
                                paths::shell_escape_posix(repo_dir)
                            ),
                        )
                        .await;
                }

                let wall = step_start.elapsed().as_secs();
                let _ = self.features.step_update(
                    &step_exec.id,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some("completed".to_string()),
                        cost_usd: None,
                        tokens: None,
                        wall_clock_secs: Some(Some(wall)),
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: Some(None),
                    },
                );
                let _ = self.notif.emit(&DomainEvent::StepProgress {
                    feature_id: self.f_id.clone(),
                    step_id: step_exec.step_id.0.clone(),
                    status: "completed".into(),
                    cost_usd: None,
                    tokens: None,
                    wall_clock_secs: Some(wall),
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                });

                let target = step_conf
                    .on_failure
                    .as_ref()
                    .map(|id| id.0.clone())
                    .unwrap_or_default();
                if let Some(target_idx) = self.steps.iter().position(|s| s.id.0 == target) {
                    StepOutcome::RedirectTo(target_idx)
                } else {
                    StepOutcome::Completed
                }
            }
            Err(reason) => {
                let wall = step_start.elapsed().as_secs();
                let _ = self.features.step_update(
                    &step_exec.id,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some("failed".to_string()),
                        cost_usd: None,
                        tokens: None,
                        wall_clock_secs: Some(Some(wall)),
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: Some(Some(reason.clone())),
                    },
                );
                let _ = self.notif.emit(&DomainEvent::StepProgress {
                    feature_id: self.f_id.clone(),
                    step_id: step_exec.step_id.0.clone(),
                    status: "failed".into(),
                    cost_usd: None,
                    tokens: None,
                    wall_clock_secs: Some(wall),
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                });
                StepOutcome::Failed(reason)
            }
        }
    }
}
