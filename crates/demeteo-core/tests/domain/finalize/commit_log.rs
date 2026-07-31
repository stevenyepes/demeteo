// Which commits on the branch describe the work.
// `super` = `domain::finalize::commit_log`.

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

// ── real_commit_log ──────────────────────────────────────────────────────

#[test]
fn a_multi_line_body_stays_with_its_subject() {
    // The record separator is the whole reason the adapter asks for `%x1e`:
    // split on newlines and the second line of a body becomes a commit.
    let raw =
        "feat(api): add retries\nUpstream flakes.\nSo we retry.\u{1e}fix: handle empty\n\u{1e}";
    assert_eq!(
        real_commit_log(raw, "f-123"),
        "- feat(api): add retries\n  Upstream flakes.\n  So we retry.\n- fix: handle empty"
    );
}

#[test]
fn an_empty_trailing_record_is_dropped() {
    // `git log` emits a separator after the last commit, so the split always
    // yields one empty tail.
    assert_eq!(real_commit_log("feat: one\n\u{1e}", "f-123"), "- feat: one");
    assert_eq!(real_commit_log("", "f-123"), "");
    assert_eq!(real_commit_log("\u{1e}\u{1e}  \u{1e}", "f-123"), "");
}

#[test]
fn plumbing_commits_never_reach_the_agent() {
    let raw = "chore: merge subtask sub-2\n\u{1e}feat(f-123): step work\n\u{1e}feat(api): real work\n\u{1e}";
    assert_eq!(real_commit_log(raw, "f-123"), "- feat(api): real work");
}
