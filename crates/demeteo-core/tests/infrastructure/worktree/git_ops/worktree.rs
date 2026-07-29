// Tests extracted from `crates/demeteo-core/src/adapters/worktree/git_ops/worktree.rs` (mirrored-tests convention). `super` = that module.

use super::super::common::*;
use super::link_dependency_caches_cmd;
use super::GitOpsHelper;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::paths::feature_cache_dir;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use rusqlite::Connection;
use std::sync::Arc;

#[test]
fn command_iterates_every_known_cache_dir() {
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1", "/repo_cache_feature-a");
    for dir in crate::paths::DEPENDENCY_CACHE_DIRS {
        assert!(
            cmd.contains(dir),
            "expected command to reference '{}': {}",
            dir,
            cmd
        );
    }
}

#[test]
fn command_gates_on_existence_and_check_ignore() {
    // These paths contain only shell-safe characters, so `shell_escape_posix`
    // leaves them bare (no quoting needed).
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1", "/repo_cache_feature-a");
    assert!(cmd.contains("check-ignore -q"));
    assert!(cmd.contains("[ -e /repo/\"$d\" ]"));
    assert!(cmd.contains("[ ! -e /repo_wt_1/\"$d\" ]"));
}

/// The worktree must symlink into *this feature's* cache root, never straight
/// into the primary checkout. Linking to `{repo}/node_modules` is what let one
/// feature's install overwrite another's — and, worse, let one feature's build
/// output decide another feature's harness verdict.
#[test]
fn worktree_links_into_the_feature_cache_not_the_shared_primary() {
    let cache = feature_cache_dir("/repo", "feature/login");
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1", &cache);

    assert!(
        cmd.contains("ln -sfn /repo_cache_feature-login/\"$d\" /repo_wt_1/\"$d\""),
        "worktree must link into the feature's own cache root: {cmd}"
    );
    assert!(
        !cmd.contains("ln -sfn /repo/\"$d\""),
        "worktree must NOT link straight at the shared primary checkout: {cmd}"
    );
}

/// Seeding must be a copy (ideally a copy-on-write clone), never a hardlink: a
/// tool that rewrites a file in place would write *through* a hardlink into
/// every other feature's tree, reintroducing the very bug this replaces.
#[test]
fn seeds_the_feature_cache_by_copy_preferring_copy_on_write() {
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1", "/repo_cache_feature-a");
    // APFS clonefile, then btrfs/xfs reflink, then a plain copy.
    assert!(cmd.contains("cp -cR"), "{cmd}");
    assert!(cmd.contains("--reflink=auto"), "{cmd}");
    assert!(
        !cmd.contains("cp -al") && !cmd.contains("ln /repo"),
        "seeding must never hardlink: {cmd}"
    );
    // Seed once — a feature's later steps reuse the cache they already have.
    assert!(
        cmd.contains("[ ! -e /repo_cache_feature-a/\"$d\" ]"),
        "{cmd}"
    );
}

#[test]
fn paths_with_special_chars_are_escaped() {
    let cmd = link_dependency_caches_cmd(
        "/repos/my repo",
        "/repos/my repo_wt_1",
        "/repos/my repo_cache_f",
    );
    assert!(cmd.contains("'/repos/my repo'"));
    assert!(cmd.contains("'/repos/my repo_wt_1'"));
    assert!(cmd.contains("'/repos/my repo_cache_f'"));
}

