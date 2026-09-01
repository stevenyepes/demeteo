//! Reading the tree, before the turn and after it.
//!
//! Both reads are the same kind of question — one scripted [`ExecutionPort`]
//! answers either — and neither decides anything: they report what the
//! worktree holds and leave the verdict to [`super`].

use crate::adapters::step_executor::steps::list_unmerged::try_list_unmerged_files;
use crate::paths;
use crate::ports::execution::{ask, Answer, ExecutionPort};

/// Why a turn stopped before it started — and, decisively, whether the machine
/// answered.
///
/// The two arms differ only in what the caller is then allowed to write. A
/// preflight that answered from an *unreachable* host used to rewrite a
/// `conflicted` row to `resolution_failed` and replace `raw_error` with "no
/// merge in progress": a diagnosis of a tree nobody looked at, telling the user
/// to re-run Sync, whose force-remove then takes the still-live conflicted
/// worktree with it.
pub(super) enum PreflightRefusal {
    /// git answered, and there is no merge here for an agent to resolve.
    NothingToResolve(String),
    /// The worktree could not be read. Nothing may be concluded from that in
    /// either direction, so nothing may be recorded from it either.
    Unreadable(String),
}

/// Is there a merge in `worktree` for an agent to resolve, and which files did
/// it leave unmerged?
pub(super) async fn preflight(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    worktree: &str,
) -> Result<Vec<crate::domain::models::ConflictFile>, PreflightRefusal> {
    let unmerged = try_list_unmerged_files(exec, machine_str, worktree)
        .await
        .map_err(|why| {
            PreflightRefusal::Unreadable(format!(
                "Could not read the sync worktree at {} to see what the merge left, \
                 so the sync was left as it was: {}",
                worktree, why
            ))
        })?;
    if !unmerged.is_empty() {
        return Ok(unmerged);
    }
    match ask(
        exec,
        machine_str,
        &format!(
            "git -C {} rev-parse --verify --quiet MERGE_HEAD",
            paths::shell_escape_posix(worktree)
        ),
    )
    .await
    {
        Answer::Said(out) if !out.trim().is_empty() => Ok(unmerged),
        Answer::Said(_) | Answer::Refused => Err(PreflightRefusal::NothingToResolve(
            "No active merge in progress. Please run 'Sync with main' first.".to_string(),
        )),
        Answer::Unreadable(e) => Err(PreflightRefusal::Unreadable(format!(
            "Could not check {} for an open merge, so the sync was left as it was: {}",
            worktree, e
        ))),
    }
}

/// Every declared conflict that still carries markers, and how many hunks each
/// has left.
///
/// The whole of what separates a converging resolution from a stalled one, and
/// the reason it counts hunks rather than files: a round that clears three of
/// four hunks inside one file moves no file count at all. Judged on files
/// remaining, three consecutive rounds that each cleared real work on one
/// feature were every one of them recorded as the same failure.
///
/// `declared` is the *persisted* conflict list, never the index's live answer —
/// see [`super::run_resolver_turn`] for why the two must not be the same list.
pub(crate) async fn unresolved_files(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    worktree: &str,
    declared: &[crate::domain::models::ConflictFile],
) -> Result<Vec<(String, usize)>, String> {
    let mut left = Vec::new();
    for file in declared {
        let path = paths::join_on(
            worktree,
            [file.path.as_str()],
            paths::targets_windows_host(machine_str),
        );
        let content = exec
            .read_file(machine_str, &path)
            .await
            .map_err(|e| format!("Failed to read resolved conflict file {}: {}", file.path, e))?;
        if has_conflict_marker(&content) {
            left.push((file.path.clone(), conflict_hunks(&content)));
        }
    }
    Ok(left)
}

/// How many conflicts a file has left, counted by opening marker.
pub(super) fn conflict_hunks(content: &str) -> usize {
    content
        .lines()
        .filter(|line| line.trim_start().starts_with("<<<<<<<"))
        .count()
}

/// Does this file still hold a merge git did not finish?
///
/// A bare `=======` is deliberately **not** one of the tells, though it is half
/// of every conflict. Seven equals signs on their own line are a Markdown setext
/// underline and a common test fixture, so treating one as a marker makes a file
/// that is genuinely resolved unresolvable — no edit can clear a marker that was
/// never a marker. The other three openers have no such second life, and a
/// half-deleted conflict still trips `>>>>>>>` or `|||||||`, so nothing that
/// clause caught goes unnoticed.
pub(super) fn has_conflict_marker(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("<<<<<<<")
            || trimmed.starts_with(">>>>>>>")
            || trimmed.starts_with("|||||||")
    })
}
