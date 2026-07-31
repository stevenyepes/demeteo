//! Which commits on the branch describe the work.

/// Commit subjects Demeteo itself wrote. They describe the machinery, not
/// the work, and the whole point of the squash is to make them disappear —
/// so they are also worthless as input for summarising the work.
pub(crate) fn is_plumbing_commit(subject: &str, feature_id: &str) -> bool {
    subject.starts_with("chore: merge subtask ")
        || subject.starts_with("chore: resolve ")
        || subject.starts_with(&format!("feat({}):", feature_id))
}

/// Read `git log --format='%s%n%b%x1e'` into the bullet list the agent is
/// shown, with Demeteo's own plumbing commits dropped.
///
/// The record separator is why parse and filter belong together: the adapter
/// picks `%x1e` precisely so a multi-line body cannot be read as the next
/// commit's subject, and a reader that split on newlines would silently undo
/// that choice.
pub(crate) fn real_commit_log(raw_log: &str, feature_id: &str) -> String {
    raw_log
        .split('\u{1e}')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter(|entry| {
            let subject = entry.lines().next().unwrap_or("");
            !is_plumbing_commit(subject, feature_id)
        })
        .map(|entry| format!("- {}", entry.replace('\n', "\n  ")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "../../../tests/domain/finalize/commit_log.rs"]
mod tests;
