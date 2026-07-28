//! Agent-driven recovery from a failed merge-back.
//!
//! Both the `agent` step and the `sequence` step do their work on a subtask
//! branch in an isolated worktree and merge it into the feature branch when
//! they are done. That merge can conflict — not because the step's own work
//! is inconsistent, but because the feature branch moved beneath it (a `sync`
//! step pulling upstream is the usual cause).
//!
//! Failing the step there throws away a complete implementation over a textual
//! conflict. Instead we hand the conflict markers to an agent and let the
//! caller retry the merge once the resolution is committed.
//!
//! # Which worktree, and on which branch
//!
//! This is subtler than it looks, because `merge_subtask`
//! behaves differently depending on where the feature branch is checked out,
//! and it has *already run and failed* by the time we get here. Two cases:
//!
//! * **The feature branch is checked out nowhere** — the normal case: the main
//!   repo stays on the default branch and the feature branch is only a ref. So
//!   `merge_subtask` checked the feature branch out **inside this worktree**
//!   and merged the subtask branch into it. When it failed, it left the
//!   worktree *on the feature branch*, mid-merge, with a conflicted index. The
//!   `git merge` we issue below is therefore a **no-op** — git refuses it
//!   ("Merging is not possible because you have unmerged files"), which is why
//!   its error is ignored. The conflicts we resolve are the ones the failed
//!   merge already left, and committing them **concludes that merge**: the
//!   feature branch advances here, and the caller's retry merge is a
//!   no-op "Already up to date".
//!
//! * **The feature branch is checked out somewhere else** — `merge_subtask`
//!   merged into *that* worktree and left this one on the subtask branch. Here
//!   the `git merge` below really does merge the feature branch into the
//!   worktree, the resolution lands on the subtask branch, and the caller's
//!   retry is the merge that matters.
//!
//! Both converge on the same contract — *when this returns `Resolved`, the
//! conflicts are gone and committed; retry the merge* — which is why the code
//! can stay branch-agnostic. But do not "simplify" the ignored merge error or
//! the retry away: each is load-bearing in exactly one of the two cases.

use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::list_unmerged::list_unmerged_files;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::StepExecution;
use crate::paths;
use crate::ports::agent_runtime::AgentSession;
use crate::ports::notification::DomainEvent;

