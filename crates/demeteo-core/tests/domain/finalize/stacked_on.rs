// What finalize discloses about commits outside its summary range.
// `super` = `domain::finalize::stacked_on`.

use super::*;

/// Fork PR `#40` touches 40 files; its fix is 2 lines. The fix PR opens
/// against `main` carrying all 40, so the agent must be told to say so.
#[test]
fn a_ref_run_is_told_its_pr_is_stacked_on_the_reviewed_request() {
    let origin = FeatureOrigin::Ref {
        fetch_spec: "refs/pull/40/head".to_string(),
        label: "main".to_string(),
    };
    let note = stacked_on_note(&origin, "main").expect("a Ref run carries the request's commits");
    assert!(note.contains("#40"), "names the reviewed request: {note}");
    assert!(!note.contains("reviewed request `main`"), "{note}");
    assert!(note.contains("into `main`"), "names the target: {note}");
    assert!(
        note.contains("also merges that reviewed request's commits"),
        "{note}"
    );
}

#[test]
fn a_gitlab_ref_names_the_merge_request_without_using_its_branch_label() {
    let origin = FeatureOrigin::Ref {
        fetch_spec: "refs/merge-requests/77/head".to_string(),
        label: "main".to_string(),
    };
    let note = stacked_on_note(&origin, "main").expect("a Ref run carries the request's commits");
    assert!(note.contains("!77"), "{note}");
}

#[test]
fn a_default_branch_run_has_nothing_stacked_under_it() {
    assert_eq!(stacked_on_note(&FeatureOrigin::DefaultBranch, "main"), None);
}

/// A same-repo fix publishes against the head it was cut from, so the PR
/// carries only the fix.
#[test]
fn a_branch_run_has_nothing_stacked_under_it() {
    let origin = FeatureOrigin::Branch {
        base: "patch-1".to_string(),
    };
    assert_eq!(stacked_on_note(&origin, "patch-1"), None);
}
