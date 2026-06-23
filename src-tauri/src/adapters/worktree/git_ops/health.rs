use super::GitOpsHelper;
use crate::paths;

impl GitOpsHelper {
    /// Check if a repository has uncommitted changes or unpushed commits
    pub async fn check_repo_dirty(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<(bool, bool), String> {
        let machine_str = machine_id.unwrap_or("local");

        // Check if directory exists
        let exists = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --is-inside-work-tree",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await
            .is_ok();

        if !exists {
            return Ok((false, false));
        }

        // 1. Check for uncommitted changes
        let status_output = match self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} status --porcelain",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await
        {
            Ok(out) => out.trim().to_string(),
            Err(e) => return Err(format!("Failed to run git status: {}", e)),
        };
        let has_uncommitted = !status_output.is_empty();

        // 2. Check for unpushed commits
        let unpushed_output = match self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} log --branches --not --remotes --oneline",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await
        {
            Ok(out) => out.trim().to_string(),
            Err(_) => String::new(),
        };
        let has_unpushed = !unpushed_output.is_empty();

        Ok((has_uncommitted, has_unpushed))
    }
}
