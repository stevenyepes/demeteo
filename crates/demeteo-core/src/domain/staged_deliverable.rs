//! Did the agent's deliverable reach the index, or is it stranded under the
//! report subdir?
//!
//! The historical docs-update bug: the agent emitted the real doc body under
//! `artifacts/s-draft.md` instead of `docs/<area>/<topic>.md`, and the
//! orchestrator happily committed the summary report — or, with
//! `commit_artifacts=false`, silently produced an empty commit. This is the
//! judgement that catches it; the two log lines it selects stay in the adapter,
//! which is also where the `git diff --cached` that produced `staged` lives.

/// What the stage says about the agent's work.
///
/// **Three states, not two.** The empty-stage case is deliberately a warning
/// and not an error: a step whose writes vanished (a permission-scope rejection,
/// or writes that all landed on excluded paths) is observable but not
/// necessarily broken, and there are legitimate steps that stage nothing.
/// Collapsing [`EmptyStage`](StageVerdict::EmptyStage) into
/// [`Ok`](StageVerdict::Ok) drops that warning; collapsing it into
/// [`Stranded`](StageVerdict::Stranded) starts failing steps that pass today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StageVerdict {
    /// Nothing to say. The stage carries at least one path outside the report
    /// subdir, or there was no signal to judge against.
    Ok,
    /// The agent reported writes outside the report subdir but the stage is
    /// empty — the deliverable did not reach the index. Warn only.
    EmptyStage,
    /// The stage contains *only* report paths while the agent reported writes
    /// outside them. Fails the step, so the retry loop can feed `reason` back
    /// into `{{retry_feedback}}` and direct the next attempt at the real repo
    /// path.
    Stranded { reason: String },
}

/// Judge one staged set against what the agent said it wrote.
///
/// `artifact_subdir` is expected already normalised through
/// [`normalize_artifact_subdir`]; an **empty** one disables the guard entirely,
/// because [`is_under_prefix`] answers `false` for every path and the stage
/// therefore always looks like it holds a non-artifact write. That is current
/// behaviour, and a "defensive" flip to `true` would start failing steps.
pub(crate) fn judge_stage(
    staged: &[&str],
    non_artifact_writes: &[String],
    artifact_subdir: &str,
) -> StageVerdict {
    if staged.is_empty() && !non_artifact_writes.is_empty() {
        return StageVerdict::EmptyStage;
    }
    if non_artifact_writes.is_empty() || staged.is_empty() {
        return StageVerdict::Ok;
    }
    if staged.iter().any(|p| !is_under_prefix(p, artifact_subdir)) {
        return StageVerdict::Ok;
    }
    StageVerdict::Stranded {
        reason: format!(
            "agent stranded the deliverable under `{}` instead of writing it to \
             the real repo path. Stage contains only artifact paths \
             ({:?}) while the agent reported writes outside the report subdir \
             ({:?}). Re-read the survey's 'Files to Create' / 'Files to Update' \
             sections and write the doc body to the real repo path (e.g. \
             `docs/<area>/<topic>.md`), NOT to {}/s-*.md.",
            artifact_subdir, staged, non_artifact_writes, artifact_subdir,
        ),
    }
}

/// True when `path` sits at or under the directory `prefix` (a directory path
/// with no trailing slash, e.g. `"artifacts"`). Matches both the directory
/// itself (`"artifacts"`) and any file inside it (`"artifacts/s-draft.md"`).
///
/// An empty `prefix` is `false`, never `true`: nothing is "under nothing", and
/// the callers read a `false` as "this path is the user's deliverable".
pub(crate) fn is_under_prefix(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return false;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// The repo-relative form of a configured report subdir: no surrounding
/// whitespace, no leading `./`, no trailing slash.
///
/// Both a `git` pathspec and [`is_under_prefix`] need this shape, and the three
/// call sites that used to spell it inline could each drift from the others —
/// which would put a path in the pathspec that the guard then judged by a
/// different name.
pub(crate) fn normalize_artifact_subdir(subdir: &str) -> &str {
    subdir.trim().trim_start_matches("./").trim_end_matches('/')
}

#[cfg(test)]
#[path = "../../tests/domain/staged_deliverable.rs"]
mod tests;
