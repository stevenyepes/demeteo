use super::super::common::*;
use crate::domain::sync_failure::SyncBlockedStage;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::{MergeGate, SyncFailure};

/// The exact bug the user hit: a feature branch is "2 commits
/// behind" main with overlapping changes. The sync must
/// surface the conflict list, not silently return "no new
/// commits upstream".
#[tokio::test]
async fn test_sync_feature_with_upstream_detects_conflicts() {
    let (local_dir, remote_dir, helper) = make_two_repos("sync_conflict").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // 1. Create a feature branch with a change to README.md.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{local}/README.md"), "feature change")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" commit -am feature"))
        .await;

    // 2. Advance upstream main (the "remote" working tree)
    //    with an *overlapping* change to the same line. The
    //    user's bug was that this never surfaced as a conflict
    //    when the local feature branch synced.
    exec.write_file("local", &format!("{remote}/README.md"), "main change")
        .await
        .unwrap();
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" commit -am main-advance"),
        )
        .await;

    // 3. Sync the feature branch with origin/main. We expect a
    //    conflict (because the README.md was edited on both
    //    sides), not a silent "no new commits upstream".
    let outcome = helper
        .sync_feature_with_upstream(
            None,
            &local,
            "feature/f-1",
            "main",
            MergeGate::default(),
            &(),
        )
        .await;

    match outcome {
        Ok(_) => panic!(
            "Expected a conflict, but sync returned Ok. The user's bug: \
             the merge should have failed because README.md was edited on \
             both sides."
        ),
        Err(SyncFailure::Blocked {
            stage, raw_error, ..
        }) => panic!(
            "A merge that left unmerged paths must not be classified as \
             blocked ({stage:?}) — the banner would then offer no way to \
             resolve it. raw_error: {raw_error}"
        ),
        Err(SyncFailure::Conflict {
            files, raw_error, ..
        }) => {
            assert!(
                !files.is_empty(),
                "Sync reported failure but no conflict files were captured. \
                 raw_error: {raw_error}"
            );
            assert!(
                files.iter().any(|f| f.path == "README.md"),
                "README.md should be in the conflict list, got: {files:?}"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// When the feature branch already includes all of upstream
/// main, the sync is a true no-op and must say so
/// (`changed: false`) — not invent a merge commit.
#[tokio::test]
async fn test_sync_feature_with_upstream_noop_when_already_in_sync() {
    let (local_dir, remote_dir, helper) = make_two_repos("sync_noop").await;
    let local = local_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // Feature branch on top of the same commit as main.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;

    let outcome = helper
        .sync_feature_with_upstream(
            None,
            &local,
            "feature/f-1",
            "main",
            MergeGate::default(),
            &(),
        )
        .await
        .expect("Sync should succeed when there is nothing to merge");

    assert!(
        !outcome.changed,
        "Sync must report `changed: false` when the feature branch already \
          matches origin/main; got: changed={}",
        outcome.changed
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// A branch whose divergence could not be counted is not a branch that is
/// already in sync. The short-circuit above the merge reads a *measured* zero
/// and nothing else, so a `rev-list` that never answered has to fall through to
/// the merge and fail there, where the reason is nameable.
#[tokio::test]
async fn test_sync_feature_with_upstream_does_not_call_an_unmeasurable_branch_synced() {
    let (local_dir, remote_dir, helper) = make_two_repos("sync_unmeasurable").await;
    let local = local_dir.to_string_lossy().to_string();

    let outcome = helper
        .sync_feature_with_upstream(
            None,
            &local,
            "feature/never-cut",
            "main",
            MergeGate::default(),
            &(),
        )
        .await;

    match outcome {
        Ok(o) => panic!(
            "A branch that does not exist cannot be reported as up to date with \
             upstream; got changed={} merge_commit_sha={:?}",
            o.changed, o.merge_commit_sha
        ),
        Err(SyncFailure::Conflict { .. }) => {
            panic!("Nothing was merged, so nothing can be conflicted")
        }
        Err(SyncFailure::Blocked { stage, .. }) => assert_eq!(
            stage,
            SyncBlockedStage::WorktreeProvision,
            "the branch is what could not be checked out"
        ),
    }

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// When origin is unreachable the sync must surface a real
/// error so the user knows the merge wasn't actually attempted.
/// (The old code silently swallowed fetch failures.)
#[tokio::test]
async fn test_sync_feature_with_upstream_reports_fetch_failure() {
    let (local_dir, remote_dir, helper) = make_two_repos("sync_fetch_fail").await;
    let local = local_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // Create a feature branch and break the remote so the fetch
    // will fail (pointing at a nonexistent path).
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" remote set-url origin /nonexistent/path"),
        )
        .await;

    let outcome = helper
        .sync_feature_with_upstream(
            None,
            &local,
            "feature/f-1",
            "main",
            MergeGate::default(),
            &(),
        )
        .await;
    match outcome {
        Ok(o) => panic!(
            "Sync must NOT return Ok when the fetch fails. Got: {:?}. \
              The user's bug was that fetch errors were silently swallowed \
              and the caller saw a misleading 'no new commits upstream'.",
            o
        ),
        Err(SyncFailure::Conflict { raw_error, .. }) => panic!(
            "A fetch that never ran is not a conflict; classifying it as one \
             is what put a 'Resolve with agent' button under a DNS failure. \
             raw_error: {raw_error}"
        ),
        Err(SyncFailure::Blocked {
            stage, raw_error, ..
        }) => {
            assert_eq!(stage, SyncBlockedStage::Fetch);
            assert!(
                raw_error.to_lowercase().contains("fetch")
                    || raw_error.to_lowercase().contains("origin")
                    || raw_error.to_lowercase().contains("remote"),
                "Error message should mention the fetch/remote failure, got: {raw_error}"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// The user hit this bug: after `sync_feature_with_upstream`
/// produced a conflict, the resolver (which used a fresh
/// worktree) found a clean working tree, the agent had nothing
/// to fix, and the commit failed with "nothing to commit".
/// This test pins the property: the conflict lives in the
/// main repo's index and working tree, and that is exactly
/// where the agent must run. A fresh worktree is NOT a
/// substitute.
#[tokio::test]
async fn test_resolver_must_run_in_main_repo_not_worktree() {
    let (local_dir, remote_dir, helper) = make_two_repos("wt_not_inherit").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // 1. Create a feature branch with an overlapping change.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-resolver"),
        )
        .await;
    exec.write_file("local", &format!("{local}/README.md"), "feature change")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" commit -am feature"))
        .await;

    // 2. Advance upstream with an overlapping change.
    exec.write_file("local", &format!("{remote}/README.md"), "main change")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" commit -am advance"))
        .await;

    // 3. Sync in the main repo — leaves it conflicted.
    let _ = helper
        .sync_feature_with_upstream(
            None,
            &local,
            "feature/f-resolver",
            "main",
            MergeGate::default(),
            &(),
        )
        .await;

    // 4. Critical assertion: the main repo's working tree DOES
    //    contain the conflict. This is what the resolver must
    //    operate on.
    let main_status = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" status --porcelain --untracked-files=no"),
        )
        .await
        .unwrap();
    assert!(
        main_status.contains("README.md"),
        "Main repo should have README.md in unmerged state; got: {}",
        main_status
    );

    // 5. Critical assertion: a fresh worktree off the same
    //    branch does NOT carry the conflict state. The naive
    //    "provision a worktree and spawn the agent there"
    //    pattern would have the agent see a clean tree and
    //    commit nothing. This is the bug the user hit.
    let wt_path = helper
        .provision_subtask_worktree(None, &local, "feature/f-resolver", "sub-resolver")
        .await
        .unwrap();
    let wt = wt_path.clone();
    let wt_status = exec
        .run_command(
            "local",
            &format!("git -C \"{wt}\" status --porcelain --untracked-files=no"),
        )
        .await
        .unwrap();
    assert!(
        wt_status.trim().is_empty(),
        "A fresh worktree MUST start clean (the conflict state lives in \
          the main repo's index, not in any worktree's index). If this \
          assertion fails the resolver is in the wrong place. Got: {}",
        wt_status
    );

    // Cleanup
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// The validate-step "extra changes not in main" bug: when the user
/// starts a feature while checked out on a feature branch (e.g. they
/// manually switched to inspect the last run), `git fetch origin
/// +master:master` succeeds because HEAD isn't on master — but a
/// previous version of `ensure_default_branch_updated` only ran the
/// fetch and stopped, leaving the local `master` ref stale by every
/// commit merged upstream since the user's last manual pull. The new
/// behaviour keeps the local `master` ref in sync via `update-ref`.
#[tokio::test]
async fn test_ensure_default_branch_updates_local_ref_when_head_on_feature() {
    let (local_dir, remote_dir, helper, _remote) =
        two_repos_with_origin_ahead("ff_on_feature", "advance").await;
    let local = local_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // Move HEAD off `main` onto a feature branch — the state the user
    // was in when they reported the bug.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/watching"),
        )
        .await;

    let stale_local_main = rev_parse(&exec, &local, "main").await;

    helper
        .ensure_default_branch_updated(None, &local, "main")
        .await
        .expect("HEAD is on a feature branch, so update-ref should succeed");

    let new_local_main = rev_parse(&exec, &local, "main").await;
    let origin_main = rev_parse(&exec, &local, "origin/main").await;

    assert_eq!(
        new_local_main, origin_main,
        "local main must be fast-forwarded to origin/main when HEAD is on another branch"
    );
    assert_ne!(
        new_local_main, stale_local_main,
        "local main ref must advance from the stale commit"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// The "user is on master, working tree clean" case (the typical
/// bootstrap state). `git fetch origin +master:master` is rejected
/// because master is checked out; the fallback does
/// `git merge --ff-only origin/master` which moves HEAD, the index,
/// and the working tree together.
#[tokio::test]
async fn test_ensure_default_branch_fast_forwards_when_clean_tree() {
    let (local_dir, remote_dir, helper, _remote) =
        two_repos_with_origin_ahead("ff_clean_tree", "advance").await;
    let local = local_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // HEAD stays on `main` (default) — the most common bootstrap state.

    let stale_local_main = rev_parse(&exec, &local, "main").await;

    helper
        .ensure_default_branch_updated(None, &local, "main")
        .await
        .expect("clean working tree + fast-forward possible should succeed");

    let new_local_main = rev_parse(&exec, &local, "main").await;
    let origin_main = rev_parse(&exec, &local, "origin/main").await;

    assert_eq!(
        new_local_main, origin_main,
        "local main must be fast-forwarded via merge --ff-only"
    );
    assert_ne!(
        new_local_main, stale_local_main,
        "local main ref must advance from the stale commit"
    );

    // The working tree must also be in sync — `merge --ff-only` updates
    // the index and the files, so the file we wrote on origin must be
    // present locally too.
    let readme = exec
        .read_file("local", &format!("{local}/README.md"))
        .await
        .unwrap();
    assert_eq!(
        readme.trim(),
        "origin advanced",
        "fast-forward merge must update the working tree, not just the ref"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// The "user is on master with uncommitted edits" case. The fallback
/// returns Err with a clear, actionable message. The feature branch is
/// still cut correctly from origin/main in the next phase (verified by
/// the pre-existing origin-cut test, which still passes after this
/// change), so the pipeline proceeds; the bootstrap detail tells the
/// user what to do.
#[tokio::test]
async fn test_ensure_default_branch_warns_when_dirty_tree() {
    let (local_dir, remote_dir, helper, _remote) =
        two_repos_with_origin_ahead("ff_dirty_tree", "advance").await;
    let local = local_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // HEAD stays on `main`.

    // Make the working tree dirty by editing a TRACKED file. Untracked
    // files don't count for `--untracked-files=no` (which is the right
    // semantic — `.env` / IDE scratch files shouldn't block a sync) but
    // a staged or modified tracked file is the real foot-gun `merge
    // --ff-only` would clobber, and that's the case this test pins.
    exec.write_file(
        "local",
        &format!("{local}/README.md"),
        "user edit in progress",
    )
    .await
    .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add README.md"))
        .await;

    let stale_local_main = rev_parse(&exec, &local, "main").await;

    let err = helper
        .ensure_default_branch_updated(None, &local, "main")
        .await
        .expect_err("dirty working tree should surface a soft error");

    assert!(
        err.contains("uncommitted changes"),
        "error must mention uncommitted changes; got: {err}"
    );
    assert!(
        err.contains("git pull"),
        "error must point the user at `git pull`; got: {err}"
    );
    assert!(
        err.contains("1 commit"),
        "error must include the behind-count so the user sees the gap; got: {err}"
    );

    // The local main ref must NOT have been mutated — the user's dirty
    // checkout and the local ref both stay as they were.
    let new_local_main = rev_parse(&exec, &local, "main").await;
    assert_eq!(
        new_local_main, stale_local_main,
        "local main ref must stay stale when working tree is dirty"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// The incident: origin's copy of the feature branch carried a hand-written fix
/// this clone had never fetched, and the sync merged the base into the stale
/// local ref instead. The merge commit was clean, the pull request showed one
/// ordinary merge, and the fix was gone — so the next agent hit the failure it
/// had fixed and fixed it again.
///
/// The assertion on `head_before` is the second half of the same property: it
/// is the base a review diff and the Discard reset are computed from, so it has
/// to name the tip the merge was actually written on top of. Left at the
/// pre-fast-forward ref it names a commit that is nobody's base — the diff would
/// carry origin's own commits as if this sync had made them, and Discard would
/// rewind the branch past them.
#[tokio::test]
async fn test_sync_fast_forwards_the_feature_branch_from_origin_first() {
    let (local_dir, remote_dir, helper) = make_two_repos("sync_ff_feature").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // 1. Cut the feature branch and publish it, as a run does.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{local}/feature.txt"), "agent work")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add ."))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" commit -m feature"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" push origin feature/f-1"),
        )
        .await;

    // 2. Somebody commits a fix on origin's copy of that branch. The clone's
    //    remote-tracking ref is dropped with it: in the incident that ref was
    //    as stale as the local branch and the fix was not even an object in
    //    this repository, which is what makes the *fetch* the thing under test
    //    rather than the ref that happened to be lying around.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" checkout feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{remote}/fix.txt"), "hand fix")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" add ."))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" commit -m hand-fix"))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" checkout main"))
        .await;
    let origin_tip = rev_parse(&exec, &remote, "feature/f-1").await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" update-ref -d refs/remotes/origin/feature/f-1"),
        )
        .await;

    // 3. And the base moves, so there is a merge for the sync to make at all.
    exec.write_file("local", &format!("{remote}/README.md"), "main change")
        .await
        .unwrap();
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" commit -am main-advance"),
        )
        .await;

    let outcome = helper
        .sync_feature_with_upstream(
            None,
            &local,
            "feature/f-1",
            "main",
            MergeGate::default(),
            &(),
        )
        .await
        .expect("nothing conflicts: the fix and the base touch different files");

    assert!(outcome.changed, "the base had a commit to merge");
    assert_eq!(
        outcome.head_before.as_deref(),
        Some(origin_tip.as_str()),
        "head_before must name the fast-forwarded tip the merge was written on"
    );
    assert!(
        exec.read_file("local", &format!("{local}/fix.txt"))
            .await
            .is_ok(),
        "the fix that was only on origin must survive the sync"
    );
    assert!(
        exec.run_command(
            "local",
            &format!("git -C \"{local}\" merge-base --is-ancestor {origin_tip} feature/f-1"),
        )
        .await
        .is_ok(),
        "origin's tip must be an ancestor of the branch the sync left behind"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// The other half: two histories, and no fast-forward between them. Demeteo
/// merging them would be Demeteo deciding what the branch is, and either side
/// it dropped would go missing exactly as silently as the incident above.
#[tokio::test]
async fn test_sync_refuses_a_feature_branch_that_diverged_from_origin() {
    let (local_dir, remote_dir, helper) = make_two_repos("sync_diverged_feature").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{local}/feature.txt"), "agent work")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add ."))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" commit -m feature"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" push origin feature/f-1"),
        )
        .await;

    // Origin gets a commit the clone does not have…
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" checkout feature/f-1"),
        )
        .await;
    exec.write_file("local", &format!("{remote}/fix.txt"), "hand fix")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" add ."))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" commit -m hand-fix"))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" checkout main"))
        .await;

    // …and the clone gets one origin does not.
    exec.write_file("local", &format!("{local}/more.txt"), "more agent work")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" add ."))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{local}\" commit -m more"))
        .await;
    let tip_before = rev_parse(&exec, &local, "feature/f-1").await;

    exec.write_file("local", &format!("{remote}/README.md"), "main change")
        .await
        .unwrap();
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" commit -am main-advance"),
        )
        .await;

    match helper
        .sync_feature_with_upstream(
            None,
            &local,
            "feature/f-1",
            "main",
            MergeGate::default(),
            &(),
        )
        .await
    {
        Ok(o) => panic!(
            "a branch that diverged from its own upstream cannot be synced; got \
             changed={} merge_commit_sha={:?}",
            o.changed, o.merge_commit_sha
        ),
        Err(SyncFailure::Conflict { raw_error, .. }) => panic!(
            "nothing was merged, so nothing can be conflicted, and the resolver \
             would open a tree with no MERGE_HEAD in it: {raw_error}"
        ),
        Err(SyncFailure::Blocked {
            stage,
            raw_error,
            worktree_path,
            ..
        }) => {
            assert_eq!(stage, SyncBlockedStage::FeatureDiverged);
            assert!(raw_error.contains("feature/f-1"), "{raw_error}");
            assert_eq!(
                worktree_path, None,
                "no merge was attempted, so there is no tree to offer"
            );
        }
    }

    assert_eq!(
        rev_parse(&exec, &local, "feature/f-1").await,
        tip_before,
        "the refusal must leave the branch exactly where it found it"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// Which git call a [`SyncBlockedStage`] is read off, asserted one call at a
/// time. The stage keys the sentence the banner shows and nothing downstream
/// can re-derive it, so a stage attached to the wrong call is a banner that
/// sends the user to check credentials for a push that was rejected — and a
/// suite that stays green, because the domain tests build the stages by hand
/// and only ever prove that `view_for` and `step_next` dispatch on them.
///
/// A real repo cannot reach most of these: `git fetch` of a branch that is not
/// on origin fails at the fetch, not at the rev-parse after it.
mod stage_at_each_call {
    use super::*;
    use crate::adapters::database::SqliteAdapter;
    use crate::adapters::step_executor::scripted_exec::ScriptedExec;
    use crate::adapters::worktree::git_ops::GitOpsHelper;
    use crate::ports::db::AppSettingsRepository;
    use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};
    use rusqlite::Connection;
    use std::sync::Arc;

    pub(super) const REPO: &str = "/repo";
    pub(super) const BRANCH: &str = "feat/x";
    pub(super) const BASE: &str = "main";

    /// Derived, never spelled. `sync_worktree_dir` shortens the branch slug on
    /// a Windows host, so a literal `/repo_wt_sync_feat-x` names the tree this
    /// code provisions on Linux and nothing at all anywhere else: under Wine
    /// every test in this module stopped at `worktree add`, came back
    /// `WorktreeProvision`, and asserted nothing whatever about the call it is
    /// named for.
    pub(super) fn wt() -> String {
        crate::paths::sync_worktree_dir(
            REPO,
            BRANCH,
            crate::paths::targets_windows_host(crate::domain::ids::LOCAL_MACHINE),
        )
    }

    const REV_PARSE_BASE: &str = "git -C /repo rev-parse --verify origin/main";

    pub(super) fn fetch_feature() -> String {
        format!("git -C {REPO} fetch origin -- {BRANCH}")
    }

    pub(super) fn feature_tracking_probe() -> String {
        format!("git -C {REPO} rev-parse --verify origin/{BRANCH}")
    }

    pub(super) fn worktree_add() -> String {
        format!("git -C {REPO} worktree add {} {BRANCH}", wt())
    }

    pub(super) fn merge() -> String {
        format!(
            "git -C {} merge origin/main -m chore(sync): sync feature with origin/main",
            wt()
        )
    }

    pub(super) fn push() -> String {
        format!("git -C {} push origin {BRANCH}", wt())
    }

    /// A sync that reaches the push, answered call by call. Every test below
    /// breaks exactly one entry, so the stage it gets back names that call and
    /// nothing else.
    pub(super) fn full_run() -> Vec<(String, Result<String, String>)> {
        let ok = |s: &str| Ok(s.to_string());
        let wt = wt();
        vec![
            ("git -C /repo fetch origin -- main".to_string(), ok("")),
            (REV_PARSE_BASE.to_string(), ok("beef")),
            // A branch origin has never seen, which is the first sync of every
            // feature: git answers the fetch of a ref it does not have with the
            // same non-zero exit as a broken remote, and the tracking ref then
            // resolves to nothing. Both halves are scripted rather than left
            // unanswered so a test that gives the branch an upstream reads as
            // the deliberate override it is.
            (
                fetch_feature(),
                Err("fatal: couldn't find remote ref feat/x".to_string()),
            ),
            (
                feature_tracking_probe(),
                Err("fatal: bad revision".to_string()),
            ),
            (
                "git -C /repo rev-parse refs/heads/feat/x".to_string(),
                ok("cafe"),
            ),
            (
                "git -C /repo rev-list --count origin/main..refs/heads/feat/x".to_string(),
                ok("0"),
            ),
            (
                "git -C /repo rev-list --count refs/heads/feat/x..origin/main".to_string(),
                ok("2"),
            ),
            (
                "git -C /repo rev-parse --abbrev-ref HEAD".to_string(),
                ok("main"),
            ),
            ("git -C /repo worktree list --porcelain".to_string(), ok("")),
            (worktree_add(), ok("")),
            (merge(), ok("")),
            (format!("git -C {wt} rev-parse HEAD"), ok("d00d")),
            (push(), ok("")),
            (
                format!("git -C {wt} status --porcelain --untracked-files=no"),
                ok("UU README.md"),
            ),
        ]
    }

    pub(super) fn as_script(
        run: &[(String, Result<String, String>)],
    ) -> Vec<(&str, Result<&str, &str>)> {
        run.iter()
            .map(|(k, v)| {
                (
                    k.as_str(),
                    match v {
                        Ok(s) => Ok(s.as_str()),
                        Err(e) => Err(e.as_str()),
                    },
                )
            })
            .collect()
    }

    /// A helper over a scripted run, and the double it answers from — which the
    /// caller needs to read back the calls the sync actually made.
    pub(super) fn helper_over(
        run: &[(String, Result<String, String>)],
    ) -> (Arc<ScriptedExec>, GitOpsHelper) {
        let exec = Arc::new(ScriptedExec::new(&[]).with_programs(&as_script(run)));
        let conn = Connection::open_in_memory().expect("in-memory db");
        let db =
            Arc::new(SqliteAdapter::new(conn).expect("adapter")) as Arc<dyn AppSettingsRepository>;
        let helper = GitOpsHelper::new(db, exec.clone());
        (exec, helper)
    }

    async fn sync_with(failing: &str, err: &str) -> SyncFailure {
        let run: Vec<(String, Result<String, String>)> = full_run()
            .into_iter()
            .map(|(key, answer)| {
                if key == failing {
                    (key, Err(err.to_string()))
                } else {
                    (key, answer)
                }
            })
            .collect();
        let (_, helper) = helper_over(&run);
        helper
            .sync_feature_with_upstream(None, REPO, BRANCH, BASE, MergeGate::default(), &())
            .await
            .expect_err("the scripted failure must not sync cleanly")
    }

    pub(super) fn stage_of(failure: SyncFailure) -> SyncBlockedStage {
        match failure {
            SyncFailure::Blocked { stage, .. } => stage,
            SyncFailure::Conflict {
                files, raw_error, ..
            } => {
                panic!(
                    "a call that never merged came back as a conflict over {files:?}: {raw_error}"
                )
            }
        }
    }

    /// Everything after this point can be cut short — the merge itself most of
    /// all — and a caller holding a durable row learns the worktree from the
    /// return value, which an interrupted sync never produces. So the row named
    /// no tree for the one state it exists for, and the directory was
    /// reclaimed only by the next sync's force-remove.
    ///
    /// The assertion is the *ordering*, not the value: told after the merge,
    /// this would still be told on every path that returns and still leave the
    /// interrupted one blind.
    #[tokio::test]
    async fn the_merge_worktree_is_reported_before_the_merge_runs() {
        #[derive(Default)]
        struct Recorder {
            path: std::sync::Mutex<Option<String>>,
            programs_before: std::sync::Mutex<usize>,
            exec: std::sync::Mutex<Option<Arc<ScriptedExec>>>,
        }
        impl crate::ports::worktree_ops::SyncWorktreeObserver for Recorder {
            fn provisioned(&self, worktree_path: &str) {
                *self.path.lock().unwrap() = Some(worktree_path.to_string());
                let issued = self
                    .exec
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|e| e.programs().len())
                    .unwrap_or_default();
                *self.programs_before.lock().unwrap() = issued;
            }
        }

        let (exec, helper) = helper_over(&full_run());
        let observer = Recorder::default();
        *observer.exec.lock().unwrap() = Some(exec.clone());

        helper
            .sync_feature_with_upstream(None, REPO, BRANCH, BASE, MergeGate::default(), &observer)
            .await
            .expect("the scripted run merges and pushes cleanly");

        assert_eq!(
            observer.path.lock().unwrap().as_deref(),
            Some(wt().as_str())
        );
        let issued = exec.programs();
        let before = *observer.programs_before.lock().unwrap();
        let merged_at = issued
            .iter()
            .position(|p| *p == merge())
            .expect("the run reaches its merge");
        assert!(
            before <= merged_at,
            "the worktree was reported after {} of {} calls, with the merge at {}: {:?}",
            before,
            issued.len(),
            merged_at,
            issued
        );
    }

    #[tokio::test]
    async fn the_base_ref_probe_is_not_the_fetch() {
        assert_eq!(
            stage_of(sync_with(REV_PARSE_BASE, "fatal: bad revision").await),
            SyncBlockedStage::BaseRefMissing
        );
    }

    #[tokio::test]
    async fn the_worktree_add_is_its_own_stage() {
        assert_eq!(
            stage_of(sync_with(&worktree_add(), "fatal: already checked out").await),
            SyncBlockedStage::WorktreeProvision
        );
    }

    #[tokio::test]
    async fn a_rejected_push_names_the_push_and_not_the_remote() {
        assert_eq!(
            stage_of(sync_with(&push(), "! [rejected] feat/x -> feat/x (fetch first)").await),
            SyncBlockedStage::Push
        );
    }

    #[tokio::test]
    async fn a_merge_the_transport_dropped_is_blocked_not_conflicted() {
        assert_eq!(
            stage_of(
                sync_with(
                    &merge(),
                    &format!("{TRANSPORT_ERROR_PREFIX}Connection appears dead")
                )
                .await
            ),
            SyncBlockedStage::Merge
        );
        assert_eq!(
            stage_of(sync_with(&merge(), &format!("{TIMEOUT_ERROR_PREFIX}exceeded 600s")).await),
            SyncBlockedStage::Merge
        );
    }

    /// The other half of that call: a merge that exited non-zero did reach a
    /// verdict, and the unmerged paths are read from the worktree it left.
    #[tokio::test]
    async fn a_merge_that_exited_non_zero_is_the_conflict() {
        match sync_with(&merge(), "CONFLICT (content): Merge conflict in README.md").await {
            SyncFailure::Conflict {
                files,
                worktree_path,
                ..
            } => {
                assert_eq!(files.len(), 1, "{files:?}");
                assert_eq!(files[0].path, "README.md");
                assert_eq!(worktree_path.as_deref(), Some(wt().as_str()));
            }
            SyncFailure::Blocked {
                stage, raw_error, ..
            } => {
                panic!("a merge that answered was filed as {stage:?}: {raw_error}")
            }
        }
    }
}

