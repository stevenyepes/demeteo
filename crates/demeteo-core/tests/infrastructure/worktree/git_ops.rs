use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use rusqlite::Connection;
use std::path::PathBuf;

#[tokio::test]
async fn test_detect_worktree_strategy_local() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_gitops_detect_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Run git init and config
    let local_exec = LocalSubprocessAdapter::new();
    let _ = local_exec
        .run_command(
            "local",
            &format!("git -C \"{}\" init -b main", temp_dir.to_string_lossy()),
        )
        .await;
    // Create mock files
    local_exec
        .write_file(
            "local",
            &format!("{}/package.json", temp_dir.to_string_lossy()),
            "{}",
        )
        .await
        .unwrap();
    local_exec
        .write_file(
            "local",
            &format!(
                "{}/.github/pull_request_template.md",
                temp_dir.to_string_lossy()
            ),
            "PR Template Content",
        )
        .await
        .unwrap();
    // Commit so HEAD branch is set
    let _ = local_exec
        .run_command(
            "local",
            &format!(
                "git -C \"{}\" config user.email \"test@demeteo.com\"",
                temp_dir.to_string_lossy()
            ),
        )
        .await;
    let _ = local_exec
        .run_command(
            "local",
            &format!(
                "git -C \"{}\" config user.name \"test\"",
                temp_dir.to_string_lossy()
            ),
        )
        .await;
    let _ = local_exec
        .run_command(
            "local",
            &format!("git -C \"{}\" add .", temp_dir.to_string_lossy()),
        )
        .await;
    let _ = local_exec
        .run_command(
            "local",
            &format!(
                "git -C \"{}\" commit -m \"Initial commit\"",
                temp_dir.to_string_lossy()
            ),
        )
        .await;

    // Initialize helper
    let conn = Connection::open_in_memory().unwrap();
    let db_adapter = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
    let git_ops = GitOpsHelper::new(db_adapter, Arc::new(local_exec));

    let strategy = git_ops
        .detect_worktree_strategy(None, &temp_dir.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(strategy.default_branch, "main");
    assert_eq!(strategy.test_command, Some("npm test".to_string()));
    assert_eq!(
        strategy.pr_template,
        Some("PR Template Content".to_string())
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// Helper: create a fresh git repo in a temp dir and return (repo_dir, git_ops).
async fn make_repo(suffix: &str) -> (std::path::PathBuf, GitOpsHelper) {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_{}_{}",
        suffix,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let exec = LocalSubprocessAdapter::new();
    let repo = temp_dir.to_string_lossy().to_string();

    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" init -b main"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" config user.email \"ci@demeteo.com\""),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" config user.name \"CI\""),
        )
        .await;
    exec.write_file("local", &format!("{repo}/README.md"), "# test")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" add ."))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" commit -m \"init\""))
        .await;

    let conn = Connection::open_in_memory().unwrap();
    let db = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
    let helper = GitOpsHelper::new(db, Arc::new(exec));
    (temp_dir, helper)
}

#[tokio::test]
async fn test_get_head_branch_returns_main() {
    let (dir, helper) = make_repo("head_branch").await;
    let branch = helper.get_head_branch(None, &dir.to_string_lossy()).await;
    assert_eq!(
        branch,
        Some("main".to_string()),
        "Expected HEAD to be 'main' after `git init -b main`"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn test_get_head_branch_missing_dir_returns_none() {
    let conn = Connection::open_in_memory().unwrap();
    let db = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
    let helper = GitOpsHelper::new(db, Arc::new(LocalSubprocessAdapter::new()));
    let result = helper
        .get_head_branch(None, "/tmp/demeteo_nonexistent_repo_xyz")
        .await;
    assert!(
        result.is_none(),
        "Expected None for a path that is not a git repo"
    );
}

#[tokio::test]
async fn test_list_worktrees_only_main_when_no_worktrees_added() {
    let (dir, helper) = make_repo("wt_main_only").await;
    let worktrees = helper
        .list_worktrees(None, &dir.to_string_lossy())
        .await
        .unwrap();
    // list_worktrees skips the primary worktree entry, so the result is empty
    assert!(
        worktrees.is_empty(),
        "Expected no additional worktrees beyond the main checkout, got: {:?}",
        worktrees
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn test_list_worktrees_with_one_extra_worktree() {
    let (dir, helper) = make_repo("wt_extra").await;
    // Canonicalize to handle macOS /tmp → /private/tmp symlink.
    // TempDir may return the symlink path while git worktree list
    // returns the real path, causing an assertion mismatch.
    let repo = std::fs::canonicalize(&dir)
        .unwrap_or_else(|_| dir.as_os_str().to_os_string().into())
        .to_string_lossy()
        .to_string();

    // Add a linked worktree on a new branch
    let wt_dir = format!("{}-wt", repo);
    let exec_tmp = LocalSubprocessAdapter::new();
    let _ = exec_tmp
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree add \"{wt_dir}\" -b feature/my-task"),
        )
        .await;

    let worktrees = helper.list_worktrees(None, &repo).await.unwrap();
    assert_eq!(worktrees.len(), 1, "Expected exactly one linked worktree");
    let wt = &worktrees[0];
    assert_eq!(wt.path, wt_dir, "Worktree path should match the added dir");
    assert_eq!(
        wt.branch.as_deref(),
        Some("feature/my-task"),
        "Branch name should be stripped of 'refs/heads/' prefix"
    );
    assert!(!wt.is_locked, "Newly added worktree should not be locked");

    // Cleanup (prune first so git lets us remove the dir)
    let _ = exec_tmp
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_dir}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_dir);
}