/// What the resolution pass billed. The caller folds these into the step's
/// running totals — a conflict turn costs real tokens.
pub(crate) struct ConflictPassBilling {
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

pub(crate) enum ConflictPass {
    /// The merge-back surfaced no conflicted files, so the original merge
    /// failure was not a content conflict and there is nothing an agent can
    /// fix. The caller keeps its original error.
    NothingToResolve,
    /// Conflicts were resolved and committed in the worktree. The caller
    /// should retry the merge-back.
    Resolved(ConflictPassBilling),
}

pub(crate) enum ConflictPassError {
    Cancelled,
    Failed(String),
    Environmental(String),
}

impl ExecutionDriver {
    /// Merge the feature branch into `wt_path` and have `session` resolve
    /// whatever conflicts that surfaces, committing the resolution.
    ///
    /// `accumulated_cost` / `accumulated_tokens` are updated in place.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn resolve_merge_conflicts_via_agent(
        &self,
        step_exec: &StepExecution,
        session: &dyn AgentSession,
        machine_str: &str,
        wt_path: &str,
        override_model: Option<&str>,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
    ) -> Result<ConflictPass, ConflictPassError> {
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} merge {}",
                    paths::shell_escape_posix(wt_path),
                    paths::shell_escape_posix(&self.branch_name)
                ),
            )
            .await;

        let unmerged = list_unmerged_files(&*self.exec, machine_str, wt_path).await;
        if unmerged.is_empty() {
            return Ok(ConflictPass::NothingToResolve);
        }

        let files_list = unmerged
            .iter()
            .map(|f| format!("- {} ({})", f.path, f.kind))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "We encountered a merge conflict while merging the latest changes from the feature \
             branch '{}' into your workspace.\n\
             Please resolve the conflicts in the following files:\n\
             {}\n\n\
             Ensure you edit these files to remove conflict markers (<<<<<<<, =======, >>>>>>>) \
             and integrate the changes correctly. Make sure all code builds and passes tests. \
             Once done, let me know.",
            self.branch_name, files_list
        );

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());
        let base_cost = *accumulated_cost;
        let base_tokens = *accumulated_tokens;

        let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
            session,
            &prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            machine_str,
            &*self.exec,
            override_model.map(str::to_string),
            self.pricing.clone(),
            |event| {
                if let AgentEvent::Text { delta } = event {
                    let _ = self.notif.emit(&DomainEvent::AgentStream {
                        feature_id: self.f_id.clone(),
                        step_execution_id: step_exec.id.clone(),
                        content: delta.clone(),
                    });
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "running".into(),
                        cost_usd: Some(base_cost),
                        tokens: Some(base_tokens),
                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    });
                }
            },
        )
        .await;

        let billing = match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                return Err(ConflictPassError::Cancelled)
            }
            crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                return Err(ConflictPassError::Failed(descriptive))
            }
            crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => {
                return Err(ConflictPassError::Environmental(descriptive))
            }
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
                ConflictPassBilling {
                    cache_read_input_tokens: outcome.cache_read_input_tokens,
                    cache_creation_input_tokens: outcome.cache_creation_input_tokens,
                }
            }
        };

        if *self.cancel_watch.borrow() {
            return Err(ConflictPassError::Cancelled);
        }

        let still_unmerged = list_unmerged_files(&*self.exec, machine_str, wt_path).await;
        if !still_unmerged.is_empty() {
            return Err(ConflictPassError::Failed(format!(
                "agent failed to resolve merge conflicts in: {:?}",
                still_unmerged.iter().map(|f| &f.path).collect::<Vec<_>>()
            )));
        }

        // Commit the resolution — but only if the agent has not already done
        // it for us, which is common: told to fix conflict markers, an agent
        // very often stages and commits on its own. That consumes `MERGE_HEAD`
        // and leaves a clean tree, so an unconditional `git commit -am` exits
        // non-zero with "nothing to commit" and we would fail the step —
        // rolling back a merge that in fact succeeded.
        //
        // A clean tree with the conflicts gone *is* the success condition, so
        // treat "nothing to commit" as done rather than as an error.
        if self.worktree_has_pending_commit(machine_str, wt_path).await {
            self.exec
                .run_command(
                    machine_str,
                    &format!(
                        "{} commit -am {}",
                        paths::git_no_hooks(wt_path),
                        paths::shell_escape_posix(&format!(
                            "chore: resolve merge conflicts with {}",
                            self.branch_name
                        )),
                    ),
                )
                .await
                .map_err(|e| {
                    ConflictPassError::Failed(format!(
                        "failed to commit the merge-conflict resolution: {}",
                        e
                    ))
                })?;
        }

        Ok(ConflictPass::Resolved(billing))
    }

    /// Is there anything for `git commit` to record in `wt_path` — either an
    /// in-progress merge to conclude, or modified tracked files?
    ///
    /// `git status --porcelain` is empty exactly when the tree is clean, and
    /// `MERGE_HEAD` exists exactly while a merge is awaiting its commit. An
    /// agent that resolved *and committed* leaves neither.
    async fn worktree_has_pending_commit(&self, machine_str: &str, wt_path: &str) -> bool {
        let safe = paths::shell_escape_posix(wt_path);
        let merge_in_progress = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} rev-parse --verify --quiet MERGE_HEAD", safe),
            )
            .await
            .map(|out| !out.trim().is_empty())
            .unwrap_or(false);
        if merge_in_progress {
            return true;
        }
        self.exec
            .run_command(machine_str, &format!("git -C {} status --porcelain", safe))
            .await
            .map(|out| !out.trim().is_empty())
            .unwrap_or(false)
    }
}
