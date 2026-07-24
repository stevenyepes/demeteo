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
use crate::adapters::step_executor::steps::conflict_pass::{ConflictPass, ConflictPassError};
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

pub(crate) mod plan;
pub(crate) mod runner;
pub(crate) mod tasks;

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/sequence/disposition.rs"]
mod disposition_tests;

/// What happened to the feature branch on the way out of a failed sequence
/// step. Folded into the stored error message, because the user has to know
/// the branch's state before they retry or ship — each variant leaves it
/// somewhere different.
enum FailureDisposition {
    /// The branch was reset to its pre-attempt tip; a retry starts clean.
    RolledBack,
    /// The reset failed (usually an unremovable worktree) and the failed
    /// attempt's commits are still on the branch.
    RollbackFailed,
    /// The tasks that completed before the failure were merged to the
    /// feature branch; the retry resumes from the failed task.
    PrefixLanded { landed: usize, total: usize },
}

impl FailureDisposition {
    fn from_rollback(rolled_back: bool) -> Self {
        if rolled_back {
            Self::RolledBack
        } else {
            Self::RollbackFailed
        }
    }

    /// Fold the branch's state into the failure message. A rollback that did
    /// not happen leaves the failed attempt's commits on the feature branch,
    /// and the user has to know that before they retry or ship — claiming a
    /// clean slate we did not deliver is worse than the failure itself. A
    /// checkpointed prefix is the deliberate version of the same situation:
    /// commits on the branch, but kept on purpose and resumed from on retry.
    fn decorate(&self, msg: &str, branch: &str) -> String {
        match self {
            Self::RolledBack => format!(
                "{} (the step's task commits have been rolled back for a clean retry)",
                msg
            ),
            Self::RollbackFailed => format!(
                "{} (WARNING: the step's task commits could NOT be rolled back and are still on \
                 branch '{}' — its worktree could not be removed. Inspect the branch before \
                 retrying.)",
                msg, branch
            ),
            Self::PrefixLanded { landed, total } => format!(
                "{} ({} of {} tasks completed before the failure; their commits were kept and \
                 merged to branch '{}', and a retry will resume from the failed task instead of \
                 starting over)",
                msg, landed, total, branch
            ),
        }
    }
}

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

        // A fresh attempt cannot have live task rows of its own, so any
        // `running` subtask_runs row for this step is a leftover from a
        // crashed or killed process. The startup watchdog closes these for
        // features it reconciles, but resume paths it skips (runner-owned
        // features, a driver re-running the step) land here — close them, or
        // the dashboard's "nodes" count over-reports forever.
        if let Err(e) = self
            .subtask_runs
            .subtask_runs_interrupt_stale(&step_exec.id, crate::paths::now_ms())
        {
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                error = %e,
                "sequence step: could not close stale subtask_runs rows"
            );
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
        let mut satisfied_decls: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut landed_this_attempt: Vec<runner::LandedTask> = Vec::new();
        let tasks_res = self
            .run_tasks_loop(
                step_exec,
                step_conf,
                accumulated_cost,
                accumulated_tokens,
                step_start,
                step_index,
                step_execs,
                &plan,
                &machine_str,
                &wt_id,
                &wt_path,
                &agent_kind,
                &override_model,
                &mut all_artifact_refs,
                &mut satisfied_decls,
                &mut landed_this_attempt,
            )
            .await;

        if let Err((msg, environmental)) = tasks_res {
            // A mid-list failure does not forfeit the tasks that already
            // finished: their work is committed in the worktree and paid
            // for. Merge that prefix to the feature branch and record it, so
            // the retry runs only the remainder (decision 13's
            // continue-and-report intent, adapted to ordered tasks — the
            // tail may depend on the failed task, so it is not run, but the
            // completed prefix is kept). Cancellation is not a failure:
            // the user asked to stop, so the branch rolls back as before.
            if !*self.cancel_watch.borrow() && !landed_this_attempt.is_empty() {
                match self
                    .checkpoint_landed_prefix(&wt_id, &wt_path, &machine_str, &landed_this_attempt)
                    .await
                {
                    Ok(()) => {
                        let entry = self
                            .sequence_checkpoints
                            .entry(step_exec.step_id.0.clone())
                            .or_default();
                        for t in &landed_this_attempt {
                            if !entry.contains(&t.id) {
                                entry.push(t.id.clone());
                            }
                        }
                        let landed_total = entry.len();
                        self.cleanup_sequence_worktree(&wt_id).await;
                        return self
                            .fail_sequence_step(
                                step_exec,
                                step_start,
                                *accumulated_cost,
                                *accumulated_tokens,
                                msg,
                                environmental,
                                FailureDisposition::PrefixLanded {
                                    landed: landed_total,
                                    total: landed_total + plan.tasks.len()
                                        - landed_this_attempt.len(),
                                },
                            )
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(
                            feature_id = %self.f_id,
                            step_id = %step_exec.step_id.0,
                            error = %e,
                            "sequence step: could not merge the completed task prefix; \
                             falling back to a full rollback"
                        );
                    }
                }
            }
            let rolled_back = self
                .cleanup_and_rollback(&wt_id, &machine_str, &base_sha)
                .await;
            return self
                .fail_sequence_step(
                    step_exec,
                    step_start,
                    *accumulated_cost,
                    *accumulated_tokens,
                    msg,
                    environmental,
                    FailureDisposition::from_rollback(rolled_back),
                )
                .await;
        }

        // 3b. Declared deliverables must exist. A `ByName` / `LastWriteTo`
        //     artifact that no task ever produced means the step ran and wrote
        //     nothing downstream can consume — the same misconfiguration class
        //     the agent step fails on (wrong model, blocked writes, agent wrote
        //     to the wrong path). Judged here, across the whole task list,
        //     rather than per task: only one task in the list may be the one
        //     that writes the report, so a per-task check would fail every
        //     other task spuriously.
        //
        //     `AllWrites` / `ChangedFiles` / `Diff` captures can never be
        //     missing (they describe whatever the agent touched), so an
        //     ordinary implement step never trips this.
        let never_produced: Vec<&str> = step_conf
            .artifacts
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter(|d| !satisfied_decls.contains(&d.name))
            .map(|d| d.name.as_str())
            .collect();
        if !never_produced.is_empty() {
            let rolled_back = self
                .cleanup_and_rollback(&wt_id, &machine_str, &base_sha)
                .await;
            let names = never_produced.join(", ");
            return self
                .fail_sequence_step(
                    step_exec,
                    step_start,
                    *accumulated_cost,
                    *accumulated_tokens,
                    format!(
                        "sequence step: every task ran, but the declared artifact(s) {} were \
                         never produced by any task. Nothing downstream can consume this step — \
                         the agent may have written to a different path, or been blocked by its \
                         model/config.",
                        names
                    ),
                    false,
                    FailureDisposition::from_rollback(rolled_back),
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
                let rolled_back = self
                    .cleanup_and_rollback(&wt_id, &machine_str, &base_sha)
                    .await;
                // A failed rollback leaves the rejected attempt's commits on
                // the feature branch. Fold the warning into the verdict so it
                // reaches both the stored error and the retry feedback —
                // `resolve_task_plan` treats a step's own failure as rolled
                // back and would otherwise tell the next attempt's agents the
                // branch is clean. Only the failure case is decorated: a
                // clean rollback is the normal path and needs no note in the
                // verdict the retry prompts render.
                let decorate = |m: &str| -> String {
                    if rolled_back {
                        m.to_string()
                    } else {
                        FailureDisposition::RollbackFailed.decorate(m, &self.branch_name)
                    }
                };
                return match verifier_err {
                    crate::domain::verifier::VerifierError::Verdict(mut failure) => {
                        failure.reason = decorate(&failure.reason);
                        StepOutcome::VerdictFailed(failure)
                    }
                    crate::domain::verifier::VerifierError::Infrastructure(msg) => {
                        StepOutcome::NonRetryable(decorate(&format!(
                            "[verifier infrastructure error — check verifier config] {}",
                            msg
                        )))
                    }
                    // Triaged as an environment problem: the box is not
                    // provisioned, editing source cannot fix it.
                    crate::domain::verifier::VerifierError::Environment(msg) => {
                        StepOutcome::NonRetryable(decorate(&msg))
                    }
                };
            }
        }

        // 5. One merge, at the end. The tasks cannot have conflicted with
        //    each other (same worktree, sequential, each committed), so the
        //    only way this conflicts is the feature branch having moved
        //    beneath us — e.g. a `sync` step pulled upstream in a prior step.
        //
        //    That is worth recovering from rather than failing on: the
        //    worktree holds a complete, verified implementation of every
        //    task, and throwing it away over conflict markers would mean
        //    re-running the whole task list. So we spend one agent turn
        //    resolving the conflict and retry the merge.
        let merge_res = self
            .merge_with_conflict_recovery(
                step_exec,
                &wt_id,
                &wt_path,
                &machine_str,
                &agent_kind,
                &override_model,
                self.resolve_step_effort(step_conf),
                accumulated_cost,
                accumulated_tokens,
                step_start,
            )
            .await;

        if let Err((msg, environmental)) = merge_res {
            let _ = self.notif.emit(&DomainEvent::ConflictDetected {
                feature_id: self.f_id.clone(),
                subtask_id: crate::adapters::worktree::git_ops::subtask_branch_name(
                    &self.branch_name,
                    &wt_id,
                ),
            });
            let rolled_back = self
                .cleanup_and_rollback(&wt_id, &machine_str, &base_sha)
                .await;
            return self
                .fail_sequence_step(
                    step_exec,
                    step_start,
                    *accumulated_cost,
                    *accumulated_tokens,
                    msg,
                    environmental,
                    FailureDisposition::from_rollback(rolled_back),
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
                // The merge has already landed by this point, so this path has
                // to undo it — otherwise a retry starts from a branch carrying
                // an empty merge commit.
                let rolled_back = self
                    .cleanup_and_rollback(&wt_id, &machine_str, &base_sha)
                    .await;
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
                        FailureDisposition::from_rollback(rolled_back),
                    )
                    .await;
            }
            refs = step_exec.artifact_paths.clone();
        }

        self.cleanup_sequence_worktree(&wt_id).await;

        // The step is done, so any mid-list checkpoint is spent: from here on
        // the ordinary "previous attempt landed" retry logic covers the
        // branch's contents, and a stale skip-list would silently exempt
        // tasks from a future full re-run.
        self.sequence_checkpoints.remove(&step_exec.step_id.0);

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

    /// Merge the step's task branch into the feature branch, spending one
    /// agent turn on conflict resolution if the merge conflicts.
    ///
    /// Unlike the agent step, there is no live session to reuse here — each
    /// task's session is killed as soon as its task commits, deliberately, so
    /// no context carries across tasks. So the resolution pass gets its own
    /// fresh session in the step's worktree. It only ever sees the conflicted
    /// files, which is all it needs.
    ///
    /// `Err((message, environmental))` when the merge could not be made to
    /// land; the caller rolls the feature branch back.
    #[allow(clippy::too_many_arguments)]
    async fn merge_with_conflict_recovery(
        &self,
        step_exec: &StepExecution,
        wt_id: &str,
        wt_path: &str,
        machine_str: &str,
        agent_kind: &str,
        override_model: &Option<String>,
        effort: crate::domain::models::EffortLevel,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
    ) -> Result<(), (String, bool)> {
        let merge_err = match self
            .git_ops
            .merge_subtask(
                self.machine_id_opt.as_deref(),
                wt_path,
                &self.branch_name,
                wt_id,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };

        let conflict_thread_id = format!("{}-{}-merge", self.f_id_str, step_exec.step_id.0);
        let session = self
            .spawn_sequence_session(
                &conflict_thread_id,
                "resolve merge conflicts",
                machine_str,
                wt_path,
                agent_kind,
                override_model,
                effort,
            )
            .await?;

        let pass = self
            .resolve_merge_conflicts_via_agent(
                step_exec,
                &*session,
                machine_str,
                wt_path,
                override_model,
                accumulated_cost,
                accumulated_tokens,
                step_start,
            )
            .await;
        let _ = self.registry.kill(&conflict_thread_id).await;

        match pass {
            // Not a content conflict — an agent cannot help. Report the
            // original merge error, which says what actually went wrong.
            Ok(ConflictPass::NothingToResolve) => Err((
                format!(
                    "sequence step: merging the completed task branch into '{}' failed: {}",
                    self.branch_name, merge_err
                ),
                false,
            )),
            Ok(ConflictPass::Resolved(_)) => self
                .git_ops
                .merge_subtask(
                    self.machine_id_opt.as_deref(),
                    wt_path,
                    &self.branch_name,
                    wt_id,
                )
                .await
                .map_err(|e| {
                    (
                        format!(
                            "sequence step: merging into '{}' still failed after the agent \
                             resolved the conflicts: {}",
                            self.branch_name, e
                        ),
                        false,
                    )
                }),
            Err(ConflictPassError::Cancelled) => {
                Err(("Execution cancelled by user".to_string(), false))
            }
            Err(ConflictPassError::Failed(msg)) => Err((
                format!(
                    "sequence step: could not resolve the conflicts merging into '{}': {}",
                    self.branch_name, msg
                ),
                false,
            )),
            Err(ConflictPassError::Environmental(msg)) => Err((
                format!(
                    "sequence step: agent error while resolving the merge conflicts: {}",
                    msg
                ),
                true,
            )),
        }
    }

    /// Preserve the completed task prefix after a mid-list failure: reset the
    /// worktree to the last completed task's commit — discarding the failed
    /// task's debris, both uncommitted writes and any commits its agent made
    /// itself — and merge the step's task branch into the feature branch.
    ///
    /// Only the *merge conflict* recovery is deliberately absent here. On the
    /// success path a conflicting merge is worth an agent turn, because the
    /// worktree holds a complete verified implementation. Here the step is
    /// already failing; spending more agent budget to salvage a partial
    /// prefix inverts that trade, so a conflict falls back to the ordinary
    /// full rollback in the caller.
    async fn checkpoint_landed_prefix(
        &self,
        wt_id: &str,
        wt_path: &str,
        machine_str: &str,
        landed: &[runner::LandedTask],
    ) -> Result<(), String> {
        let last = landed
            .last()
            .ok_or_else(|| "no completed tasks to checkpoint".to_string())?;

        self.exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} reset --hard {}",
                    paths::shell_escape_posix(wt_path),
                    paths::shell_escape_posix(&last.sha),
                ),
            )
            .await
            .map_err(|e| {
                format!(
                    "could not reset the worktree to the last completed task's commit {}: {}",
                    last.sha, e
                )
            })?;

        self.git_ops
            .merge_subtask(
                self.machine_id_opt.as_deref(),
                wt_path,
                &self.branch_name,
                wt_id,
            )
            .await
            .map_err(|e| {
                format!(
                    "merging the completed task prefix into '{}' failed: {}",
                    self.branch_name, e
                )
            })
    }

    /// Tear the step's worktree down and reset the feature branch to
    /// `base_sha`, so a retry starts from the tip the step began at.
    ///
    /// **The order is load-bearing.** `merge_subtask` checks the feature
    /// branch *out inside this worktree* when it is not checked out anywhere
    /// else (which is the normal case — the feature branch is otherwise just a
    /// ref). Git refuses to `branch -f` a branch that a worktree holds, so
    /// rolling back before removing the worktree fails with "cannot force
    /// update the branch ... used by worktree at ...". Remove first, then
    /// reset.
    ///
    /// Returns whether the branch actually moved back. Callers must not claim
    /// a rollback they did not get: if the worktree could not be removed (a
    /// locked file — the case `provision_subtask_worktree` explicitly warns
    /// about), the reset fails and the step's commits are still on the branch.
    async fn cleanup_and_rollback(&self, wt_id: &str, machine_str: &str, base_sha: &str) -> bool {
        self.cleanup_sequence_worktree(wt_id).await;

        let reset = self
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

        match reset {
            Ok(_) => true,
            Err(e) => {
                tracing::error!(
                    feature_id = %self.f_id,
                    branch = %self.branch_name,
                    base_sha = %base_sha,
                    error = %e,
                    "sequence step: could not roll the feature branch back; the failed step's \
                     commits are still on it"
                );
                false
            }
        }
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
    #[allow(clippy::too_many_arguments)]
    async fn fail_sequence_step(
        &self,
        step_exec: &StepExecution,
        step_start: Instant,
        cost: f64,
        tokens: i64,
        msg: String,
        environmental: bool,
        disposition: FailureDisposition,
    ) -> StepOutcome {
        let is_cancelled = *self.cancel_watch.borrow();
        let status_str = if is_cancelled {
            "interrupted"
        } else {
            "failed"
        };
        let wall = step_start.elapsed().as_secs();
        let stored = disposition.decorate(&msg, &self.branch_name);
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

// ── NodeHandler registration (P1.7) ───────────────────────────────────────────

/// The `sequence` node type behind the [`NodeHandler`] seam. Pure
/// delegation: execution is [`ExecutionDriver::handle_sequence_step`],
/// byte-for-byte the behavior the old `match` arm dispatched. Owns the
/// retired `parallel` alias (see the module docs above) so workflows
/// the user cloned before the rename keep running.
///
/// [`NodeHandler`]: crate::adapters::step_executor::registry::NodeHandler
pub(crate) struct SequenceNodeHandler;

/// JSON Schema for the `sequence` node's `config` payload — the
/// residual [`StepConfig`] fields after migration lifts
/// `task_list_from` into a typed `task_list` edge.
#[allow(dead_code)] // Read via `NodeHandler::config_schema` (first runtime caller: P3.1).
static SEQUENCE_CONFIG_SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(|| {
        serde_json::json!({
            "type": "object",
            "description": "Configuration for a `sequence` node: run an \
                ordered task list, one fresh agent session per task, in a \
                single worktree, merging once at the end. The task list \
                arrives on a typed `task_list` edge (v1: `task_list_from`); \
                without one, the node plans its own decomposition.",
            "properties": {
                "agent_kind": {
                    "type": ["string", "null"],
                    "description": "Per-step agent runtime override for the \
                        task agents. Unset inherits the run/project chain."
                },
                "model": {
                    "type": ["string", "null"],
                    "description": "Per-step model override for the task \
                        agents."
                },
                "effort": {
                    "type": ["string", "null"],
                    "enum": ["low", "medium", "high", "xhigh", "max", null],
                    "description": "Per-step reasoning-effort override. \
                        Unset inherits."
                },
                "prompt_template": {
                    "type": ["string", "null"],
                    "description": "Prompt template each task agent renders, \
                        with the task's own goal injected."
                },
                "max_iterations": {
                    "type": ["integer", "null"],
                    "minimum": 1,
                    "description": "v1 legacy retry budget; see the agent \
                        node's field of the same name."
                },
                "artifacts": {
                    "type": ["array", "null"],
                    "description": "Declared artifact captures committed or \
                        stored after the step.",
                    "items": { "type": "object" }
                },
                "verifier": {
                    "type": ["object", "null"],
                    "description": "Optional harness/verifier turn run after \
                        the list lands; a FAIL verdict feeds the retry \
                        policy targeted at the tasks owning the implicated \
                        files."
                },
                "capability": {
                    "type": ["string", "null"],
                    "description": "Write-scope capability class. Sequence \
                        steps default to Implement (they legitimately write \
                        across the source tree)."
                },
                "allow_network": {
                    "type": "boolean",
                    "default": false,
                    "description": "Opt the task agents into web search / \
                        fetch."
                },
                "allow_shell": {
                    "type": "boolean",
                    "default": false,
                    "description": "Opt a non-shell capability into the \
                        shell."
                }
            },
            "additionalProperties": true
        })
    });

#[async_trait::async_trait]
impl crate::adapters::step_executor::registry::NodeHandler for SequenceNodeHandler {
    fn kind(&self) -> &'static str {
        "sequence"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // The superseded name. Its concurrent fan-out was removed; such
        // steps now run their tasks sequentially. Kept so workflows the
        // user cloned or overrode keep running instead of failing with
        // "Unknown step kind".
        &["parallel"]
    }

    fn config_schema(&self) -> &'static serde_json::Value {
        &SEQUENCE_CONFIG_SCHEMA
    }

    async fn execute(
        &self,
        ctx: crate::adapters::step_executor::registry::NodeCtx<'_>,
    ) -> StepOutcome {
        ctx.driver
            .handle_sequence_step(
                ctx.step_exec,
                ctx.step_conf,
                ctx.accumulated_cost,
                ctx.accumulated_tokens,
                ctx.step_start,
                ctx.step_index,
                ctx.step_execs,
            )
            .await
    }
}