#[tokio::test]
async fn test_provision_subtask_worktree_fallback_when_branch_exists() {
    let (dir, helper) = make_repo("wt_fallback").await;
    let repo = dir.to_string_lossy().to_string();

    // Create the subtask branch manually first so that creating it again via -b fails
    let exec = LocalSubprocessAdapter::new();
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" branch main_subtask_sub-1"),
        )
        .await;

    // Now provision the worktree — it should fall back to checking out the existing branch and succeed
    let wt_path = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-1")
        .await
        .unwrap();

    // Verify the worktree path exists
    assert!(std::path::Path::new(&wt_path).exists());

    // Cleanup
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_path}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

/// Regression: an orphan worktree directory left behind by an
/// interrupted run must not cause the next provision to fail with
/// "'<path>' already exists". Before the fix the cleanup was just
/// `rm -rf` whose error was discarded; if the dir couldn't be removed
/// (permission, busy, mount) the subsequent `git worktree add` failed.
#[tokio::test]
async fn test_provision_subtask_worktree_handles_orphan_dir() {
    let (dir, helper) = make_repo("wt_orphan").await;
    let repo = dir.to_string_lossy().to_string();

    // Pre-create the exact path provision_subtask_worktree would use,
    // as an orphan dir NOT registered with git.
    let wt_path = format!("{}_wt_sub-orphan", repo);
    std::fs::create_dir_all(&wt_path).unwrap();
    std::fs::write(format!("{wt_path}/leftover.txt"), "from crashed run").unwrap();
    assert!(std::path::Path::new(&wt_path).exists());

    // Provision should clean up the orphan and create the worktree.
    let result = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-orphan")
        .await;
    assert!(
        result.is_ok(),
        "provision with orphan dir should succeed; got {:?}",
        result
    );
    assert!(std::path::Path::new(&wt_path).exists(), "wt should exist");
    assert!(
        !std::path::Path::new(&format!("{wt_path}/leftover.txt")).exists(),
        "leftover file should be gone"
    );

    // Cleanup.
    let exec = LocalSubprocessAdapter::new();
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_path}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

/// Regression: a worktree that IS registered with git (e.g. from a
/// previous run that didn't clean up) must not block re-provisioning.
#[tokio::test]
async fn test_provision_subtask_worktree_handles_registered_worktree() {
    let (dir, helper) = make_repo("wt_registered").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // First provision: create the worktree normally.
    helper
        .provision_subtask_worktree(None, &repo, "main", "sub-reg")
        .await
        .unwrap();
    let wt_path = format!("{}_wt_sub-reg", repo);
    assert!(std::path::Path::new(&wt_path).exists());

    // Simulate an interrupted run: worktree is registered with git
    // but the next provision still needs to take over. Don't clean
    // up; just call provision again.
    let result = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-reg")
        .await;
    assert!(
        result.is_ok(),
        "re-provision over registered worktree should succeed; got {:?}",
        result
    );

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_path}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