/// The cache root is keyed by feature branch, so two features on one repo can
/// never resolve to the same directory — that is the whole isolation property.
#[test]
fn feature_cache_dirs_are_distinct_per_feature_and_slash_free() {
    let a = feature_cache_dir("/repo", "feature/login");
    let b = feature_cache_dir("/repo", "feature/checkout");
    assert_ne!(a, b);
    assert_eq!(a, "/repo_cache_feature-login");
    // A raw `/` would nest the cache under a `feature/` directory instead of
    // sitting alongside the repo.
    assert!(!a.trim_start_matches("/repo").contains('/'), "{a}");
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
    let exec_tmp = fresh_exec();
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
    let exec = fresh_exec();
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

/// Regression: the `-b` fallback must not resurrect a failed attempt's work.
///
/// When a step fails, the driver resets the feature branch back to the tip it
/// started from and deletes the subtask branch. If that cleanup does not run
/// (a crash, a locked worktree), the subtask branch survives *pointing at the
/// abandoned commits*. The next attempt's `worktree add -b` then fails
/// (branch exists) and falls back to checking the stale branch out — so the
/// retry would build on the very work the rollback threw away and merge all of
/// it back, silently undoing the rollback.
///
/// Provisioning must hand back a worktree at the feature branch either way.
#[tokio::test]
async fn test_provision_subtask_worktree_fallback_discards_stale_branch_commits() {
    let (dir, helper) = make_repo("wt_fallback_stale").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let feature_tip = exec
        .run_command("local", &format!("git -C \"{repo}\" rev-parse main"))
        .await
        .unwrap()
        .trim()
        .to_string();

    // A previous attempt left its subtask branch behind, carrying a commit the
    // rollback was supposed to discard.
    for cmd in [
        format!("git -C \"{repo}\" branch main_subtask_sub-1"),
        format!("git -C \"{repo}\" worktree add --force \"{repo}_stale\" main_subtask_sub-1"),
        format!("echo abandoned > \"{repo}_stale/abandoned.txt\""),
        format!("git -C \"{repo}_stale\" add -A"),
        format!("git -C \"{repo}_stale\" commit -m abandoned"),
        format!("git -C \"{repo}\" worktree remove --force \"{repo}_stale\""),
    ] {
        exec.run_command("local", &cmd).await.unwrap();
    }

    let wt_path = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-1")
        .await
        .unwrap();

    let head = exec
        .run_command("local", &format!("git -C \"{wt_path}\" rev-parse HEAD"))
        .await
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        head, feature_tip,
        "the reused subtask branch must be reset to the feature branch, not left on the \
         abandoned attempt's tip"
    );
    assert!(
        !std::path::Path::new(&wt_path)
            .join("abandoned.txt")
            .exists(),
        "the failed attempt's file must not reappear in the retry's worktree"
    );

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
    let exec = fresh_exec();
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
    let exec = fresh_exec();

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
    let exec = fresh_exec();
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

/// Regression: two features running concurrently against the **same**
/// repo must not share a subtask worktree directory.
///
/// This is the case `test_provision_subtask_worktree_distinct_per_feature`
/// only asserts by hand-wave (it uses two separate repos, where a
/// collision is impossible by construction). The real deployment has many
/// features on one project — hence one `repo_dir` — and the parallel step
/// used to derive its worktree from the bare planner id (`sub-1`), so
/// every feature resolved to the *same* `{repo}_wt_sub-1`.
///
/// The damage is not a name clash but data loss: `provision_subtask_worktree`
/// opens by force-removing and `rm -rf`ing its target. A sibling feature
/// starting its implement step therefore deleted the live worktree of a
/// feature whose agent was still running — and since a worker's writes are
/// only committed later (phase B), everything it had written was gone. The
/// owning feature then failed with "cannot change to '<path>': No such file
/// or directory", the branch was rolled back, and `s-validate` correctly
/// reported the feature as unimplemented while `s-implement` had reported
/// success.
///
/// So this asserts the property that actually matters: provisioning for
/// feature B leaves feature A's uncommitted work untouched.
#[tokio::test]
async fn test_provision_subtask_worktree_same_repo_two_features_do_not_collide() {
    let (dir, helper) = make_repo("wt_same_repo_two_features").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // Both features run the same planner-assigned subtask id (`sub-1`) —
    // planner ids are per-feature and always start at `sub-1`, so this is
    // the common case, not an edge case. The call site scopes them by
    // feature id before they reach the provisioner.
    let wt_a = helper
        .provision_subtask_worktree(None, &repo, "main", "f-AAA-sub-1")
        .await
        .expect("feature A should provision");
    let wt_b = helper
        .provision_subtask_worktree(None, &repo, "main", "f-BBB-sub-1")
        .await
        .expect("feature B should provision");

    assert_ne!(
        wt_a, wt_b,
        "two features on the same repo must not share a worktree directory"
    );

    // Feature A's agent writes a file and, as in phase A, does NOT commit it.
    let a_work = format!("{wt_a}/feature_a_work.rs");
    std::fs::write(&a_work, "// feature A's uncommitted implementation").unwrap();

    // Feature B now re-provisions — the retry path, which force-removes and
    // `rm -rf`s its own worktree. Under the old unscoped naming this is the
    // call that destroyed feature A's work.
    helper
        .provision_subtask_worktree(None, &repo, "main", "f-BBB-sub-1")
        .await
        .expect("feature B should re-provision");

    assert!(
        std::path::Path::new(&a_work).exists(),
        "feature B's provisioning destroyed feature A's uncommitted work at {a_work} — \
         the worktree directories are colliding"
    );

    // Cleanup.
    for wt in [&wt_a, &wt_b] {
        let _ = exec
            .run_command(
                "local",
                &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
            )
            .await;
        let _ = std::fs::remove_dir_all(wt);
    }
    let _ = std::fs::remove_dir_all(&repo);
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
    let exec = fresh_exec();

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
    let exec = fresh_exec();
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

/// A project runs N features concurrently (decision 18), so two features on one
/// repo must never share a dependency cache.
///
/// They used to: every worktree symlinked straight at `{repo}/node_modules`, so
/// feature B's install silently overwrote feature A's. The real damage was not
/// the corrupted tree — it was that a `verify` step's harness verdict could be
/// decided by *another feature's* build output, and that verdict drives
/// Demeteo's retry and critic loops. A feature would chase a failure that was
/// never its own.
#[tokio::test]
async fn test_concurrent_features_do_not_share_a_dependency_cache() {
    let (dir, helper) = make_repo("wt_dep_isolation").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    // An already-installed primary checkout.
    std::fs::write(format!("{repo}/.gitignore"), "node_modules/\n").unwrap();
    std::fs::create_dir_all(format!("{repo}/node_modules")).unwrap();
    std::fs::write(format!("{repo}/node_modules/left-pad"), "v1.0.0").unwrap();
    for cmd in [
        format!("git -C \"{repo}\" add .gitignore"),
        format!("git -C \"{repo}\" commit -m gitignore"),
        format!("git -C \"{repo}\" branch feature/alpha"),
        format!("git -C \"{repo}\" branch feature/beta"),
    ] {
        exec.run_command("local", &cmd).await.unwrap();
    }

    // Two features, running at the same time on the same repo.
    let wt_a = helper
        .provision_subtask_worktree(None, &repo, "feature/alpha", "f-alpha-step-impl")
        .await
        .expect("alpha provision");
    let wt_b = helper
        .provision_subtask_worktree(None, &repo, "feature/beta", "f-beta-step-impl")
        .await
        .expect("beta provision");

    // Both were seeded from the primary install.
    assert_eq!(
        std::fs::read_to_string(format!("{wt_a}/node_modules/left-pad")).unwrap(),
        "v1.0.0"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{wt_b}/node_modules/left-pad")).unwrap(),
        "v1.0.0"
    );

    // Feature beta installs an incompatible version of a shared dep.
    std::fs::write(format!("{wt_b}/node_modules/left-pad"), "v2.0.0").unwrap();

    // Feature alpha must be untouched — this is the whole property.
    assert_eq!(
        std::fs::read_to_string(format!("{wt_a}/node_modules/left-pad")).unwrap(),
        "v1.0.0",
        "feature beta's install leaked into feature alpha's worktree"
    );
    // And neither may have corrupted the primary checkout they were seeded from.
    assert_eq!(
        std::fs::read_to_string(format!("{repo}/node_modules/left-pad")).unwrap(),
        "v1.0.0",
        "a feature's install leaked back into the primary checkout"
    );

    // Deleting a feature's branch reclaims its cache — otherwise every feature
    // ever run leaks a whole node_modules.
    let cache_b = crate::paths::feature_cache_dir(&repo, "feature/beta");
    assert!(std::path::Path::new(&cache_b).exists());
    helper
        .branch_delete(None, &repo, "feature/beta")
        .await
        .expect("branch delete");
    assert!(
        !std::path::Path::new(&cache_b).exists(),
        "feature beta's dependency cache leaked after its branch was deleted"
    );

    for wt in [&wt_a, &wt_b] {
        let _ = exec
            .run_command(
                "local",
                &format!("git -C \"{repo}\" worktree remove --force \"{wt}\""),
            )
            .await;
        let _ = std::fs::remove_dir_all(wt);
    }
    let _ = std::fs::remove_dir_all(crate::paths::feature_cache_dir(&repo, "feature/alpha"));
    let _ = std::fs::remove_dir_all(&dir);
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
    let exec = fresh_exec();
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
    let exec = fresh_exec();

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
    let helper = GitOpsHelper::new(db, Arc::new(fresh_exec()));
    let resolved = helper
        .merge_base(None, "/tmp/demeteo_nonexistent_repo_xyz", "main", "feature")
        .await;
    assert!(resolved.is_none());
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
    let exec = fresh_exec();

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
    let exec = fresh_exec();

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
    let exec = fresh_exec();

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

/// Regression: the artifact-scope fence (`apply_artifact_scope`) chmods
/// protected paths in a step's worktree to `a-w` before the agent's turn.
/// If the step is a Verify/Artifacts/ReadOnly step (e.g. validate, critic),
/// that fence is still in place when the step finishes and
/// `cleanup_subtask_worktree` runs — `unlink()` needs write on the parent
/// directory, so an `a-w src/` blocks both `git worktree remove --force`
/// and `rm -rf`. Every command in `cleanup_subtask_worktree` is
/// best-effort (`let _ = ...`), so the failure was previously swallowed,
/// leaving a gutted, git-disconnected directory skeleton on disk forever.
#[tokio::test]
async fn test_cleanup_subtask_worktree_handles_chmod_locked_worktree() {
    let (dir, helper) = make_repo("cleanup_chmod_locked").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let wt_path = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-cleanup-chmod")
        .await
        .expect("provision should succeed");
    assert!(std::path::Path::new(&wt_path).exists());

    // Mimic the artifact-scope fence left behind by a Verify/Artifacts/
    // ReadOnly step: every top-level entry chmod'd a-w recursively.
    let _ = exec
        .run_command("local", &format!("chmod -R a-w '{wt_path}'"))
        .await;

    helper
        .cleanup_subtask_worktree(None, &repo, "main", "sub-cleanup-chmod")
        .await
        .expect("cleanup should succeed even with a chmod-locked worktree");

    assert!(
        !std::path::Path::new(&wt_path).exists(),
        "chmod-locked worktree dir should be fully removed by cleanup"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

/// Same permission-fence hazard as
/// `test_cleanup_subtask_worktree_handles_chmod_locked_worktree`, but via
/// `branch_delete`'s worktree-removal loop — the path used when a whole
/// feature/branch is torn down, which iterates every `_subtask_*`
/// worktree and must clean up each one even if the fence chmod'd it a-w.
#[tokio::test]
async fn test_branch_delete_handles_chmod_locked_worktree() {
    let (dir, helper) = make_repo("branch_delete_chmod_locked").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    let feature_branch = "feature/f-chmod-del";
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" branch {feature_branch}"),
        )
        .await;

    let wt_path = helper
        .provision_subtask_worktree(None, &repo, feature_branch, "sub-1")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("chmod -R a-w '{wt_path}'"))
        .await;

    helper
        .branch_delete(None, &repo, feature_branch)
        .await
        .expect("branch_delete should succeed even with a chmod-locked subtask worktree");

    assert!(
        !std::path::Path::new(&wt_path).exists(),
        "chmod-locked subtask worktree dir should be removed by branch_delete"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&wt_path);
}

// ── Detached worktrees (HB2b) ────────────────────────────────────────────────
//
// The baseline fallback measures the *base* commit, which predates the feature
// entirely. `provision_subtask_worktree` cannot serve it: it takes a branch and
// creates one. These cover the primitive that can.

/// Commit a second time and return `(first_sha, second_sha)`.
async fn two_commits(repo: &str) -> (String, String) {
    let exec = fresh_exec();
    let sha = |out: String| out.trim().to_string();
    let first = sha(exec
        .run_command("local", &format!("git -C \"{repo}\" rev-parse HEAD"))
        .await
        .unwrap());
    exec.write_file("local", &format!("{repo}/second.txt"), "second")
        .await
        .unwrap();
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" add ."))
        .await;
    let _ = exec
        .run_command("local", &format!("git -C \"{repo}\" commit -m second"))
        .await;
    let second = sha(exec
        .run_command("local", &format!("git -C \"{repo}\" rev-parse HEAD"))
        .await
        .unwrap());
    (first, second)
}

