use super::GitOpsHelper;
use crate::ports::execution::ProgramRequest;

impl GitOpsHelper {
    /// Merge a subtask branch back into the parent feature branch.
    ///
    /// Operates in the **worktree** (`wt_path`) instead of the main repo
    /// so concurrent pipelines cannot race on a shared checkout.
    pub async fn merge_subtask(
        &self,
        machine_id: Option<&str>,
        wt_path: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let subtask_branch = super::subtask_branch_name(feature_branch, subtask_id);
        // Find if the feature branch is checked out in any worktree (including the main repo).
        let checked_out_path = match self
            .exec
            .run_program(machine_str, super::worktree::worktree_list_request(wt_path))
            .await
        {
            Ok(worktree_list) => crate::domain::worktree_listing::parse(&worktree_list)
                .all()
                .find(|worktree| worktree.branch.as_deref() == Some(feature_branch))
                .map(|worktree| worktree.path.clone()),
            Err(_) => None,
        };

        // Conventional-commit form: a target repo that lints the whole PR
        // range in CI rejects a bare "Merge subtask sub-2" even though the
        // local hook is bypassed.
        let message = format!("chore: merge subtask {subtask_id}");

        if let Some(ref active_wt) = checked_out_path {
            // The feature branch is already checked out in a worktree (e.g. main repo).
            // Merge the subtask branch directly into that worktree.
            self.abort_inflight_merge(machine_str, active_wt).await;
            self.exec
                .run_program(
                    machine_str,
                    git_request(
                        active_wt,
                        [
                            "-c",
                            "core.hooksPath=/dev/null",
                            "merge",
                            &subtask_branch,
                            "-m",
                            &message,
                        ],
                    ),
                )
                .await?;
        } else {
            // The feature branch is not checked out in any worktree.
            // Checkout the feature branch in the subtask worktree, then merge.
            self.abort_inflight_merge(machine_str, wt_path).await;
            self.exec
                .run_program(
                    machine_str,
                    git_request(wt_path, ["checkout", feature_branch]),
                )
                .await?;
            self.exec
                .run_program(
                    machine_str,
                    git_request(
                        wt_path,
                        [
                            "-c",
                            "core.hooksPath=/dev/null",
                            "merge",
                            &subtask_branch,
                            "-m",
                            &message,
                        ],
                    ),
                )
                .await?;
        }
        Ok(())
    }

    /// Clear any half-finished merge left in `safe_dir` (an already
    /// shell-escaped worktree path) by a prior attempt that was interrupted
    /// or failed mid-merge. Without this, the next `git merge` aborts with
    /// "fatal: You have not concluded your merge (MERGE_HEAD exists)" and the
    /// retry can never make progress.
    ///
    /// Best-effort: `git merge --abort` fails harmlessly when there is no
    /// in-progress merge, so its error is ignored. `git reset --hard HEAD`
    /// then clears any lingering conflicted index / working-tree state
    /// (e.g. a half-resolved merge with no MERGE_HEAD). Both are safe here
    /// because the subtask work lives committed on the subtask branch — the
    /// feature-branch checkout carries no changes worth preserving.
    async fn abort_inflight_merge(&self, machine_str: &str, dir: &str) {
        let _ = self
            .exec
            .run_program(machine_str, git_request(dir, ["merge", "--abort"]))
            .await;
        let _ = self
            .exec
            .run_program(machine_str, git_request(dir, ["reset", "--hard", "HEAD"]))
            .await;
    }
}

fn git_request<const N: usize>(repo_dir: &str, args: [&str; N]) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: [
            vec!["-C".to_string(), repo_dir.to_string()],
            args.into_iter().map(str::to_string).collect(),
        ]
        .concat(),
        ..ProgramRequest::default()
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/merge.rs"]
mod tests;