/// Two features with the same step id on the same project must get
/// distinct worktree directories. The call site now scopes the
/// `subtask_id` by `feature_id` so `{repo}_wt_{subtask_id}` is unique
/// per feature. This test calls the helper directly with two distinct
/// `subtask_id`s (the feature-scoped ones a parallel run would
/// produce) and asserts they produce disjoint paths.
#[tokio::test]
async fn test_provision_subtask_worktree_distinct_per_feature() {
    let (dir_a, helper_a) = make_repo("wt_par_a").await;
    let (dir_b, helper_b) = make_repo("wt_par_b").await;

    // Two features, same step id. Feature-scoped subtask_ids.
    let wt_a = helper_a
        .provision_subtask_worktree(
            None,
            &dir_a.to_string_lossy(),
            "main",
            "f-A-step-s-research",
        )
        .await
        .expect("feature A should provision");
    let wt_b = helper_b
        .provision_subtask_worktree(
            None,
            &dir_b.to_string_lossy(),
            "main",
            "f-B-step-s-research",
        )
        .await
        .expect("feature B should provision");

    // Even though the repos are different in this test (to keep the
    // git-side bookkeeping separate), the wt_dir suffixes reflect
    // the feature-scoped subtask_id, so two features on the *same*
    // repo would also be distinct.
    assert!(wt_a.contains("f-A-step-s-research"));
    assert!(wt_b.contains("f-B-step-s-research"));
    assert_ne!(wt_a, wt_b);

    // Cleanup.
    let exec = LocalSubprocessAdapter::new();
    for (repo, wt) in [
        (dir_a.to_string_lossy().to_string(), wt_a),
        (dir_b.to_string_lossy().to_string(), wt_b),
    ] {
        let _ = exec
            .run_command(
                "local",
                &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
            )
            .await;
        let _ = std::fs::remove_dir_all(repo);
        let _ = std::fs::remove_dir_all(wt);
    }
}

/// Regression: a previous run that applied the artifact-scope fence
/// (`chmod -R a-w` on protected paths) leaves the worktree in a state
/// where `rm -rf` cannot traverse it — `unlink()` needs write on the
/// parent directory, not the file itself, so an `a-w src/` blocks
/// cleanup. The provisioner must restore write permissions before
/// removing the directory, otherwise the next redirect or retry hits
/// "already exists" with no clear way to recover.
#[tokio::test]
async fn test_provision_subtask_worktree_handles_chmod_locked_leftover() {
    let (dir, helper) = make_repo("wt_chmod_locked").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // Pre-create the wt path as an unregistered dir with a protected
    // subdirectory chmod'd to a-w — mimicking what the scope fence
    // leaves behind after a crashed run.
    let wt_path = format!("{}_wt_sub-chmod", repo);
    std::fs::create_dir_all(format!("{wt_path}/src")).unwrap();
    std::fs::write(format!("{wt_path}/src/main.rs"), "fn main() {}").unwrap();
    let _ = exec
        .run_command("local", &format!("chmod -R a-w '{wt_path}'"))
        .await;

    // Sanity: with the chmod applied, an unwary `rm -rf` would fail
    // to remove `src/main.rs` because `src/` itself is a-w.
    let naive_rm = exec
        .run_command("local", &format!("rm -rf '{wt_path}/src/main.rs'"))
        .await;
    assert!(
        naive_rm.is_err(),
        "sanity: chmod a-w on src/ should block naive rm on src/main.rs"
    );

    // Provision should chmod u+w back, then rm -rf successfully, and
    // create the worktree fresh.
    let result = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-chmod")
        .await;
    assert!(
        result.is_ok(),
        "provision with chmod-locked leftover should succeed; got {:?}",
        result
    );
    assert!(std::path::Path::new(&wt_path).exists());

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_path}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

/// Set up two local repos and wire them together as fake
/// origin/main. The "remote" is a regular working tree that
/// we push to via a bare-clone URL; the "local" is a normal
/// working tree that we sync from. Both start with the same
/// initial commit. The caller mutates each side to set up the
/// upstream/feature divergence before calling
/// `sync_feature_with_upstream`.
async fn make_two_repos(suffix: &str) -> (PathBuf, PathBuf, GitOpsHelper) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let remote_dir = std::env::temp_dir().join(format!("demeteo_test_remote_{}_{}", suffix, stamp));
    let local_dir = std::env::temp_dir().join(format!("demeteo_test_local_{}_{}", suffix, stamp));
    std::fs::create_dir_all(&remote_dir).unwrap();
    std::fs::create_dir_all(&local_dir).unwrap();
    let exec = LocalSubprocessAdapter::new();

    // 1. The "remote" is a regular working tree that we push
    //    to. We disable the safety check so we can push to the
    //    currently checked-out branch.
    let remote = remote_dir.to_string_lossy().to_string();
    let _ = exec
        .run_command("local", &format!("git init -b main \"{remote}\""))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" config user.email \"ci@demeteo.com\""),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" config user.name \"CI\""),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" config receive.denyCurrentBranch ignore"),
        )
        .await;
    exec.write_file("local", &format!("{remote}/README.md"), "init")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" add ."))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" commit -m init"))
        .await;

    // 2. The "local" is a clone of the remote so it shares the
    //    initial commit and has `origin` already wired up.
    let local = local_dir.to_string_lossy().to_string();
    let _ = exec
        .run_command("local", &format!("git clone \"{remote}\" \"{local}\""))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" config user.email \"ci@demeteo.com\""),
        )
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" config user.name \"CI\""),
        )
        .await;

    let conn = Connection::open_in_memory().unwrap();
    let db = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
    let helper = GitOpsHelper::new(db, Arc::new(exec));
    (local_dir, remote_dir, helper)
}

