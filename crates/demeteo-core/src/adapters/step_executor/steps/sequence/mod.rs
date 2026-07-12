//! The `sequence` step: run an ordered task list, one fresh agent per task,
//! all in a single worktree, and merge the result back once.
//!
//! This replaces the old `parallel` step, which fanned out one worker per
//! subtask across concurrent worktrees and merged each back independently.
//! That design cost more than it bought:
//!
//! * **Concurrent worktrees collide.** Worktree directories are derived from
//!   the repo dir, which every feature on a project shares. Provisioning
//!   force-removes its target, so one feature could delete a sibling's live
//!   worktree along with its uncommitted work.
//! * **N merges mean N chances to conflict.** Each subtask merged into the
//!   feature branch separately, so subtask 2 could conflict with subtask 1
//!   and needed an agent to resolve it mid-step.
//! * **Disjoint file ownership is a fiction.** The planner had to partition
//!   files up front so workers could not collide — but real work rarely
//!   partitions cleanly, and a task needing a file another task owned had no
//!   way to say so.
//!
//! Running the tasks in order in one worktree dissolves all three. There is
//! one worktree per (feature, step) — named with the same feature-scoped
//! convention agent steps use, so cross-feature collision is impossible.
//! There is one merge, at the end. And because each task commits before the
//! next begins, task N *sees* task N-1's work, so `files` need not be
//! disjoint and a later task can legitimately build on an earlier one.
//!
//! What survives from the old design is the part that actually helped: each
//! task gets a *fresh* agent session, so no single context window has to
//! carry the whole feature.

use std::time::Instant;

use crate::adapters::step_executor::artifacts::compute_git_diff;
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