/// The feature branch's own upstream, which the sync has to reconcile before it
/// has anything safe to merge a base into. Scripted where
/// `test_sync_fast_forwards_the_feature_branch_from_origin_first` uses real git,
/// because these three properties are about calls that were or were not made —
/// which is a fact about the run, not about the tree it left.
mod feature_upstream {
    use super::stage_at_each_call::*;
    use super::*;
    use crate::ports::worktree_ops::SyncOutcome;

    fn ahead_count() -> String {
        format!("git -C {REPO} rev-list --count origin/{BRANCH}..refs/heads/{BRANCH}")
    }

    fn behind_count() -> String {
        format!("git -C {REPO} rev-list --count refs/heads/{BRANCH}..origin/{BRANCH}")
    }

    fn fast_forward() -> String {
        format!("git -C {} merge --ff-only origin/{BRANCH}", wt())
    }

    /// `full_run`, with `answers` replacing the entries it already has and
    /// adding the ones it does not — the baseline run is a branch origin has
    /// never seen, so every test here is an override of that.
    fn run_with(answers: &[(String, Result<&str, &str>)]) -> Vec<(String, Result<String, String>)> {
        let owned = |v: &Result<&str, &str>| match v {
            Ok(s) => Ok(s.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let mut run = full_run();
        for (key, answer) in answers {
            match run.iter_mut().find(|(k, _)| k == key) {
                Some(entry) => entry.1 = owned(answer),
                None => run.push((key.clone(), owned(answer))),
            }
        }
        run
    }

    async fn sync_over(
        run: &[(String, Result<String, String>)],
    ) -> (Vec<String>, Result<SyncOutcome, SyncFailure>) {
        let (exec, helper) = helper_over(run);
        let result = helper
            .sync_feature_with_upstream(None, REPO, BRANCH, BASE, MergeGate::default(), &())
            .await;
        (exec.programs(), result)
    }

    /// The fetch is the one call in the pair that is allowed to fail: a branch
    /// nobody has pushed and a remote that refused this one ref come back
    /// identically, and the first is every feature's first sync. What decides is
    /// the ref probe after it, which reads whatever the fetch did or did not
    /// leave behind.
    #[tokio::test]
    async fn a_feature_fetch_that_failed_does_not_stop_the_sync() {
        let (programs, result) = sync_over(&run_with(&[
            (fetch_feature(), Err("fatal: could not read from remote")),
            (feature_tracking_probe(), Ok("beef2")),
            (behind_count(), Ok("0")),
            (ahead_count(), Ok("0")),
        ]))
        .await;

        result.expect("a best-effort fetch that failed must not block the merge");
        assert!(
            programs.contains(&merge()),
            "the sync stopped before its merge: {programs:?}"
        );
    }

    /// The fast-forward runs in the checkout and before the base merge, which is
    /// the whole of what it is for: run after, it would be merging origin's
    /// commits into a merge commit that was already written without them.
    #[tokio::test]
    async fn origin_is_merged_into_the_branch_only_after_the_branch_catches_up() {
        let (programs, result) = sync_over(&run_with(&[
            (fetch_feature(), Ok("")),
            (feature_tracking_probe(), Ok("beef2")),
            (behind_count(), Ok("2")),
            (ahead_count(), Ok("0")),
            (fast_forward(), Ok("")),
        ]))
        .await;

        let outcome = result.expect("a branch origin has simply moved past fast-forwards");
        assert_eq!(
            outcome.head_before.as_deref(),
            Some("d00d"),
            "head_before must be re-read in the worktree, not left at the pre-fast-forward ref"
        );
        let ff_at = programs.iter().position(|p| *p == fast_forward());
        let merge_at = programs.iter().position(|p| *p == merge());
        assert!(
            ff_at < merge_at && ff_at.is_some(),
            "fast-forward at {ff_at:?}, base merge at {merge_at:?}: {programs:?}"
        );
    }

    #[tokio::test]
    async fn a_counted_divergence_is_refused_before_a_worktree_exists() {
        let (programs, result) = sync_over(&run_with(&[
            (fetch_feature(), Ok("")),
            (feature_tracking_probe(), Ok("beef2")),
            (behind_count(), Ok("1")),
            (ahead_count(), Ok("2")),
        ]))
        .await;

        let failure = result.expect_err("a diverged branch has nothing safe to merge into");
        assert_eq!(stage_of(failure), SyncBlockedStage::FeatureDiverged);
        assert!(
            !programs.contains(&worktree_add()) && !programs.contains(&merge()),
            "nothing may be provisioned or merged over a divergence: {programs:?}"
        );
    }

    /// The same refusal from git rather than from the counts, which is the
    /// reading that survives a `rev-list` that did not answer — and the one that
    /// has git's own words to show.
    #[tokio::test]
    async fn a_fast_forward_git_refuses_is_the_same_refusal() {
        let (programs, result) = sync_over(&run_with(&[
            (fetch_feature(), Ok("")),
            (feature_tracking_probe(), Ok("beef2")),
            (behind_count(), Ok("2")),
            (ahead_count(), Ok("0")),
            (
                fast_forward(),
                Err("fatal: Not possible to fast-forward, aborting."),
            ),
        ]))
        .await;

        match result.expect_err("a refused fast-forward leaves nothing safe to merge into") {
            SyncFailure::Blocked {
                stage,
                raw_error,
                worktree_path,
                ..
            } => {
                assert_eq!(stage, SyncBlockedStage::FeatureDiverged);
                assert!(
                    raw_error.contains("fatal: Not possible to fast-forward, aborting."),
                    "git's own words are the only account of what it refused: {raw_error}"
                );
                assert_eq!(worktree_path.as_deref(), Some(wt().as_str()));
            }
            SyncFailure::Conflict { raw_error, .. } => {
                panic!("nothing was merged, so nothing can be conflicted: {raw_error}")
            }
        }
        assert!(
            !programs.contains(&merge()),
            "the base must not be merged onto a branch that could not catch up: {programs:?}"
        );
    }
}