/// The exact bug the user hit: a feature branch is "2 commits
/// behind" main with overlapping changes. The sync must
/// surface the conflict list, not silently return "no new
/// commits upstream".
#[tokio::test]
async fn test_sync_feature_with_upstream_detects_conflicts() {
    let (local_dir, remote_dir, helper) = make_two_repos("sync_conflict").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

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
        Err(failure) => {
            assert!(
                !failure.files.is_empty(),
                "Sync reported failure but no conflict files were captured. \
                 raw_error: {}",
                failure.raw_error
            );
            assert!(
                failure.files.iter().any(|f| f.path == "README.md"),
                "README.md should be in the conflict list, got: {:?}",
                failure.files
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
    let exec = LocalSubprocessAdapter::new();

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
    let exec = LocalSubprocessAdapter::new();

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
        Err(failure) => {
            assert!(
                failure.raw_error.to_lowercase().contains("fetch")
                    || failure.raw_error.to_lowercase().contains("origin")
                    || failure.raw_error.to_lowercase().contains("remote"),
                "Error message should mention the fetch/remote failure, got: {}",
                failure.raw_error
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
    let exec = LocalSubprocessAdapter::new();

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

// ─────────────────────────────────────────────────────────────────────────────
// Artifact-scope enforcement
//
// Covers the chmod fence and the post-step diff guard against a real git
// repo. The chmod fence stops honest mistakes at write time; the diff guard
// is the safety net that catches any bypass (e.g. `chmod u+w .` shell
// escape) before the bad changes reach the merge step.

#[tokio::test]
async fn test_scope_chmod_blocks_out_of_scope_writes() {
    let (dir, helper) = make_repo("scope_block").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // Add a source file the agent must not touch.
    exec.write_file("local", &format!("{repo}/src/main.rs"), "fn main() {}")
        .await
        .unwrap();
    exec.run_command("local", &format!("git -C \"{repo}\" add ."))
        .await
        .unwrap();
    exec.run_command("local", &format!("git -C \"{repo}\" commit -m addsrc"))
        .await
        .unwrap();

    // Open a worktree at the existing HEAD so chmod operates on a real
    // working tree (the helper expects the dir to exist and be a worktree).
    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // Apply scope: only `artifacts/report.md` is writable.
    let writable = vec![std::path::PathBuf::from("artifacts/report.md")];
    helper
        .apply_artifact_scope(None, &wt, &writable)
        .await
        .expect("scope setup should succeed");

    // 1. A write to `src/main.rs` should now fail (chmod a-w on src/).
    let bad_write = exec
        .write_file("local", &format!("{wt}/src/main.rs"), "hijacked")
        .await;
    assert!(
        bad_write.is_err(),
        "write to protected path should fail under scope fence"
    );

    // 2. A write to the allowed artifacts path should succeed.
    std::fs::create_dir_all(format!("{wt}/artifacts")).unwrap();
    let good_write = exec
        .write_file("local", &format!("{wt}/artifacts/report.md"), "# report")
        .await;
    assert!(
        good_write.is_ok(),
        "write to allowed artifacts path should succeed"
    );

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_scope_diff_guard_reverts_out_of_scope_writes() {
    let (dir, helper) = make_repo("scope_revert").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // Source file committed to HEAD.
    exec.write_file("local", &format!("{repo}/src/lib.rs"), "pub fn ok() {}")
        .await
        .unwrap();
    exec.run_command("local", &format!("git -C \"{repo}\" add ."))
        .await
        .unwrap();
    exec.run_command("local", &format!("git -C \"{repo}\" commit -m addsrc"))
        .await
        .unwrap();

    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // Simulate the agent bypassing the chmod fence (e.g. via
    // `chmod u+w .` shell escape) and writing to a protected path.
    std::fs::create_dir_all(format!("{wt}/src")).unwrap();
    std::fs::write(
        format!("{wt}/src/lib.rs"),
        "pub fn hijacked() {} // agent ran chmod u+w and modified me",
    )
    .unwrap();
    // And an untracked file too.
    std::fs::write(format!("{wt}/src/new_file.rs"), "evil").unwrap();

    // Run the diff guard. Writable set: only `artifacts/`.
    let writable = vec![std::path::PathBuf::from("artifacts")];
    let reverted = helper
        .verify_and_revert_out_of_scope_writes(None, &wt, &writable)
        .await
        .expect("diff guard should succeed");

    // Both writes should be reported as reverted.
    assert!(
        reverted.iter().any(|p| p == "src/lib.rs"),
        "expected src/lib.rs in reverted list, got {:?}",
        reverted
    );
    assert!(
        reverted.iter().any(|p| p == "src/new_file.rs"),
        "expected src/new_file.rs in reverted list, got {:?}",
        reverted
    );

    // The tracked file should be back to its committed content.
    let restored = std::fs::read_to_string(format!("{wt}/src/lib.rs")).unwrap();
    assert_eq!(restored, "pub fn ok() {}");

    // The untracked file should be gone.
    assert!(!std::path::Path::new(&format!("{wt}/src/new_file.rs")).exists());

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_scope_diff_guard_keeps_in_scope_writes() {
    let (dir, helper) = make_repo("scope_keep").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // Agent writes the report inside the allowed scope.
    std::fs::create_dir_all(format!("{wt}/artifacts")).unwrap();
    std::fs::write(
        format!("{wt}/artifacts/research-report.md"),
        "# Research Report\n",
    )
    .unwrap();

    // Diff guard runs with the allowed scope. Should report nothing
    // reverted and leave the file in place.
    let writable = vec![std::path::PathBuf::from("artifacts/research-report.md")];
    let reverted = helper
        .verify_and_revert_out_of_scope_writes(None, &wt, &writable)
        .await
        .expect("diff guard should succeed");
    assert!(
        reverted.is_empty(),
        "in-scope write should not be reverted; got {:?}",
        reverted
    );

    let content = std::fs::read_to_string(format!("{wt}/artifacts/research-report.md")).unwrap();
    assert_eq!(content, "# Research Report\n");

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_scope_all_writes_sentinel_disables_enforcement() {
    let (dir, helper) = make_repo("scope_off").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    let wt = format!("{}_wt", repo);
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" worktree add \"{wt}\" HEAD"),
    )
    .await
    .unwrap();

    // `s-implement`'s parallel capture is AllWrites → sentinel returned.
    let writable = vec![std::path::PathBuf::from("__ALL_WRITES__")];

    // chmod fence is a no-op: file remains writable.
    helper
        .apply_artifact_scope(None, &wt, &writable)
        .await
        .unwrap();
    std::fs::create_dir_all(format!("{wt}/src")).unwrap();
    assert!(
        exec.write_file("local", &format!("{wt}/src/main.rs"), "ok")
            .await
            .is_ok(),
        "AllWrites sentinel must leave the worktree fully writable"
    );

    // Diff guard is a no-op: writes are not reverted.
    std::fs::write(format!("{wt}/src/whatever.rs"), "fine").unwrap();
    let reverted = helper
        .verify_and_revert_out_of_scope_writes(None, &wt, &writable)
        .await
        .unwrap();
    assert!(reverted.is_empty());

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// Regression: a fresh `git worktree add` doesn't carry over gitignored
/// dependency caches (`node_modules/`, `target/`, …) because they were
/// never committed. Without symlinking them in from the primary
/// checkout, any harness run inside the subtask worktree (`npm test`,
/// `cargo test`) fails immediately on missing dependencies.
#[tokio::test]
async fn test_provision_subtask_worktree_symlinks_dependency_caches() {
    let (dir, helper) = make_repo("wt_dep_link").await;
    let repo = dir.to_string_lossy().to_string();

    // Simulate an already-installed primary checkout: a gitignored
    // `node_modules/` with real content, plus a `target/` dir that is
    // NOT gitignored (e.g. a misconfigured or unusual project) — this
    // one must NOT be linked.
    std::fs::write(format!("{repo}/.gitignore"), "node_modules/\n").unwrap();
    std::fs::create_dir_all(format!("{repo}/node_modules")).unwrap();
    std::fs::write(format!("{repo}/node_modules/pkg.js"), "module.exports = 1;").unwrap();
    std::fs::create_dir_all(format!("{repo}/target")).unwrap();
    std::fs::write(format!("{repo}/target/artifact.bin"), "binary").unwrap();
    let exec = LocalSubprocessAdapter::new();
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" add .gitignore"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" commit -m \"add gitignore\""),
        )
        .await;

    let wt_path = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-deps")
        .await
        .expect("provision should succeed");

    // node_modules/ is gitignored in the primary checkout → symlinked in,
    // and readable through the link.
    let linked_pkg = format!("{wt_path}/node_modules/pkg.js");
    assert!(
        std::path::Path::new(&linked_pkg).exists(),
        "expected node_modules/pkg.js to be reachable via symlink in the worktree"
    );
    let meta = std::fs::symlink_metadata(format!("{wt_path}/node_modules")).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "expected node_modules to be a symlink in the subtask worktree"
    );

    // target/ is NOT gitignored in this repo → must be left alone (git's
    // own worktree checkout, empty, not our symlink).
    let target_meta = std::fs::symlink_metadata(format!("{wt_path}/target"));
    assert!(
        target_meta.is_err(),
        "target/ was not gitignored in this repo and must not be symlinked"
    );

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_path}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

/// Regression: git does not recognize a symlink standing in for a
/// directory as matching a trailing-slash `.gitignore` pattern (e.g.
/// `node_modules/`), so the symlinked dependency cache shows up as
/// untracked. `commit_worktree_changes`'s `git add -A` must not stage
/// it — committing an absolute host path onto the feature branch would
/// corrupt the branch for anyone else who checks it out.
#[tokio::test]
async fn test_commit_worktree_changes_never_stages_symlinked_dependency_caches() {
    let (dir, helper) = make_repo("wt_dep_commit").await;
    let repo = dir.to_string_lossy().to_string();

    std::fs::write(format!("{repo}/.gitignore"), "node_modules/\n").unwrap();
    std::fs::create_dir_all(format!("{repo}/node_modules")).unwrap();
    std::fs::write(format!("{repo}/node_modules/pkg.js"), "module.exports = 1;").unwrap();
    let exec = LocalSubprocessAdapter::new();
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" add .gitignore"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" commit -m \"add gitignore\""),
        )
        .await;

    let wt_path = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-commit")
        .await
        .expect("provision should succeed");

    // Sanity: confirm the symlink is present before committing (same
    // assertion as the provisioning test, kept local so this test is
    // self-contained if run in isolation).
    let meta = std::fs::symlink_metadata(format!("{wt_path}/node_modules")).unwrap();
    assert!(meta.file_type().is_symlink());

    // A real change the agent made, alongside the pre-existing symlink.
    std::fs::write(format!("{wt_path}/feature.txt"), "actual work").unwrap();

    let sha = crate::adapters::step_executor::artifacts::commit_worktree_changes(
        &exec,
        "local",
        &wt_path,
        "test commit",
        "artifacts/",
        false,
        &[],
    )
    .await
    .expect("commit should succeed");

    let committed_files = exec
        .run_command(
            "local",
            &format!("git -C \"{wt_path}\" show --name-only --pretty=format: {sha}"),
        )
        .await
        .unwrap();
    assert!(
        committed_files.contains("feature.txt"),
        "the real change must still be committed: {committed_files}"
    );
    assert!(
        !committed_files.contains("node_modules"),
        "the symlinked dependency cache must never be committed: {committed_files}"
    );

    // The working tree must still have the symlink intact (unaffected by
    // the exclusion — we skip *staging* it, not touch it on disk).
    let meta_after = std::fs::symlink_metadata(format!("{wt_path}/node_modules")).unwrap();
    assert!(meta_after.file_type().is_symlink());

    // Cleanup.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_path}\""),
        )
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

/// Regression: the critic (and any other reviewer) must see the
/// *complete* feature diff after a retry, not just the latest
/// incremental fix. This reproduces the exact scenario reported: an
/// implement step merges v1 into the feature branch, validation fails,
/// the implement step retries and merges v2. A per-attempt base SHA
/// (recaptured as the feature branch's tip at the start of the retry)
/// already includes v1's commits, so a diff computed from it only shows
/// v2 — `merge_base` against the default branch must return the
/// original fork point regardless, so a diff computed from it covers
/// v1 and v2 together.
#[tokio::test]
async fn test_merge_base_stays_at_fork_point_across_retries() {
    let (dir, helper) = make_repo("merge_base_fork_point").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // The fork point: where "feature" branches off "main".
    let fork_point = exec
        .run_command("local", &format!("git -C \"{repo}\" rev-parse HEAD"))
        .await
        .unwrap()
        .trim()
        .to_string();
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" branch feature"))
        .await;

    // Attempt 1 (v1): merged into the feature branch, as a successful
    // implement step would do.
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" checkout feature"))
        .await;
    std::fs::write(format!("{repo}/v1.txt"), "v1 change").unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" add v1.txt"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" commit -m \"v1 implement\""),
        )
        .await;

    // The buggy per-attempt base: `rev-parse feature` recaptured at the
    // start of the retry — already past v1.
    let per_attempt_base_v2 = exec
        .run_command("local", &format!("git -C \"{repo}\" rev-parse feature"))
        .await
        .unwrap()
        .trim()
        .to_string();
    assert_ne!(
        per_attempt_base_v2, fork_point,
        "sanity: v1's commit must have moved the per-attempt base past the fork point"
    );

    // Attempt 2 (v2): the retry's incremental fix, also merged in.
    std::fs::write(format!("{repo}/v2.txt"), "v2 fix").unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" add v2.txt"))
        .await;
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" commit -m \"v2 retry fix\""),
        )
        .await;

    // `merge_base` must still return the ORIGINAL fork point, not the
    // per-attempt base captured mid-retry.
    let resolved = helper
        .merge_base(None, &repo, "main", "feature")
        .await
        .expect("merge_base should resolve");
    assert_eq!(
        resolved, fork_point,
        "merge_base must stay at the true fork point across retries"
    );

    // Prove the practical consequence: a diff from the fork point
    // contains both v1 and v2; a diff from the buggy per-attempt base
    // contains only v2.
    let full_diff = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" diff {resolved}..feature --name-only"),
        )
        .await
        .unwrap();
    assert!(full_diff.contains("v1.txt"), "full_diff: {full_diff}");
    assert!(full_diff.contains("v2.txt"), "full_diff: {full_diff}");

    let buggy_diff = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" diff {per_attempt_base_v2}..feature --name-only"),
        )
        .await
        .unwrap();
    assert!(
        !buggy_diff.contains("v1.txt"),
        "sanity: this demonstrates the bug being fixed — buggy_diff: {buggy_diff}"
    );
    assert!(buggy_diff.contains("v2.txt"), "buggy_diff: {buggy_diff}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_merge_base_returns_none_for_unrelated_branches() {
    let conn = Connection::open_in_memory().unwrap();
    let db = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
    let helper = GitOpsHelper::new(db, Arc::new(LocalSubprocessAdapter::new()));
    let resolved = helper
        .merge_base(None, "/tmp/demeteo_nonexistent_repo_xyz", "main", "feature")
        .await;
    assert!(resolved.is_none());
}

