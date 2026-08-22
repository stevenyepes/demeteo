use crate::domain::merge_status::parse_unmerged;
use crate::domain::models::ConflictFile;
use crate::paths;
use crate::ports::execution::{ask, Answer};

/// Ask `repo_dir` which files a merge left unresolved, or why it could not say.
///
/// The one wrapper around [`parse_unmerged`]: the `git status` invocation is
/// the adapter's, the XY-code table is the domain's.
///
/// Only an *unreadable* command is an `Err`. A refusal is git's verdict that
/// there is nothing to read at `repo_dir` — the same reading
/// `application::sync_session`'s `--git-dir` probe takes — and the caller's
/// next question (is a merge open?) is refused for the same reason and reaches
/// the same conclusion. An abandoned command is not a verdict either way, and
/// answering one with an empty list is what let an unreachable host report "no
/// merge in progress" about a live conflict and then rewrite its row.
pub(crate) async fn try_list_unmerged_files(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Result<Vec<ConflictFile>, String> {
    match ask(
        exec,
        machine_id,
        &format!(
            "git -C {} status --porcelain --untracked-files=no",
            paths::shell_escape_posix(repo_dir)
        ),
    )
    .await
    {
        Answer::Said(raw) => Ok(parse_unmerged(&raw)),
        Answer::Refused => Ok(Vec::new()),
        Answer::Unreadable(e) => Err(e),
    }
}

/// [`try_list_unmerged_files`] for a caller that is only enriching a message it
/// has already decided to send, and has nothing to do about a tree that will
/// not answer.
pub(crate) async fn list_unmerged_files(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<ConflictFile> {
    try_list_unmerged_files(exec, machine_id, repo_dir)
        .await
        .unwrap_or_default()
}
