use super::GitOpsHelper;
use crate::paths;
use crate::ports::worktree_ops::MergePreCheck;

impl GitOpsHelper {
    /// Check if a subtask is already merged or would conflict, without
    /// touching any working tree. Uses `git fetch` + `git merge-base` +
    /// `git merge-tree` — all pure ref/object operations.
    ///
    /// `repo_dir` is the main clone (used only for its `.git` refs/objects).
    pub async fn precheck_merge(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_branch: &str,
    ) -> Result<MergePreCheck, String> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(repo_dir);

        // Fetch latest feature branch from origin into shared refs.
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} fetch origin {}",
                    safe_dir,
                    paths::shell_escape_posix(feature_branch),
                ),
            )
            .await;

        // Already merged?  subtask is an ancestor of origin/feature_branch.
        if self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} merge-base --is-ancestor {} refs/remotes/origin/{}",
                    safe_dir,
                    paths::shell_escape_posix(subtask_branch),
                    paths::shell_escape_posix(feature_branch),
                ),
            )
            .await
            .is_ok()
        {
            return Ok(MergePreCheck::AlreadyMerged);
        }

        // Would conflict?  In-memory merge (no working tree touched).
        match self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} merge-tree --write-tree refs/remotes/origin/{} {}",
                    safe_dir,
                    paths::shell_escape_posix(feature_branch),
                    paths::shell_escape_posix(subtask_branch),
                ),
            )
            .await
        {
            Ok(_) => Ok(MergePreCheck::CleanMerge),
            Err(_) => Ok(MergePreCheck::WouldConflict),
        }
    }

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
        let machine_str = machine_id.unwrap_or("local");
        let subtask_branch = format!("{}_subtask_{}", feature_branch, subtask_id);
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

        if let Some(ref active_wt) = checked_out_path {
            // The feature branch is already checked out in a worktree (e.g. main repo).
            // Merge the subtask branch directly into that worktree.
            let safe_active_wt = paths::shell_escape_posix(active_wt);
            let cmd = format!(
                "git -C {} merge {} -m \"Merge subtask {}\"",
                safe_active_wt,
                safe_sb,
                paths::shell_escape_posix(subtask_id),
            );
            self.exec.run_command(machine_str, &cmd).await?;
        } else {
            // The feature branch is not checked out in any worktree.
            // Checkout the feature branch in the subtask worktree, then merge.
            self.exec
                .run_command(
                    machine_str,
                    &format!("git -C {} checkout {}", safe_wt, safe_fb),
                )
                .await?;

            let cmd = format!(
                "git -C {} merge {} -m \"Merge subtask {}\"",
                safe_wt,
                safe_sb,
                paths::shell_escape_posix(subtask_id),
            );
            self.exec.run_command(machine_str, &cmd).await?;
        }
        Ok(())
    }
}