/// The headline property: the worktree is checked out at the sha it was asked
/// for, not at the branch tip. A baseline measured at the tip would compare the
/// feature's work against itself.
#[tokio::test]
async fn a_detached_worktree_is_checked_out_at_the_requested_sha() {
    let (dir, helper) = make_repo("wt_detached_sha").await;
    let repo = dir.to_string_lossy().to_string();
    let (first, second) = two_commits(&repo).await;
    assert_ne!(first, second);

    let wt = helper
        .provision_detached_worktree(None, &repo, &first, "baseline", None)
        .await
        .expect("provisions");

    assert_eq!(
        helper.head_sha(None, &wt).await.as_deref(),
        Some(first.as_str()),
        "the worktree must sit at the base commit, not at the branch tip"
    );
    // The second commit's file must not be there — the point of measuring an
    // older commit is that it does not contain the newer work.
    assert!(!std::path::Path::new(&wt).join("second.txt").exists());

    let _ = helper
        .cleanup_detached_worktree(None, &repo, "baseline")
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// Detached is the safety property, not an implementation detail: a worktree
/// with no branch cannot be committed onto and cannot be merged back by
/// anything, so a measurement can never contaminate the feature.
#[tokio::test]
async fn a_detached_worktree_carries_no_branch() {
    let (dir, helper) = make_repo("wt_detached_nobranch").await;
    let repo = dir.to_string_lossy().to_string();
    let (first, _) = two_commits(&repo).await;

    helper
        .provision_detached_worktree(None, &repo, &first, "baseline", None)
        .await
        .expect("provisions");

    let listed = helper.list_worktrees(None, &repo).await.expect("lists");
    let entry = listed
        .iter()
        .find(|w| w.path.ends_with("_wt_baseline"))
        .expect("the detached worktree is registered");
    assert_eq!(
        entry.branch, None,
        "a baseline worktree must hold no branch: {entry:?}"
    );

    let _ = helper
        .cleanup_detached_worktree(None, &repo, "baseline")
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// Teardown must leave nothing behind — no directory and no registration. The
/// caller runs it on the failure path too, where the measurement itself came
/// back red, so anything left here accumulates once per failed validate.
#[tokio::test]
async fn cleanup_removes_the_directory_and_the_registration() {
    let (dir, helper) = make_repo("wt_detached_cleanup").await;
    let repo = dir.to_string_lossy().to_string();
    let (first, _) = two_commits(&repo).await;

    let wt = helper
        .provision_detached_worktree(None, &repo, &first, "baseline", None)
        .await
        .expect("provisions");
    assert!(std::path::Path::new(&wt).exists());

    helper
        .cleanup_detached_worktree(None, &repo, "baseline")
        .await
        .expect("cleans up");

    assert!(
        !std::path::Path::new(&wt).exists(),
        "the worktree directory must be gone"
    );
    let listed = helper.list_worktrees(None, &repo).await.expect("lists");
    assert!(
        !listed.iter().any(|w| w.path.ends_with("_wt_baseline")),
        "the registration must be pruned too, or the next `add` fails with \
         'already used by worktree at': {listed:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// An unresolvable sha must fail loudly rather than hand back a worktree at
/// whatever git felt like. The caller degrades to "no baseline" on `Err`; a
/// silent success would produce a record describing the wrong commit.
#[tokio::test]
async fn an_unresolvable_sha_is_an_error_not_a_silent_checkout() {
    let (dir, helper) = make_repo("wt_detached_badsha").await;
    let repo = dir.to_string_lossy().to_string();

    let err = helper
        .provision_detached_worktree(
            None,
            &repo,
            "0000000000000000000000000000000000000000",
            "baseline",
            None,
        )
        .await
        .expect_err("an unknown commit cannot be checked out");
    assert!(
        err.contains("provision_detached_worktree"),
        "the error must name the operation: {err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A leftover directory from a crashed run must not block the next
/// measurement — the same leftover-state discipline the subtask path carries.
#[tokio::test]
async fn an_orphan_directory_is_cleared_before_the_add() {
    let (dir, helper) = make_repo("wt_detached_orphan").await;
    let repo = dir.to_string_lossy().to_string();
    let (first, _) = two_commits(&repo).await;

    let orphan = format!("{repo}_wt_baseline");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(format!("{orphan}/debris.txt"), "left over").unwrap();

    let wt = helper
        .provision_detached_worktree(None, &repo, &first, "baseline", None)
        .await
        .expect("an orphan directory must not block provisioning");
    assert!(!std::path::Path::new(&wt).join("debris.txt").exists());

    let _ = helper
        .cleanup_detached_worktree(None, &repo, "baseline")
        .await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// `head_sha` answers about the *commit*, where `get_head_branch` answers about
/// the ref — and a detached worktree has no ref to answer with.
#[tokio::test]
async fn head_sha_reports_nothing_for_a_directory_that_is_not_a_repo() {
    let (dir, helper) = make_repo("wt_head_sha_missing").await;
    assert_eq!(helper.head_sha(None, "/nonexistent/path/xyz").await, None);
    let _ = std::fs::remove_dir_all(&dir);
}
