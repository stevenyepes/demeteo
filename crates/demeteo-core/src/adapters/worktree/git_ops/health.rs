use super::GitOpsHelper;
use crate::ports::execution::ProgramRequest;

impl GitOpsHelper {
    /// Check if a repository has uncommitted changes or unpushed commits
    pub async fn check_repo_dirty(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<(bool, bool), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);

        // Check if directory exists
        let exists = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--is-inside-work-tree"]),
            )
            .await
            .is_ok();

        if !exists {
            return Ok((false, false));
        }

        // 1. Check for uncommitted changes
        let status_output = match self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["status", "--porcelain"]),
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
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    ["log", "--branches", "--not", "--remotes", "--oneline"],
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