/// Small helper: `git rev-parse <ref>` in `dir`, trimmed to the bare SHA.
async fn rev_parse(exec: &LocalSubprocessAdapter, dir: &str, rev: &str) -> String {
    exec.run_command("local", &format!("git -C \"{dir}\" rev-parse {rev}"))
        .await
        .unwrap()
        .trim()
        .to_string()
}

/// The origin-sync fix (bundled with the bootstrap-progress work): a new
/// feature branch must be cut from the freshly-fetched `origin/<default>`,
/// NOT the local `<default>` ref — which lags because the main checkout sits
/// on it (git refuses to fast-forward a checked-out branch). Reproduces the
/// tail's ordering: fetch origin, then create the branch.
#[tokio::test]
async fn test_create_feature_branch_cuts_from_origin_not_stale_local() {
    let (local_dir, remote_dir, helper) = make_two_repos("branch_from_origin").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // Advance origin/main by one commit that the local clone has NOT fetched.
    // The local `main` ref (checked out) still points at the initial commit.
    exec.write_file("local", &format!("{remote}/README.md"), "origin advanced")
        .await
        .unwrap();
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" commit -am origin-advance"),
        )
        .await;

    let stale_local_main = rev_parse(&exec, &local, "main").await;

    // The tail runs this first (best-effort): it fetches origin/main to the
    // fresh commit. Its final local-ref fast-forward is *rejected* because
    // main is checked out — that Err is expected and ignored by the tail.
    let _ = helper
        .ensure_default_branch_updated(None, &local, "main")
        .await;

    helper
        .create_feature_branch(None, &local, "main", "feature/f-sync")
        .await
        .expect("create_feature_branch should succeed");

    let feature_sha = rev_parse(&exec, &local, "feature/f-sync").await;
    let origin_main_sha = rev_parse(&exec, &local, "origin/main").await;

    assert_eq!(
        feature_sha, origin_main_sha,
        "feature branch must be cut from the freshly-fetched origin/main"
    );
    assert_ne!(
        feature_sha, stale_local_main,
        "feature branch must NOT be cut from the stale local main ref"
    );

    let _ = std::fs::remove_dir_all(&local_dir);
    let _ = std::fs::remove_dir_all(&remote_dir);
}

