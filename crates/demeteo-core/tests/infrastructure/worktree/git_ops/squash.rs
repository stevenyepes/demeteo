use super::super::common::*;
use super::*;
use crate::ports::execution::ExecutionPort;

/// Every hook shape that reaches a Windows host, decided without one.
///
/// The two integration tests below cover the case that matters in practice —
/// `#!/bin/sh` — but only on the platform that was never broken. On Windows
/// the wrong answer here is a spawn error reported to the authoring agent as
/// the hook's verdict, and nothing in a Linux run can observe that. This is
/// the whole of the falsifiable part.
#[test]
fn a_hook_is_launched_through_a_shell_exactly_when_one_would_read_its_shebang() {
    use hook::{Launch, Shell};

    let sh = Launch::Shell(Shell::Sh);
    let bash = Launch::Shell(Shell::Bash);

    assert_eq!(
        hook::launch("commit-msg", Some("#!/bin/sh\ngrep x \"$1\"")),
        sh
    );
    assert_eq!(hook::launch("commit-msg", Some("#!/bin/bash\n")), bash);
    assert_eq!(hook::launch("commit-msg", Some("#!/usr/bin/env sh\n")), sh);
    assert_eq!(
        hook::launch("commit-msg", Some("#!/usr/bin/env bash\n")),
        bash
    );
    assert_eq!(
        hook::launch("commit-msg", Some("#!/usr/bin/env -S bash -e\n")),
        bash,
        "an `env -S` line names its shell after the switch"
    );
    assert_eq!(
        hook::launch("commit-msg", Some("#!/bin/sh\r\n")),
        sh,
        "a hook checked out with CRLF endings still names sh"
    );

    assert_eq!(
        hook::launch(
            "commit-msg",
            Some("npx --no-install commitlint --edit $1\n")
        ),
        sh,
        "husky's generated hooks carry no shebang and are sh"
    );
    assert_eq!(
        hook::launch("commit-msg", Some("")),
        sh,
        "an empty hook is a script that does nothing, not a binary"
    );

    assert_eq!(
        hook::launch("commit-msg.exe", Some("MZ\u{0}\u{0}")),
        Launch::Direct,
        "a compiled hook is already startable"
    );
    assert_eq!(
        hook::launch("C:\\repo\\.git/hooks/commit-msg.CMD", Some("@echo off\n")),
        Launch::Direct,
        "the extension test is case-insensitive and survives mixed separators"
    );
    assert_eq!(
        hook::launch("commit-msg", None),
        Launch::Direct,
        "a hook that is not UTF-8 is not a script"
    );
    assert_eq!(
        hook::launch("commit-msg", Some("#!/usr/bin/env node\n")),
        Launch::Direct,
        "locating node would be a second interpreter search"
    );
    assert_eq!(hook::launch("commit-msg", Some("#!\n")), Launch::Direct);
}

/// The hook and the message file both have to reach the script in a form the
/// shell running it — and anything that shell hands them on to — accepts.
#[test]
fn a_shell_launched_hook_is_handed_forward_slash_paths() {
    let direct = ProgramRequest {
        executable: "C:\\Users\\RUNNER~1\\repo\\.git/hooks/commit-msg".to_string(),
        args: vec!["C:\\Users\\RUNNER~1\\repo\\.git/DEMETEO_COMMIT_MSG".to_string()],
        cwd: Some("C:\\Users\\RUNNER~1\\repo".to_string()),
        ..ProgramRequest::default()
    };

    let through =
        hook::through_posix_shell(direct, Path::new("C:\\Program Files\\Git\\bin\\sh.exe"));

    assert_eq!(through.executable, "C:\\Program Files\\Git\\bin\\sh.exe");
    assert_eq!(
        through.args,
        vec![
            "C:/Users/RUNNER~1/repo/.git/hooks/commit-msg".to_string(),
            "C:/Users/RUNNER~1/repo/.git/DEMETEO_COMMIT_MSG".to_string(),
        ],
        "the script and its $1 are what the shell reads as text"
    );
    assert_eq!(
        through.cwd,
        Some("C:\\Users\\RUNNER~1\\repo".to_string()),
        "cwd goes to CreateProcessW, which never reads it as text"
    );
}

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

#[tokio::test]
async fn test_validate_commit_message_uses_a_linked_worktrees_git_dir() {
    let (dir, helper) = make_repo("squash_validate_linked_worktree").await;
    let repo = dir.to_string_lossy().to_string();
    let worktree = dir.join("linked-worktree");
    let worktree_path = worktree.to_string_lossy().to_string();
    let exec = fresh_exec();

    exec.run_command(
        "local",
        &format!(
            "git -C \"{repo}\" branch validation-worktree && git -C \"{repo}\" worktree add \"{worktree_path}\" validation-worktree"
        ),
    )
    .await
    .unwrap();

    let hook = format!("{repo}/.git/hooks/commit-msg");
    exec.write_file(
        "local",
        &hook,
        "#!/bin/sh\ngrep -Eq '^chore: .+' \"$1\" || exit 1\n",
    )
    .await
    .unwrap();
    exec.run_command("local", &format!("chmod +x \"{hook}\""))
        .await
        .unwrap();

    let accepted = helper
        .validate_commit_message(None, &worktree_path, "chore: resolve sync conflicts")
        .await;
    assert!(
        accepted.is_ok(),
        "a linked worktree must validate through its real git admin directory; got: {:?}",
        accepted.err()
    );

    // A rejection here is also what proves the hook *ran*: the accepting case
    // above passes just as well when validation was skipped entirely, which is
    // what a Windows host with no `bash.exe` does.
    assert!(
        helper
            .validate_commit_message(None, &worktree_path, "not conventional")
            .await
            .is_err(),
        "the hook must still reject invalid messages from a linked worktree"
    );

    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{worktree_path}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}
