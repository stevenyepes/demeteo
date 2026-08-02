//! Which of a repository's linked worktrees a terminal session may open.
//!
//! This was `is_terminal_worktree` in
//! `crates/demeteo-core/src/application/projects.rs`, spelled inside the
//! `async fn` that also made the port call — so the rule could only be reached
//! by a test willing to stand up the worktree port first, and in practice was
//! only ever exercised through one. See [`crate::domain`].
//!
//! What it decided there was a guess. Terminal worktrees sit at
//! `<project_root>/terminal-worktrees/<repo_name>/`, but a project root built
//! from configuration is a *logical* path while Git replays the *physical* one
//! it resolved, and those differ wherever a symlink sits on the way in — on
//! macOS `/var` and `/private/var` name one directory. With no path it could
//! compare against, the old rule scanned for a `terminal-worktrees` component
//! anywhere, which a workspace that merely lived under a directory of that name
//! satisfied for every worktree in the repository, pipeline-owned ones
//! included. The fix is not a tighter scan; it is an anchor Git itself
//! reported.

use crate::domain::ids::SUBTASK_BRANCH_INFIX;
use crate::domain::models::WorktreeInfo;

/// The physical terminal area for a repository whose paths were computed
/// logically, or `None` when the observation admits no area at all.
///
/// `project_root` and `repo_dir` come from configuration; `primary_worktree` is
/// the main checkout as `git worktree list` reported it, which is the same
/// directory as `repo_dir` with every symlink already resolved. Anchoring on it
/// costs nothing — Git names it in the listing this filter is reading anyway.
///
/// The number of components to climb is *derived* from how far `repo_dir` sits
/// below `project_root`, never assumed to be two. Where a checkout lives under
/// a project root is a layout decision that has already moved once; a hardcoded
/// climb would keep compiling and anchor somewhere quietly wrong.
///
/// `None` rather than an error: nothing here parses, so nothing can fail in a
/// way worth naming. It answers "does this observation admit an area" — no when
/// `repo_dir` is not strictly below `project_root`, when the descent is not a
/// plain one (a `..` would make the climb count a lie), or when the primary is
/// too shallow to climb. Callers must not read `None` as "no worktrees": an
/// area that cannot be proven and an area that is genuinely empty are the same
/// answer to the user and opposite answers to an operator.
pub fn physical_area(
    project_root: &str,
    repo_dir: &str,
    primary_worktree: &str,
) -> Option<std::path::PathBuf> {
    let descent = std::path::Path::new(repo_dir)
        .strip_prefix(std::path::Path::new(project_root))
        .ok()?;
    if descent.components().count() == 0
        || !descent
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }

    let mut root = std::path::Path::new(primary_worktree);
    for _ in descent.components() {
        root = root.parent()?;
    }

    Some(
        root.join(crate::paths::TERMINAL_WORKTREES_SUBDIR)
            .join(std::path::Path::new(repo_dir).file_name()?),
    )
}

/// Whether one listed worktree is a location a terminal session may open.
///
/// Containment is asked of [`std::path::Path`], not of the string: as text,
/// `…/terminal-worktrees/app-scratch` starts with `…/terminal-worktrees/app`,
/// and a sibling repository would be handed out as this one's.
///
/// The branch clause is redundant and deliberate. A pipeline's checkouts are
/// `<repo_dir>_wt_<id>` — siblings of the repository under `repos/` — so none
/// can satisfy the area test, and the guard costs nothing while the day someone
/// moves those under a shared root is exactly the day this filter would
/// otherwise start handing out a worktree an agent is mid-run in.
/// `validate_terminal_branch` refuses the infix at creation so that no
/// worktree a user *can* make is silently excluded here.
pub fn is_terminal_location(
    area: &std::path::Path,
    worktree_path: &str,
    branch: Option<&str>,
) -> bool {
    let path = std::path::Path::new(worktree_path);
    path.starts_with(area)
        && path != area
        && !branch.is_some_and(|branch| branch.contains(SUBTASK_BRANCH_INFIX))
}

/// The terminal-openable subset of one repository's linked worktrees.
pub fn selectable(
    project_root: &str,
    repo_dir: &str,
    primary_worktree: &str,
    linked: Vec<WorktreeInfo>,
) -> Option<Vec<WorktreeInfo>> {
    let area = physical_area(project_root, repo_dir, primary_worktree)?;
    Some(
        linked
            .into_iter()
            .filter(|worktree| {
                is_terminal_location(&area, &worktree.path, worktree.branch.as_deref())
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "../../tests/domain/terminal_worktree.rs"]
mod tests;
