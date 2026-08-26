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

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::spend::RunningSpend;
use crate::adapters::step_executor::steps::list_unmerged::list_unmerged_files;
use crate::adapters::step_executor::steps::pending_commit::{self, PendingCommit};
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

/// How the resolution turn ended, when it did not end well.
///
/// Held rather than returned, because the tree has not been read yet and the
/// tree is what decides. Kept as two arms rather than one string because the
/// classes route differently once the tree *does* refuse: an environmental stop
/// must not feed a rework loop
/// ([`is_process_level_error`](crate::adapters::agent::cli_runtime)), and a
/// class folded into prose is a class the next reader has to parse back out.
enum TurnStop {
    Reported(String),
    Environmental(String),
}

impl TurnStop {
    /// This stop, as the explanation of a tree that is still conflicted.
    fn refuse(self, tree_refusal: &str) -> ConflictPassError {
        use crate::domain::sync_session::resolution_refusal;
        match self {
            Self::Reported(stop) => {
                ConflictPassError::Failed(resolution_refusal(Some(&stop), tree_refusal))
            }
            Self::Environmental(stop) => {
                ConflictPassError::Environmental(resolution_refusal(Some(&stop), tree_refusal))
            }
        }
    }
}

impl ExecutionDriver {
    /// Merge the feature branch into `wt_path` and have `session` resolve
    /// whatever conflicts that surfaces, committing the resolution.
    ///
    /// `spend`'s totals are updated in place.
    pub(crate) async fn resolve_merge_conflicts_via_agent(
        &self,
        step_exec: &StepExecution,
        session: &dyn AgentSession,
        machine_str: &str,
        wt_path: &str,
        override_model: Option<&str>,
        spend: RunningSpend<'_>,
    ) -> Result<ConflictPass, ConflictPassError> {
        let RunningSpend {
            cost: accumulated_cost,
            tokens: accumulated_tokens,
            start: step_start,
        } = spend;
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
        // The verification sentence names the project's own command for the
        // reason `sync_resolve::prompt::build_resolver_prompt` spells out: "make sure
        // all code builds and passes tests" leaves the agent to find the
        // command first, and every guess is a turn against a budget.
        let verification = match self.base_ctx.get("test_command").trim() {
            "" => String::new(),
            cmd => format!(
                " Then verify with this project's own command, exactly as written: `{}` — \
                 do not go looking for another one if it does not work here.",
                cmd
            ),
        };
        let prompt = format!(
            "We encountered a merge conflict while merging the latest changes from the feature \
             branch '{}' into your workspace.\n\
             Please resolve the conflicts in the following files:\n\
             {}\n\n\
             Ensure you edit these files to remove conflict markers (<<<<<<<, =======, >>>>>>>) \
             and integrate the changes correctly.{} \
             Once done, let me know.",
            self.branch_name, files_list, verification
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

        // Only a stop short-circuits. Git's index is what says whether the
        // conflicts are gone, and an agent's exit status is not a reading of it
        // — the same rule stated in full on
        // `domain::sync_session::resolution_refusal`. A turn that hit its dollar
        // ceiling one edit after finishing the work was answered here with a
        // discarded implementation and a step failure.
        let (billing, turn_stop) = match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                return Err(ConflictPassError::Cancelled)
            }
            crate::adapters::agent::event_stream::TurnResult::Failed { reason, spent } => {
                *accumulated_cost += spent.cost_usd;
                *accumulated_tokens += spent.tokens;
                (
                    ConflictPassBilling {
                        cache_read_input_tokens: spent.cache_read_input_tokens,
                        cache_creation_input_tokens: spent.cache_creation_input_tokens,
                    },
                    Some(TurnStop::Reported(reason)),
                )
            }
            crate::adapters::agent::event_stream::TurnResult::Environmental { reason, spent } => {
                *accumulated_cost += spent.cost_usd;
                *accumulated_tokens += spent.tokens;
                (
                    ConflictPassBilling {
                        cache_read_input_tokens: spent.cache_read_input_tokens,
                        cache_creation_input_tokens: spent.cache_creation_input_tokens,
                    },
                    Some(TurnStop::Environmental(reason)),
                )
            }
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
                (
                    ConflictPassBilling {
                        cache_read_input_tokens: outcome.cache_read_input_tokens,
                        cache_creation_input_tokens: outcome.cache_creation_input_tokens,
                    },
                    None,
                )
            }
        };

        if *self.cancel_watch.borrow() {
            return Err(ConflictPassError::Cancelled);
        }

        let still_unmerged = list_unmerged_files(&*self.exec, machine_str, wt_path).await;
        if !still_unmerged.is_empty() {
            let tree_refusal = format!(
                "agent failed to resolve merge conflicts in: {:?}",
                still_unmerged.iter().map(|f| &f.path).collect::<Vec<_>>()
            );
            return Err(match turn_stop {
                Some(stop) => stop.refuse(&tree_refusal),
                None => ConflictPassError::Failed(tree_refusal),
            });
        }

        // `-am` rather than the sync resolver's `add -A` + `-m`: nothing is
        // staged above, and this worktree outlives the pass — the step goes on
        // working in it — so an untracked file lying in it is not part of the
        // resolution. See `steps::pending_commit` for why the guard is here.
        match pending_commit::probe(&*self.exec, machine_str, wt_path).await {
            PendingCommit::Nothing => {}
            PendingCommit::Unreadable(why) => {
                return Err(ConflictPassError::Failed(format!(
                "could not tell whether the merge-conflict resolution still needs committing: {}",
                why
            )))
            }
            PendingCommit::Pending => {
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
        }

        Ok(ConflictPass::Resolved(billing))
    }
}