/// Offline / no-remote fallback: with no `origin/<default>` to resolve,
/// `create_feature_branch` falls back to the local default branch and still
/// succeeds (the pre-fix behavior, preserved).
#[tokio::test]
async fn test_create_feature_branch_falls_back_to_local_without_origin() {
    let (dir, helper) = make_repo("branch_no_origin").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    let local_main = rev_parse(&exec, &repo, "main").await;

    helper
        .create_feature_branch(None, &repo, "main", "feature/f-local")
        .await
        .expect("create_feature_branch should fall back to local main");

    let feature_sha = rev_parse(&exec, &repo, "feature/f-local").await;
    assert_eq!(
        feature_sha, local_main,
        "with no origin, the feature branch is cut from local main"
    );

    let _ = std::fs::remove_dir_all(&dir);
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
    let (local_dir, remote_dir, helper) = make_two_repos("ff_on_feature").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // Move HEAD off `main` onto a feature branch — the state the user
    // was in when they reported the bug.
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/watching"),
        )
        .await;

    // Advance origin/main by one commit the local clone has NOT fetched.
    exec.write_file("local", &format!("{remote}/README.md"), "origin advanced")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" commit -am advance"))
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
    let (local_dir, remote_dir, helper) = make_two_repos("ff_clean_tree").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // HEAD stays on `main` (default) — the most common bootstrap state.

    // Advance origin/main by one commit the local clone has NOT fetched.
    exec.write_file("local", &format!("{remote}/README.md"), "origin advanced")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" commit -am advance"))
        .await;

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
    let (local_dir, remote_dir, helper) = make_two_repos("ff_dirty_tree").await;
    let local = local_dir.to_string_lossy().to_string();
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

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

    // Advance origin/main by one commit the local clone has NOT fetched.
    exec.write_file("local", &format!("{remote}/README.md"), "origin advanced")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{remote}\" commit -am advance"))
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

