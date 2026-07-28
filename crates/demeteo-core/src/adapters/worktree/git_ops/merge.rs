use super::GitOpsHelper;
use crate::paths;

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
        let safe_wt = paths::shell_escape_posix(wt_path);
        let safe_fb = paths::shell_escape_posix(feature_branch);
        let safe_sb = paths::shell_escape_posix(&subtask_branch);

        // Find if the feature branch is checked out in any worktree (including the main repo).
        let mut checked_out_path = None;
        if let Ok(worktree_list) = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} worktree list --porcelain", safe_wt),
            )
            .await
        {
            let mut current_path = None;
            for line in worktree_list.lines() {
                if line.starts_with("worktree ") {
                    current_path = Some(line.trim_start_matches("worktree ").trim().to_string());
                } else if line.starts_with("branch ") {
                    let branch_name = line
                        .trim_start_matches("branch refs/heads/")
                        .trim_start_matches("branch ")
                        .trim();
                    if branch_name == feature_branch {
                        checked_out_path = current_path.clone();
                        break;
                    }
                }
            }
        }

        // Conventional-commit form: a target repo that lints the whole PR
        // range in CI rejects a bare "Merge subtask sub-2" even though the
        // local hook is bypassed.
        let message = paths::shell_escape_posix(&format!("chore: merge subtask {}", subtask_id));

        if let Some(ref active_wt) = checked_out_path {
            // The feature branch is already checked out in a worktree (e.g. main repo).
            // Merge the subtask branch directly into that worktree.
            self.abort_inflight_merge(machine_str, &paths::shell_escape_posix(active_wt))
                .await;
            let cmd = format!(
                "{} merge {} -m {}",
                paths::git_no_hooks(active_wt),
                safe_sb,
                message,
            );
            self.exec.run_command(machine_str, &cmd).await?;
        } else {
            // The feature branch is not checked out in any worktree.
            // Checkout the feature branch in the subtask worktree, then merge.
            self.abort_inflight_merge(machine_str, &safe_wt).await;
            self.exec
                .run_command(
                    machine_str,
                    &format!("git -C {} checkout {}", safe_wt, safe_fb),
                )
                .await?;

            let cmd = format!(
                "{} merge {} -m {}",
                paths::git_no_hooks(wt_path),
                safe_sb,
                message,
            );
            self.exec.run_command(machine_str, &cmd).await?;
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
    async fn abort_inflight_merge(&self, machine_str: &str, safe_dir: &str) {
        let _ = self
            .exec
            .run_command(machine_str, &format!("git -C {} merge --abort", safe_dir))
            .await;
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} reset --hard HEAD", safe_dir),
            )
            .await;
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/merge.rs"]
mod tests;
