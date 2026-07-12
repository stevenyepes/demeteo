// Tests for `steps/finalize/context.rs` (mirrored-tests convention).

use super::*;

/// Demeteo's own bookkeeping commits describe the machinery, not the work.
/// The whole point of the squash is to make them disappear, so they are also
/// worthless as input to the summary — and actively misleading, since an
/// agent shown "chore: merge subtask sub-2" tends to write about merging.
#[test]
fn plumbing_commits_are_recognised_as_demeteos_own() {
    assert!(is_plumbing_commit("chore: merge subtask sub-2", "f-123"));
    assert!(is_plumbing_commit(
        "chore: resolve merge conflicts with feature/f-123",
        "f-123"
    ));
    assert!(is_plumbing_commit(
        "chore: resolve sync conflicts with origin/main",
        "f-123"
    ));
    assert!(is_plumbing_commit(
        "feat(f-123): implement the thing",
        "f-123"
    ));
}

#[test]
fn real_work_commits_are_kept() {
    assert!(!is_plumbing_commit("feat(api): add retry budget", "f-123"));
    assert!(!is_plumbing_commit("fix: handle the empty case", "f-123"));
    // Another feature's step commit is not ours to filter, and shouldn't
    // appear on this branch anyway.
    assert!(!is_plumbing_commit("feat(f-999): other work", "f-123"));
    // A human's genuine chore commit must survive.
    assert!(!is_plumbing_commit("chore: bump deps", "f-123"));
}
