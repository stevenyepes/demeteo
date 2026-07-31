use crate::domain::merge_status::parse_unmerged;
use crate::domain::models::ConflictFile;
use crate::paths;

/// Ask `repo_dir` which files a merge left unresolved.
///
/// The one wrapper around [`parse_unmerged`]: the `git status` invocation is
/// the adapter's, the XY-code table is the domain's.
pub(crate) async fn list_unmerged_files(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<ConflictFile> {
    let raw = match exec
        .run_command(
            machine_id,
            &format!(
                "git -C {} status --porcelain --untracked-files=no",
                paths::shell_escape_posix(repo_dir)
            ),
        )
        .await
    {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    parse_unmerged(&raw)
}
