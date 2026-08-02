//! The terminal-location rule, reachable without a worktree port under it —
//! which is what it was not while it lived in `application::projects`. `super`
//! is `crate::domain::terminal_worktree`.

use super::*;

/// A logical project root and the physical one Git would report for it, in the
/// shape macOS produces for anything under the system temporary directory.
const LOGICAL_ROOT: &str = "/var/folders/ws/projects/p1";
const PHYSICAL_ROOT: &str = "/private/var/folders/ws/projects/p1";

fn worktree(path: &str, branch: Option<&str>) -> WorktreeInfo {
    WorktreeInfo {
        path: path.to_string(),
        branch: branch.map(str::to_string),
        is_locked: false,
    }
}

// ── The anchor ───────────────────────────────────────────────────────────────

#[test]
fn a_physical_primary_anchors_an_area_the_logical_root_could_not_reach() {
    let area = physical_area(
        LOGICAL_ROOT,
        &format!("{LOGICAL_ROOT}/repos/app"),
        &format!("{PHYSICAL_ROOT}/repos/app"),
    );

    assert_eq!(
        area,
        Some(std::path::PathBuf::from(format!(
            "{PHYSICAL_ROOT}/terminal-worktrees/app"
        ))),
        "the area must be rooted where Git says the checkout is, not where configuration says"
    );
}

#[test]
fn the_climb_is_as_deep_as_the_repository_actually_sits() {
    // Not a layout production builds today. It is here so that the day a repo
    // gains a directory level, a hardcoded two-step climb fails loudly rather
    // than anchoring one directory too high.
    let area = physical_area(
        LOGICAL_ROOT,
        &format!("{LOGICAL_ROOT}/repos/group/app"),
        &format!("{PHYSICAL_ROOT}/repos/group/app"),
    );

    assert_eq!(
        area,
        Some(std::path::PathBuf::from(format!(
            "{PHYSICAL_ROOT}/terminal-worktrees/app"
        )))
    );
}

#[test]
fn a_repository_outside_its_project_root_anchors_nothing() {
    assert_eq!(
        physical_area(LOGICAL_ROOT, "/elsewhere/repos/app", "/elsewhere/repos/app"),
        None
    );
}

#[test]
fn a_repository_that_is_the_project_root_anchors_nothing() {
    assert_eq!(
        physical_area(LOGICAL_ROOT, LOGICAL_ROOT, PHYSICAL_ROOT),
        None
    );
}

#[test]
fn a_descent_reaching_back_through_the_project_root_anchors_nothing() {
    assert_eq!(
        physical_area(
            LOGICAL_ROOT,
            &format!("{LOGICAL_ROOT}/repos/../../escapee/app"),
            &format!("{PHYSICAL_ROOT}/repos/app"),
        ),
        None,
        "a `..` makes the climb count a lie about where the root is"
    );
}

#[test]
fn a_primary_too_shallow_for_the_descent_anchors_nothing() {
    assert_eq!(
        physical_area(LOGICAL_ROOT, &format!("{LOGICAL_ROOT}/repos/app"), "/app"),
        None
    );
}

// ── What the anchor admits ───────────────────────────────────────────────────

#[test]
fn a_sibling_sharing_the_areas_name_as_a_prefix_is_not_inside_it() {
    let area = std::path::PathBuf::from("/p/terminal-worktrees/app");

    assert!(
        !is_terminal_location(&area, "/p/terminal-worktrees/app-scratch/one", None),
        "containment is a question about path components, not about text"
    );
    assert!(is_terminal_location(
        &area,
        "/p/terminal-worktrees/app/one",
        None
    ));
}

#[test]
fn the_area_itself_is_not_offered_as_a_location() {
    let area = std::path::PathBuf::from("/p/terminal-worktrees/app");

    assert!(!is_terminal_location(
        &area,
        "/p/terminal-worktrees/app",
        None
    ));
}

#[test]
fn a_workspace_living_under_a_terminal_worktrees_directory_still_withholds_the_pipelines_checkouts()
{
    // The reported bug, as a unit: every path here contains a
    // `terminal-worktrees` component, so the rule this replaced admitted all of
    // them — including a worktree a running step owns.
    let root = "/home/u/terminal-worktrees/ws/projects/p1";
    let repo = format!("{root}/repos/app");

    // The out-of-area entry carries an ordinary branch on purpose. Give it the
    // pipeline's infix and the branch guard would reject it, and this test
    // would pass with the anchor removed — which is the whole subject here.
    let selected = selectable(
        root,
        &repo,
        &repo,
        vec![
            worktree(&format!("{repo}_wt_s-1"), Some("feature/one")),
            worktree(
                &format!("{root}/terminal-worktrees/app/mine"),
                Some("terminal/mine"),
            ),
        ],
    )
    .expect("the area is derivable");

    assert_eq!(
        selected,
        vec![worktree(
            &format!("{root}/terminal-worktrees/app/mine"),
            Some("terminal/mine")
        )],
        "only the area anchored on what git reported may be offered"
    );
}

#[test]
fn a_subtask_branch_inside_the_area_is_still_not_a_location() {
    // Unreachable in production — `validate_terminal_branch` refuses the infix
    // at creation, and a pipeline never puts a checkout in this area. The guard
    // is what keeps those two facts from being load-bearing on their own.
    let area = std::path::PathBuf::from("/p/terminal-worktrees/app");

    assert!(!is_terminal_location(
        &area,
        "/p/terminal-worktrees/app/one",
        Some("feature/one_subtask_s-1")
    ));
}

#[test]
fn an_underivable_area_is_not_an_empty_listing() {
    assert_eq!(
        selectable(LOGICAL_ROOT, "/elsewhere/app", "/elsewhere/app", vec![]),
        None,
        "an empty list is what a healthy repository with no terminal worktrees returns"
    );
}
