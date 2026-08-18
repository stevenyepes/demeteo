use super::super::common::*;
use crate::domain::sync_failure::SyncBlockedStage;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::SyncFailure;

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
        .sync_feature_with_upstream(None, &local, "feature/f-1", "main")
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
        .sync_feature_with_upstream(None, &local, "feature/f-1", "main")
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
        .sync_feature_with_upstream(None, &local, "feature/f-1", "main")
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
        .sync_feature_with_upstream(None, &local, "feature/f-resolver", "main")
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

    const REPO: &str = "/repo";
    const BRANCH: &str = "feat/x";
    const BASE: &str = "main";
    const WT: &str = "/repo_wt_sync_feat-x";

    const REV_PARSE_BASE: &str = "git -C /repo rev-parse --verify origin/main";
    const WORKTREE_ADD: &str = "git -C /repo worktree add /repo_wt_sync_feat-x feat/x";
    const MERGE: &str = "git -C /repo_wt_sync_feat-x merge origin/main \
                         -m chore(sync): sync feature with origin/main";
    const PUSH: &str = "git -C /repo_wt_sync_feat-x push origin feat/x";

    /// A sync that reaches the push, answered call by call. Every test below
    /// breaks exactly one entry, so the stage it gets back names that call and
    /// nothing else.
    fn full_run() -> Vec<(&'static str, Result<&'static str, &'static str>)> {
        vec![
            ("git -C /repo fetch origin -- main", Ok("")),
            (REV_PARSE_BASE, Ok("beef")),
            ("git -C /repo rev-parse refs/heads/feat/x", Ok("cafe")),
            (
                "git -C /repo rev-list --count origin/main..refs/heads/feat/x",
                Ok("0"),
            ),
            (
                "git -C /repo rev-list --count refs/heads/feat/x..origin/main",
                Ok("2"),
            ),
            ("git -C /repo rev-parse --abbrev-ref HEAD", Ok("main")),
            ("git -C /repo worktree list --porcelain", Ok("")),
            (WORKTREE_ADD, Ok("")),
            (MERGE, Ok("")),
            ("git -C /repo_wt_sync_feat-x rev-parse HEAD", Ok("d00d")),
            (PUSH, Ok("")),
            (
                "git -C /repo_wt_sync_feat-x status --porcelain --untracked-files=no",
                Ok("UU README.md"),
            ),
        ]
    }

    async fn sync_with(failing: &str, err: &str) -> SyncFailure {
        let programs: Vec<(&str, Result<&str, &str>)> = full_run()
            .into_iter()
            .map(|(key, answer)| {
                if key == failing {
                    (key, Err(err))
                } else {
                    (key, answer)
                }
            })
            .collect();
        let conn = Connection::open_in_memory().expect("in-memory db");
        let db =
            Arc::new(SqliteAdapter::new(conn).expect("adapter")) as Arc<dyn AppSettingsRepository>;
        let helper = GitOpsHelper::new(
            db,
            Arc::new(ScriptedExec::new(&[]).with_programs(&programs)),
        );
        helper
            .sync_feature_with_upstream(None, REPO, BRANCH, BASE)
            .await
            .expect_err("the scripted failure must not sync cleanly")
    }

    fn stage_of(failure: SyncFailure) -> SyncBlockedStage {
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
            stage_of(sync_with(WORKTREE_ADD, "fatal: already checked out").await),
            SyncBlockedStage::WorktreeProvision
        );
    }

    #[tokio::test]
    async fn a_rejected_push_names_the_push_and_not_the_remote() {
        assert_eq!(
            stage_of(sync_with(PUSH, "! [rejected] feat/x -> feat/x (fetch first)").await),
            SyncBlockedStage::Push
        );
    }

    #[tokio::test]
    async fn a_merge_the_transport_dropped_is_blocked_not_conflicted() {
        assert_eq!(
            stage_of(
                sync_with(
                    MERGE,
                    &format!("{TRANSPORT_ERROR_PREFIX}Connection appears dead")
                )
                .await
            ),
            SyncBlockedStage::Merge
        );
        assert_eq!(
            stage_of(sync_with(MERGE, &format!("{TIMEOUT_ERROR_PREFIX}exceeded 600s")).await),
            SyncBlockedStage::Merge
        );
    }

    /// The other half of that call: a merge that exited non-zero did reach a
    /// verdict, and the unmerged paths are read from the worktree it left.
    #[tokio::test]
    async fn a_merge_that_exited_non_zero_is_the_conflict() {
        match sync_with(MERGE, "CONFLICT (content): Merge conflict in README.md").await {
            SyncFailure::Conflict {
                files,
                worktree_path,
                ..
            } => {
                assert_eq!(files.len(), 1, "{files:?}");
                assert_eq!(files[0].path, "README.md");
                assert_eq!(worktree_path.as_deref(), Some(WT));
            }
            SyncFailure::Blocked {
                stage, raw_error, ..
            } => {
                panic!("a merge that answered was filed as {stage:?}: {raw_error}")
            }
        }
    }
}