pub(crate) mod plan;
pub(crate) mod runner;
pub(crate) mod tasks;

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_sequence_step(
        &mut self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        if *self.cancel_watch.borrow() {
            return StepOutcome::Cancelled;
        }

        let (agent_kind, override_model) = self.resolve_step_agent(step_conf);
        let machine_str = self
            .machine_id_opt
            .clone()
            .unwrap_or_else(|| "local".to_string());

        // Rollback anchor: the feature branch tip before this attempt. On
        // failure we reset the branch ref back to it so a retry starts clean.
        let base_sha = match self
            .exec
            .run_command(
                &machine_str,
                &format!(
                    "git -C {} rev-parse {}",
                    paths::shell_escape_posix(&self.target_dir),
                    paths::shell_escape_posix(&self.branch_name),
                ),
            )
            .await
        {
            Ok(s) => s.trim().to_string(),
            Err(e) => {
                return StepOutcome::Failed(format!(
                    "sequence step: could not capture base SHA for rollback anchor: {}",
                    e
                ))
            }
        };

        // 1. Resolve the ordered task list.
        let retry_iteration = self.retry_ctx.as_ref().map(|rc| rc.iteration).unwrap_or(0);
        let plan = match self
            .resolve_task_plan(
                step_exec,
                step_conf,
                accumulated_cost,
                accumulated_tokens,
                retry_iteration,
                &agent_kind,
                &override_model,
                &machine_str,
                step_execs,
                step_index,
            )
            .await
        {
            Ok(p) => p,
            Err(outcome) => return outcome,
        };

        if plan.tasks.is_empty() {
            return StepOutcome::Failed(
                "sequence step: the task list is empty — there is nothing to implement."
                    .to_string(),
            );
        }

        tracing::info!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            tasks = plan.tasks.len(),
            attempt = retry_iteration,
            "sequence step: running tasks in order"
        );

        // 2. One worktree for the whole step, feature-scoped exactly as an
        //    agent step's is. Two features on the same repo therefore get
        //    different directories, and nothing this step does can disturb a
        //    sibling feature's worktree.
        let wt_id = format!("{}-step-{}", self.f_id_str, step_exec.step_id.0);
        let wt_path = match self
            .git_ops
            .provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &wt_id,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return StepOutcome::Environmental(format!(
                    "sequence step: worktree provision failed ({}): {}",
                    wt_id, e
                ))
            }
        };

        // Scope fence. A no-op for `Implement` capability (whole worktree
        // writable), which is what a sequence step normally carries.
        if let Err(e) = self
            .git_ops
            .apply_artifact_scope(
                self.machine_id_opt.as_deref(),
                &wt_path,
                &self.sequence_writable_paths(step_conf),
            )
            .await
        {
            self.cleanup_sequence_worktree(&wt_id).await;
            return StepOutcome::Environmental(format!(
                "sequence step: artifact scope setup failed: {}",
                e
            ));
        }

        // 3. Run the tasks in order.
        let mut all_artifact_refs = Vec::new();
        let tasks_res = self
            .run_tasks_loop(
                step_exec,
                step_conf,
                accumulated_cost,
                accumulated_tokens,
                step_start,
                step_index,
                step_execs,
                &plan.tasks,
                &machine_str,
                &wt_path,
                &agent_kind,
                &override_model,
                &mut all_artifact_refs,
            )
            .await;

        if let Err((msg, environmental)) = tasks_res {
            self.cleanup_sequence_worktree(&wt_id).await;
            self.rollback_feature_branch(&machine_str, &base_sha).await;
            return self
                .fail_sequence_step(
                    step_exec,
                    step_start,
                    *accumulated_cost,
                    *accumulated_tokens,
                    msg,
                    environmental,
                )
                .await;
        }

        // 4. Verifier (harness / judge), if the step declares one. It runs
        //    against the worktree — where every task's commits already are —
        //    so it sees the complete change before anything reaches the
        //    feature branch.
        if let Some(ref verifier_cfg) = step_conf.verifier {
            let verifier_result = self
                .run_verifier_logic(
                    step_exec,
                    verifier_cfg,
                    &wt_path,
                    &[],
                    accumulated_cost,
                    accumulated_tokens,
                    step_start,
                    &agent_kind,
                    &override_model,
                    &machine_str,
                )
                .await;

            if let Err(verifier_err) = verifier_result {
                let _ = self
                    .registry
                    .kill(&format!("{}-verifier", self.f_id.as_str()))
                    .await;
                self.cleanup_sequence_worktree(&wt_id).await;
                self.rollback_feature_branch(&machine_str, &base_sha).await;
                return match verifier_err {
                    crate::domain::verifier::VerifierError::Verdict(failure) => {
                        StepOutcome::VerdictFailed(failure)
                    }
                    crate::domain::verifier::VerifierError::Infrastructure(msg) => {
                        StepOutcome::NonRetryable(format!(
                            "[verifier infrastructure error — check verifier config] {}",
                            msg
                        ))
                    }
                    // Triaged as an environment problem: the box is not
                    // provisioned, editing source cannot fix it.
                    crate::domain::verifier::VerifierError::Environment(msg) => {
                        StepOutcome::NonRetryable(msg)
                    }
                };
            }
        }

        // 5. One merge, at the end. The tasks cannot have conflicted with
        //    each other (same worktree, sequential, each committed), so the
        //    only way this conflicts is the feature branch having moved
        //    beneath us — e.g. a `sync` step pulled upstream in a prior step.
        if let Err(e) = self
            .git_ops
            .merge_subtask(
                self.machine_id_opt.as_deref(),
                &wt_path,
                &self.branch_name,
                &wt_id,
            )
            .await
        {
            let _ = self.notif.emit(&DomainEvent::ConflictDetected {
                feature_id: self.f_id.clone(),
                subtask_id: format!("{}_subtask_{}", self.branch_name, wt_id),
            });
            self.cleanup_sequence_worktree(&wt_id).await;
            self.rollback_feature_branch(&machine_str, &base_sha).await;
            return self
                .fail_sequence_step(
                    step_exec,
                    step_start,
                    *accumulated_cost,
                    *accumulated_tokens,
                    format!(
                        "sequence step: merging the completed task branch into '{}' failed: {}",
                        self.branch_name, e
                    ),
                    false,
                )
                .await;
        }

        // 6. Summary artifact: the whole feature's diff, computed from the
        //    fork point rather than this attempt's base, so a retry's critic
        //    reviews the complete change and not just the incremental fix.
        //
        //    Two-dot range against `target_dir` rather than a single-ref
        //    `git diff`: `target_dir` sits on the default branch for the
        //    whole run (the feature branch is only ever a ref), so a
        //    single-ref diff would compare the default branch's working tree
        //    against the range start and render the implementation as
        //    additions that exist in commits but not on disk — which reads as
        //    "the code was committed then reverted".
        let diff_ref = match self.resolve_fork_point_ref(&machine_str).await {
            Some(fork_point) => format!("{}..{}", fork_point, self.branch_name),
            None => format!("{}..{}", base_sha, self.branch_name),
        };
        let diff_body =
            compute_git_diff(&*self.exec, &machine_str, &self.target_dir, &diff_ref).await;
        let mut refs = Vec::new();
        if !diff_body.trim().is_empty() {
            let diff_artifact = Artifact {
                name: "code-diff".into(),
                mime: "text/x-diff".into(),
                content: diff_body,
                source: crate::domain::artifact::ArtifactSource::Diff {
                    base: base_sha.clone(),
                    head: self.branch_name.clone(),
                    path_filter: None,
                },
            };
            if let Ok(reference) =
                self.artifacts
                    .put(&self.f_id_str, &step_exec.step_id.0, &diff_artifact)
            {
                refs.push(reference);
            }
        }
        refs.extend(all_artifact_refs);

        self.cleanup_sequence_worktree(&wt_id).await;

        // Every task ran and committed, yet the branch carries nothing.
        //
        // On a retry that is legitimate: a targeted re-run whose fix was
        // already in place is a no-op, and there are prior-attempt artifacts
        // worth keeping in scope for the critic. On the first attempt it
        // means the implementation produced nothing at all — and reporting
        // `completed` there is exactly what used to hand a green status to
        // `s-validate`, which then correctly found the feature unimplemented.
        // Fail instead.
        if refs.is_empty() {
            if retry_iteration == 0 {
                return self
                    .fail_sequence_step(
                        step_exec,
                        step_start,
                        *accumulated_cost,
                        *accumulated_tokens,
                        "sequence step: every task completed but the feature branch carries no \
                         changes — the implementation produced nothing."
                            .to_string(),
                        false,
                    )
                    .await;
            }
            refs = step_exec.artifact_paths.clone();
        }

        let wall = step_start.elapsed().as_secs();
        let primary = refs.first().cloned();
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some("completed".to_string()),
                cost_usd: Some(Some(*accumulated_cost)),
                tokens: Some(Some(*accumulated_tokens)),
                wall_clock_secs: Some(Some(wall)),
                artifact_path: Some(primary),
                artifact_paths: Some(refs),
                error_message: Some(None),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "completed".into(),
            cost_usd: Some(*accumulated_cost),
            tokens: Some(*accumulated_tokens),
            wall_clock_secs: Some(wall),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
        StepOutcome::Completed
    }

    /// Reset the feature branch ref back to `base_sha`.
    ///
    /// A ref-only `branch -f`, never a `reset --hard` in `target_dir`: the
    /// main repo stays checked out on the project's default branch for the
    /// whole run, so a hard reset there would shove the *default* branch to
    /// the feature tip. The feature branch is not checked out in the main
    /// repo, so `branch -f` moves only its ref.
    async fn rollback_feature_branch(&self, machine_str: &str, base_sha: &str) {
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} branch -f {} {}",
                    paths::shell_escape_posix(&self.target_dir),
                    paths::shell_escape_posix(&self.branch_name),
                    base_sha,
                ),
            )
            .await;
    }

    async fn cleanup_sequence_worktree(&self, wt_id: &str) {
        let _ = self
            .git_ops
            .cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                wt_id,
            )
            .await;
    }

    /// Persist the failure and translate it into the right outcome. Honors
    /// cancellation: a step the user interrupted is not a failure.
    async fn fail_sequence_step(
        &self,
        step_exec: &StepExecution,
        step_start: Instant,
        cost: f64,
        tokens: i64,
        msg: String,
        environmental: bool,
    ) -> StepOutcome {
        let is_cancelled = *self.cancel_watch.borrow();
        let status_str = if is_cancelled {
            "interrupted"
        } else {
            "failed"
        };
        let wall = step_start.elapsed().as_secs();
        let stored = if is_cancelled {
            format!(
                "{} (the step's task commits have been rolled back for a clean retry)",
                msg
            )
        } else {
            msg.clone()
        };
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some(status_str.to_string()),
                cost_usd: Some(Some(cost)),
                tokens: Some(Some(tokens)),
                wall_clock_secs: Some(Some(wall)),
                artifact_path: None,
                artifact_paths: None,
                error_message: Some(Some(stored)),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: status_str.into(),
            cost_usd: Some(cost),
            tokens: Some(tokens),
            wall_clock_secs: Some(wall),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });

        if is_cancelled {
            return StepOutcome::Cancelled;
        }
        if environmental {
            return StepOutcome::Environmental(msg);
        }
        StepOutcome::Failed(msg)
    }
}
