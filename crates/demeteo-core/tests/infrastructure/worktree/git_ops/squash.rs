use super::super::common::*;
use super::*;
use crate::ports::execution::ExecutionPort;

/// The core contract: N commits become 1, the content is byte-for-byte
/// unchanged, and the pre-squash tip stays reachable by name.
#[tokio::test]
async fn test_squash_feature_branch_collapses_history_preserving_tree() {
    let (dir, helper, repo) = make_repo_with_feature_commits("squash_collapse", 4).await;
    let exec = fresh_exec();

    let tree_before = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" rev-parse feature/f-sq^{{tree}}"),
        )
        .await
        .unwrap();
    let tip_before = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" rev-parse feature/f-sq"),
        )
        .await
        .unwrap();

    let outcome = helper
        .squash_feature_branch(
            None,
            &repo,
            "feature/f-sq",
            "main",
            "feat(x): do the whole thing\n\nLong body explaining it.",
        )
        .await
        .expect("squash should succeed");

    let (sha, collapsed, backup_ref) = match outcome {
        SquashOutcome::Squashed {
            sha,
            collapsed,
            backup_ref,
        } => (sha, collapsed, backup_ref),
        other => panic!("expected Squashed, got {other:?}"),
    };
    assert_eq!(collapsed, 4, "four commits should have been collapsed");

    // Exactly one commit now sits on top of main.
    let count = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" rev-list --count main..feature/f-sq"),
        )
        .await
        .unwrap();
    assert_eq!(count.trim(), "1", "branch should carry a single commit");

    // Squashing rewrites history, never content.
    let tree_after = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" rev-parse feature/f-sq^{{tree}}"),
        )
        .await
        .unwrap();
    assert_eq!(
        tree_after.trim(),
        tree_before.trim(),
        "the squashed commit must carry the identical tree"
    );

    // The message survived intact, subject and body.
    let subject = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" log -1 --format=%s feature/f-sq"),
        )
        .await
        .unwrap();
    let body = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" log -1 --format=%b feature/f-sq"),
        )
        .await
        .unwrap();
    assert_eq!(subject.trim(), "feat(x): do the whole thing");
    assert_eq!(body.trim(), "Long body explaining it.");
    assert_eq!(
        exec.run_command(
            "local",
            &format!("git -C \"{repo}\" rev-parse feature/f-sq")
        )
        .await
        .unwrap()
        .trim(),
        sha,
        "the branch should point at the returned sha"
    );

    // The branch is checked out here. Because the tree did not change, the
    // working tree must still be clean — no reset, no stray modifications.
    // This is the property that `commit-tree` buys over `reset --soft`.
    let status = exec
        .run_command("local", &format!("git -C \"{repo}\" status --porcelain"))
        .await
        .unwrap();
    assert!(
        status.trim().is_empty(),
        "working tree must stay clean after a squash; got: {status:?}"
    );

    // The old history is still reachable by name, so the rewrite is undoable.
    let backed_up = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" rev-parse {backup_ref}"),
        )
        .await
        .unwrap();
    assert_eq!(backed_up.trim(), tip_before.trim());

    let _ = std::fs::remove_dir_all(&dir);
}

/// The undo path: restore the branch (and the checkout holding it) to the
/// full pre-squash history.
#[tokio::test]
async fn test_restore_pre_squash_brings_back_the_original_history() {
    let (dir, helper, repo) = make_repo_with_feature_commits("squash_restore", 3).await;
    let exec = fresh_exec();

    let tip_before = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" rev-parse feature/f-sq"),
        )
        .await
        .unwrap();

    helper
        .squash_feature_branch(None, &repo, "feature/f-sq", "main", "feat: squashed")
        .await
        .unwrap();
    helper
        .restore_pre_squash(None, &repo, "feature/f-sq")
        .await
        .expect("restore should succeed");

    assert_eq!(
        exec.run_command(
            "local",
            &format!("git -C \"{repo}\" rev-parse feature/f-sq")
        )
        .await
        .unwrap()
        .trim(),
        tip_before.trim(),
        "branch should be back at its pre-squash tip"
    );
    assert_eq!(
        exec.run_command(
            "local",
            &format!("git -C \"{repo}\" rev-list --count main..feature/f-sq"),
        )
        .await
        .unwrap()
        .trim(),
        "3",
        "all three original commits should be back"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A branch that adds no commits has no PR worth opening — and must not be
/// rewritten into an empty one.
#[tokio::test]
async fn test_squash_feature_branch_reports_nothing_to_squash() {
    let (dir, helper, repo) = make_repo_with_feature_commits("squash_empty", 0).await;

    let outcome = helper
        .squash_feature_branch(None, &repo, "feature/f-sq", "main", "feat: nothing")
        .await
        .unwrap();
    assert_eq!(outcome, SquashOutcome::NothingToSquash);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Commits whose net effect on the tree is nil (a change and its revert)
/// are also nothing to publish, even though the branch has commits.
#[tokio::test]
async fn test_squash_feature_branch_treats_net_zero_change_as_nothing() {
    let (dir, helper, repo) = make_repo_with_feature_commits("squash_netzero", 1).await;
    let exec = fresh_exec();

    // Revert the one commit: the branch has 2 commits but the same tree as main.
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" rm -q file0.txt"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" commit --no-verify -m \"revert it\""),
        )
        .await;

    let outcome = helper
        .squash_feature_branch(None, &repo, "feature/f-sq", "main", "feat: net zero")
        .await
        .unwrap();
    assert_eq!(outcome, SquashOutcome::NothingToSquash);

    let _ = std::fs::remove_dir_all(&dir);
}

/// The repo's own commit-msg hook judges the squashed message *before* it
/// is used, so commitlint becomes feedback for the authoring agent instead
/// of a failed commit. Its output is handed back verbatim.
#[tokio::test]
async fn test_validate_commit_message_runs_the_repos_commit_msg_hook() {
    let (dir, helper) = make_repo("squash_validate").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // No hook installed → no opinion.
    assert!(
        helper
            .validate_commit_message(None, &repo, "anything at all")
            .await
            .is_ok(),
        "a repo with no commit-msg hook must not reject anything"
    );

    // A commitlint-style hook: conventional commits only.
    let hook = format!("{repo}/.git/hooks/commit-msg");
    exec.write_file(
        "local",
        &hook,
        "#!/bin/sh\ngrep -Eq '^(feat|fix|chore)(\\(.+\\))?: .+' \"$1\" || {\n  echo '✖ subject may not be empty [subject-empty]' >&2\n  exit 1\n}\n",
    )
    .await
    .unwrap();
    exec.run_command("local", &format!("chmod +x \"{hook}\""))
        .await
        .unwrap();

    let rejected = helper
        .validate_commit_message(None, &repo, "Merge subtask sub-2")
        .await
        .expect_err("the hook should reject a non-conventional message");
    assert!(
        rejected.hook_output.contains("subject-empty"),
        "the hook's own output must reach the agent verbatim; got: {:?}",
        rejected.hook_output
    );

    assert!(
        helper
            .validate_commit_message(None, &repo, "feat(api): add the thing")
            .await
            .is_ok(),
        "a conventional message should pass the same hook"
    );

    // Validation must be side-effect free — no commit, no stray temp file.
    let status = exec
        .run_command("local", &format!("git -C \"{repo}\" status --porcelain"))
        .await
        .unwrap();
    assert!(
        status.trim().is_empty(),
        "validating a message must not dirty the working tree; got: {status:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
