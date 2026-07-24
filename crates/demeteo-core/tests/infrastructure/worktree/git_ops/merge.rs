use super::super::common::*;
use crate::ports::execution::ExecutionPort;

/// Regression: a prior attempt interrupted mid-merge leaves `MERGE_HEAD`
/// set on the feature-branch checkout. On retry `merge_subtask` must clear
/// that stale in-progress merge instead of aborting with
/// "fatal: You have not concluded your merge (MERGE_HEAD exists)".
#[tokio::test]
async fn test_merge_subtask_recovers_from_stale_merge_head() {
    let (dir, helper) = make_repo("merge_stale_head").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // Ref-only feature branch, as the pipeline creates it.
    let feature_branch = "feature/f-m";
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" branch {feature_branch}"),
        )
        .await;

    // A subtask worktree with one clean commit adding a new file — merging
    // it into the feature branch does NOT conflict; the only thing that can
    // block it is a leftover in-progress merge.
    let wt_path = helper
        .provision_subtask_worktree(None, &repo, feature_branch, "sub-1")
        .await
        .unwrap();
    exec.write_file("local", &format!("{wt_path}/newfile.txt"), "from subtask")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{wt_path}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{wt_path}\" commit -m \"subtask work\""),
        )
        .await;

    // A worktree on the feature branch, left with a half-finished merge
    // (`--no-commit` leaves MERGE_HEAD set even on a clean merge) — exactly
    // the state an interrupted retry leaves behind.
    let feat_wt = format!("{repo}_feat_wt");
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree add \"{feat_wt}\" {feature_branch}"),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!(
                "git -C \"{feat_wt}\" merge --no-commit --no-ff {feature_branch}_subtask_sub-1"
            ),
        )
        .await;
    // Sanity: the stale MERGE_HEAD is really present.
    assert!(
        exec.run_command(
            "local",
            &format!("git -C \"{feat_wt}\" rev-parse -q --verify MERGE_HEAD"),
        )
        .await
        .is_ok(),
        "test setup should have left a stale MERGE_HEAD"
    );

    // The retry: merge_subtask targets the feature-branch checkout, which
    // still carries the stale MERGE_HEAD. It must recover and succeed.
    let result = helper
        .merge_subtask(None, &feat_wt, feature_branch, "sub-1")
        .await;
    assert!(
        result.is_ok(),
        "merge_subtask should clear the stale MERGE_HEAD and merge; got {result:?}"
    );

    // The stale merge is gone and the subtask's file is now on the branch.
    assert!(
        exec.run_command(
            "local",
            &format!("git -C \"{feat_wt}\" rev-parse -q --verify MERGE_HEAD"),
        )
        .await
        .is_err(),
        "no MERGE_HEAD should remain after merge_subtask"
    );
    assert!(
        exec.run_command(
            "local",
            &format!("git -C \"{feat_wt}\" cat-file -e HEAD:newfile.txt"),
        )
        .await
        .is_ok(),
        "subtask's file should be present on the feature branch after merge"
    );

    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{feat_wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
    let _ = std::fs::remove_dir_all(&feat_wt);
}

/// Regression: a target repo with a commitlint-style `commit-msg` hook used
/// to reject the merge commit ("subject may not be empty"), leaving the merge
/// staged but uncommitted. The rejection is deterministic, so every retry hit
/// the same hook and the pipeline could never make progress. Demeteo's own
/// merge commits must run with the repo's hooks disabled.
#[tokio::test]
async fn test_merge_subtask_survives_rejecting_commit_msg_hook() {
    let (dir, helper) = make_repo("merge_commit_hook").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // A commit-msg hook that rejects every message, no matter how it is
    // phrased — the general case of a target repo whose hooks we cannot
    // satisfy (commitlint with a required `scope-enum`, a `pre-commit` that
    // runs the whole test suite). A well-phrased commit message is not a fix
    // for this; the hooks must not run for Demeteo's own commits at all.
    let hook = format!("{repo}/.git/hooks/commit-msg");
    exec.write_file(
        "local",
        &hook,
        "#!/bin/sh\necho '✖ subject may not be empty [subject-empty]' >&2\nexit 1\n",
    )
    .await
    .unwrap();
    exec.run_command("local", &format!("chmod +x \"{hook}\""))
        .await
        .unwrap();

    let feature_branch = "feature/f-h";
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" branch {feature_branch}"),
        )
        .await;

    // A subtask worktree with one clean commit — nothing here conflicts, so
    // the hook is the only thing that can block the merge.
    let wt_path = helper
        .provision_subtask_worktree(None, &repo, feature_branch, "sub-2")
        .await
        .unwrap();
    exec.write_file("local", &format!("{wt_path}/hooked.txt"), "from subtask")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{wt_path}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{wt_path}\" commit --no-verify -m \"subtask work\""),
        )
        .await;

    // Advance the feature branch past the point sub-2 branched from (as an
    // earlier sibling subtask's merge would). The branches now diverge, so
    // the merge cannot fast-forward and git must write a real merge commit —
    // which is what runs the commit-msg hook.
    let sib_wt = format!("{repo}_sibling_wt");
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree add \"{sib_wt}\" {feature_branch}"),
        )
        .await;
    exec.write_file("local", &format!("{sib_wt}/sibling.txt"), "from sibling")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{sib_wt}\" add ."))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{sib_wt}\" commit --no-verify -m \"feat: sibling work\""),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{sib_wt}\""),
        )
        .await;

    let result = helper
        .merge_subtask(None, &wt_path, feature_branch, "sub-2")
        .await;
    assert!(
        result.is_ok(),
        "merge_subtask must bypass the repo's commit-msg hook; got {result:?}"
    );

    // The merge was actually committed, not left staged with MERGE_HEAD set.
    assert!(
        exec.run_command(
            "local",
            &format!("git -C \"{wt_path}\" rev-parse -q --verify MERGE_HEAD"),
        )
        .await
        .is_err(),
        "merge should be committed, not left half-finished"
    );
    assert!(
        exec.run_command(
            "local",
            &format!("git -C \"{wt_path}\" cat-file -e HEAD:hooked.txt"),
        )
        .await
        .is_ok(),
        "subtask's file should be present on the feature branch after merge"
    );

    // The merge commit's own message is conventional, so a CI-side lint over
    // the whole PR range passes too.
    let subject = exec
        .run_command(
            "local",
            &format!("git -C \"{wt_path}\" log -1 --format=%s {feature_branch}"),
        )
        .await
        .unwrap();
    assert_eq!(subject.trim(), "chore: merge subtask sub-2");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
    let _ = std::fs::remove_dir_all(&sib_wt);
}
