// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/sequence/git.rs` (mirrored-tests convention). `super` = that module.
//
// The command strings a `sequence` step sends, pinned verbatim.
//
// Two things are being held still. The **shape** — these strings were
// hand-written at eleven call sites before they lived here, and a helper that
// silently reworded one would change what runs on the user's repo. And the
// **escaping**, which is the reason the helper exists: an operand that needs
// quoting must get it at every position, including the ones a reader of a
// `format!` would have to check by eye.

use super::*;

const SHA: &str = "1111111111111111111111111111111111111111";

/// The fast path: nothing here needs quoting, so nothing may acquire any —
/// this is the byte-for-byte shape the step issued before the helper existed.
#[test]
fn the_command_shapes_are_the_ones_git_sees() {
    assert_eq!(
        rev_parse_cmd("/repo", "feat/x"),
        "git -C /repo rev-parse feat/x"
    );
    assert_eq!(
        commit_exists_cmd("/repo", SHA),
        format!("git -C /repo cat-file -e {SHA}^{{commit}}")
    );
    assert_eq!(
        merge_base_cmd("/repo", SHA, "main"),
        format!("git -C /repo merge-base {SHA} main")
    );
    assert_eq!(
        reset_hard_cmd("/wt", SHA),
        format!("git -C /wt reset --hard {SHA}")
    );
    assert_eq!(
        branch_force_cmd("/repo", "feat/x", SHA),
        format!("git -C /repo branch -f feat/x {SHA}")
    );
    assert_eq!(
        update_ref_cmd("/repo", "refs/demeteo/seq/f-1/s-impl", SHA),
        format!("git -C /repo update-ref refs/demeteo/seq/f-1/s-impl {SHA}")
    );
    assert_eq!(
        delete_ref_cmd("/repo", "refs/demeteo/seq/f-1/s-impl"),
        "git -C /repo update-ref -d refs/demeteo/seq/f-1/s-impl"
    );
    assert_eq!(
        diff_name_only_cmd("/wt", SHA),
        format!("git -C /wt diff --name-only {SHA}")
    );
}

/// The regression the helper is for. A repo path with a space, and a branch
/// name carrying a shell metacharacter, must be quoted *wherever* they
/// appear — the missed escape this replaces was a second operand, not a
/// first, and read exactly like its escaped neighbours.
#[test]
fn every_operand_is_escaped_not_just_the_repo() {
    assert_eq!(
        rev_parse_cmd("/My Projects/repo", "feat/a;rm -rf b"),
        "git -C '/My Projects/repo' rev-parse 'feat/a;rm -rf b'"
    );
    assert_eq!(
        branch_force_cmd("/My Projects/repo", "feat/a b", "HEAD~1"),
        "git -C '/My Projects/repo' branch -f 'feat/a b' 'HEAD~1'"
    );
    assert_eq!(
        merge_base_cmd("/repo", "a b", "c d"),
        "git -C /repo merge-base 'a b' 'c d'"
    );
    assert_eq!(
        update_ref_cmd("/repo", "refs/x y", "a b"),
        "git -C /repo update-ref 'refs/x y' 'a b'"
    );
    assert_eq!(
        commit_exists_cmd("/repo", "a b"),
        "git -C /repo cat-file -e 'a b'^{commit}"
    );
}