/// Regression: `branch_delete` must remove subtask worktrees BEFORE it
/// deletes their branches. `git branch -D` refuses to delete a branch
/// still checked out in a worktree, so if the branch delete runs first
/// the subtask ref survives (the delete failure was swallowed by
/// `2>/dev/null`). After the reorder, the worktree is gone by the time
/// the branch delete runs, so no `_subtask_*` ref is left behind.
#[tokio::test]
async fn test_branch_delete_removes_subtask_worktree_and_branch() {
    let (dir, helper) = make_repo("branch_delete_order").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    // A ref-only feature branch, as the pipeline creates it.
    let feature_branch = "feature/f-del";
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" branch {feature_branch}"),
        )
        .await;

    // Provision a real subtask worktree checked out on the subtask
    // branch — this is what makes the naive branch-delete-first order
    // fail.
    let wt_path = helper
        .provision_subtask_worktree(None, &repo, feature_branch, "sub-1")
        .await
        .unwrap();
    assert!(
        std::path::Path::new(&wt_path).exists(),
        "subtask worktree should exist before cleanup"
    );

    helper
        .branch_delete(None, &repo, feature_branch)
        .await
        .expect("branch_delete should succeed");

    // The worktree directory is gone.
    assert!(
        !std::path::Path::new(&wt_path).exists(),
        "subtask worktree dir should be removed by branch_delete"
    );

    // The regression assertion: NO subtask branch is left dangling.
    let branches = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" branch --list '{feature_branch}_subtask_*'"),
        )
        .await
        .unwrap();
    assert!(
        branches.trim().is_empty(),
        "no _subtask_* ref should survive branch_delete; got: {branches:?}"
    );

    // The feature branch itself is gone too.
    let feature = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" branch --list {feature_branch}"),
        )
        .await
        .unwrap();
    assert!(
        feature.trim().is_empty(),
        "feature branch should be deleted; got: {feature:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

/// Regression: a prior attempt interrupted mid-merge leaves `MERGE_HEAD`
/// set on the feature-branch checkout. On retry `merge_subtask` must clear
/// that stale in-progress merge instead of aborting with
/// "fatal: You have not concluded your merge (MERGE_HEAD exists)".
#[tokio::test]
async fn test_merge_subtask_recovers_from_stale_merge_head() {
    let (dir, helper) = make_repo("merge_stale_head").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

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
    let exec = LocalSubprocessAdapter::new();

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

// ─────────────────────────────────────────────────────────────────────────
// squash_feature_branch / validate_commit_message
// ─────────────────────────────────────────────────────────────────────────

/// Build a repo whose `feature/f-sq` branch is checked out and carries
/// `n` commits on top of `main`, each adding one file. Returns the repo path.
async fn make_repo_with_feature_commits(
    suffix: &str,
    n: usize,
) -> (std::path::PathBuf, GitOpsHelper, String) {
    let (dir, helper) = make_repo(suffix).await;
    let repo = dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();

    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" checkout -b feature/f-sq"),
        )
        .await;
    for i in 0..n {
        exec.write_file("local", &format!("{repo}/file{i}.txt"), &format!("v{i}"))
            .await
            .unwrap();
        let _ = exec
            .run_command("local", &format!("git -C \"{repo}\" add ."))
            .await;
        let _ = exec
            .run_command(
                "local",
                &format!("git -C \"{repo}\" commit --no-verify -m \"step {i} work\""),
            )
            .await;
    }
    (dir, helper, repo)
}

/// The core contract: N commits become 1, the content is byte-for-byte
/// unchanged, and the pre-squash tip stays reachable by name.
#[tokio::test]
async fn test_squash_feature_branch_collapses_history_preserving_tree() {
    let (dir, helper, repo) = make_repo_with_feature_commits("squash_collapse", 4).await;
    let exec = LocalSubprocessAdapter::new();

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
    let exec = LocalSubprocessAdapter::new();

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
    let exec = LocalSubprocessAdapter::new();

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
    let exec = LocalSubprocessAdapter::new();

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
