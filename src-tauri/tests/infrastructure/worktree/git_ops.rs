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
