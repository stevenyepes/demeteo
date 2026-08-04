// Tests extracted from `crates/demeteo-core/src/adapters/worktree/git_ops/worktree.rs` (mirrored-tests convention). `super` = that module.

use super::super::common::*;
use super::GitOpsHelper;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::paths::feature_cache_dir;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::{TerminalWorktreeRequest, WorktreeOpsPort};
use rusqlite::Connection;
use std::sync::Arc;

/// A creation request that names no base, so the start point stays the primary
/// checkout's HEAD. Tests about *where a branch is cut* set `base_branch`
/// themselves; every other test is about the destination and should not have to
/// stand up a remote to reach it.
fn terminal_request(branch: &str, worktree_name: &str) -> TerminalWorktreeRequest {
    TerminalWorktreeRequest {
        branch: branch.to_string(),
        base_branch: None,
        worktree_name: worktree_name.to_string(),
    }
}

/// A repository sitting where a bootstrapped project puts it —
/// `<project_root>/repos/<name>` — so the terminal area these tests exercise is
/// derived from a real project root rather than an invented one.
async fn make_project_repo(suffix: &str) -> (std::path::PathBuf, String, GitOpsHelper) {
    let (checkout, helper) = make_repo(suffix).await;
    let project_root = checkout.with_extension("project");
    let repos = project_root.join(crate::paths::REPOS_SUBDIR);
    std::fs::create_dir_all(&repos).expect("creates the project repos directory");
    let repo_dir = repos.join(checkout.file_name().expect("temporary repo has a name"));
    std::fs::rename(&checkout, &repo_dir).expect("moves the checkout into the project layout");
    (project_root, repo_dir.to_string_lossy().to_string(), helper)
}

/// The area for `repo_dir` under `project_root`, spelled out here rather than
/// reusing the production helper so a wrong relocation cannot pass by agreeing
/// with itself.
fn expected_area(project_root: &std::path::Path, repo_dir: &str) -> std::path::PathBuf {
    project_root
        .join(crate::paths::TERMINAL_WORKTREES_SUBDIR)
        .join(
            std::path::Path::new(repo_dir)
                .file_name()
                .expect("repository has a name"),
        )
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
async fn terminal_worktree_creation_returns_linked_worktree_metadata() {
    let (project_root, repo, helper) = make_project_repo("terminal_worktree_create").await;

    let created = WorktreeOpsPort::create_terminal_worktree(
        &helper,
        None,
        &repo,
        &project_root.to_string_lossy(),
        &terminal_request("terminal/session", "session-one"),
    )
    .await
    .expect("creates a terminal worktree");

    assert_eq!(created.worktree.branch.as_deref(), Some("terminal/session"));
    assert!(!created.worktree.is_locked);
    assert_eq!(
        std::path::Path::new(&created.worktree.path)
            .canonicalize()
            .unwrap(),
        expected_area(&project_root, &repo)
            .join("session-one")
            .canonicalize()
            .unwrap()
    );
    assert!(std::path::Path::new(&created.worktree.path).exists());
    // What `create` returns must be what `list` replays, or nothing can match
    // a freshly created worktree against the listing it appears in.
    assert_eq!(
        helper
            .list_worktrees(None, &repo)
            .await
            .unwrap()
            .iter()
            .map(|worktree| worktree.path.as_str())
            .collect::<Vec<_>>(),
        [created.worktree.path.as_str()]
    );
    // The bootstrap prune sweeps `repos/` by configured repository name, so a
    // terminal worktree anywhere below it is deleted on the next re-bootstrap.
    assert!(
        !std::path::Path::new(&created.worktree.path)
            .starts_with(project_root.join(crate::paths::REPOS_SUBDIR)),
        "the terminal area must not live under the pruned repos/ directory: {}",
        created.worktree.path
    );

    let exec = fresh_exec();
    let _ = exec
        .run_command(
            "local",
            &format!(
                "git -C \"{repo}\" worktree remove --force \"{}\"",
                created.worktree.path
            ),
        )
        .await;
    let _ = std::fs::remove_dir_all(&project_root);
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_worktree_rejects_a_symlinked_area_or_nested_parent() {
    use std::os::unix::fs::symlink;

    let (project_root, repo, helper) = make_project_repo("terminal_worktree_symlink").await;
    let root = project_root.to_string_lossy().to_string();
    let area_root = project_root.join(crate::paths::TERMINAL_WORKTREES_SUBDIR);
    let area = expected_area(&project_root, &repo);
    let outside = project_root.join("demeteo_terminal_worktree_outside");
    std::fs::create_dir(&outside).expect("creates outside directory");

    symlink(&outside, &area_root).expect("creates area-root escape link");
    let area_root_error = helper
        .create_terminal_worktree(
            None,
            &repo,
            &root,
            &terminal_request("terminal/area-link", "session"),
        )
        .await
        .expect_err("a symlinked worktree area root must be rejected");
    assert!(area_root_error.contains("symlink"), "{area_root_error}");
    std::fs::remove_file(&area_root).expect("removes area-root link");

    std::fs::create_dir_all(&area).expect("creates controlled area");
    symlink(&outside, area.join("nested")).expect("creates nested escape link");
    let nested_error = helper
        .create_terminal_worktree(
            None,
            &repo,
            &root,
            &terminal_request("terminal/nested-link", "nested/session"),
        )
        .await
        .expect_err("a symlinked nested parent must be rejected");
    assert!(nested_error.contains("symlink"), "{nested_error}");
    assert!(
        !outside.join("session").exists(),
        "Git must not follow either escape link"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

#[tokio::test]
async fn terminal_worktree_branch_starts_at_the_current_primary_checkout_head() {
    let (project_root, repo, helper) = make_project_repo("terminal_worktree_start_point").await;
    let exec = fresh_exec();

    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" checkout -b source/branch"),
    )
    .await
    .expect("creates source branch");
    exec.write_file("local", &format!("{repo}/source.txt"), "source commit")
        .await
        .expect("writes source commit");
    exec.run_command("local", &format!("git -C \"{repo}\" add source.txt"))
        .await
        .expect("stages source commit");
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" commit -m source-commit"),
    )
    .await
    .expect("commits source branch");
    let primary_head = rev_parse(&exec, &repo, "HEAD").await;

    let created = helper
        .create_terminal_worktree(
            None,
            &repo,
            &project_root.to_string_lossy(),
            &terminal_request("terminal/from-primary-head", "session"),
        )
        .await
        .expect("creates linked worktree from the primary checkout head");

    assert_eq!(
        rev_parse(&exec, &repo, "refs/heads/terminal/from-primary-head").await,
        primary_head,
        "-b must create the terminal branch at the current primary-checkout HEAD"
    );
    assert_eq!(
        helper.get_head_branch(None, &repo).await.as_deref(),
        Some("source/branch"),
        "terminal creation must not move the primary checkout"
    );

    let _ = exec
        .run_command(
            "local",
            &format!(
                "git -C \"{repo}\" worktree remove --force \"{}\"",
                created.worktree.path
            ),
        )
        .await;
    let _ = std::fs::remove_dir_all(&project_root);
}

#[tokio::test]
async fn terminal_worktree_rejects_an_existing_branch_without_reusing_it() {
    let (project_root, repo, helper) = make_project_repo("terminal_worktree_existing_branch").await;
    let exec = fresh_exec();
    exec.run_command(
        "local",
        &format!("git -C \"{repo}\" branch terminal/already-exists"),
    )
    .await
    .expect("creates pre-existing branch");
    let existing_branch_head = rev_parse(&exec, &repo, "refs/heads/terminal/already-exists").await;
    let destination = expected_area(&project_root, &repo).join("new-session");

    let error = helper
        .create_terminal_worktree(
            None,
            &repo,
            &project_root.to_string_lossy(),
            &terminal_request("terminal/already-exists", "new-session"),
        )
        .await
        .expect_err("an existing branch must not be reused");

    assert!(error.contains("git worktree add"), "{error}");
    assert_eq!(
        rev_parse(&exec, &repo, "refs/heads/terminal/already-exists").await,
        existing_branch_head,
        "creation must not reset or otherwise change the existing branch"
    );
    assert!(
        !destination.exists(),
        "a failed existing-branch create must not leave a destination to reuse"
    );
    let _ = std::fs::remove_dir_all(&project_root);
}

#[tokio::test]
async fn terminal_worktree_rejects_unsafe_branch_and_location_inputs() {
    let (project_root, repo, helper) = make_project_repo("terminal_worktree_invalid").await;
    let root = project_root.to_string_lossy().to_string();

    for (branch, name) in [
        ("bad..branch", "session"),
        // Git would accept this. The terminal listing withholds any branch
        // carrying the pipeline's infix, so accepting it here would hand back a
        // worktree that never appears again.
        ("terminal/notes_subtask_1", "session"),
        ("terminal/session", ""),
        ("terminal/session", "/outside"),
        ("terminal/session", "C:\\outside"),
        ("terminal/session", "../outside"),
        ("terminal/session", "nested/../outside"),
    ] {
        let error = helper
            .create_terminal_worktree(None, &repo, &root, &terminal_request(branch, name))
            .await
            .expect_err("unsafe terminal worktree input must be rejected");
        assert!(error.contains("create_terminal_worktree"), "{error}");
    }
    assert!(
        helper.list_worktrees(None, &repo).await.unwrap().is_empty(),
        "invalid input must not create a worktree"
    );
    let _ = std::fs::remove_dir_all(&project_root);
}

/// Builds `<base>/project/repos/app` as a one-commit repository, so a test can
/// choose the path the project root sits on rather than take the temp-dir one
/// [`make_project_repo`] picks.
async fn make_project_repo_at(base: &std::path::Path) -> (String, String, GitOpsHelper) {
    let project_root = base.join("project");
    let repo_dir = project_root.join(crate::paths::REPOS_SUBDIR).join("app");
    std::fs::create_dir_all(&repo_dir).expect("creates the project layout");
    let repo = repo_dir.to_string_lossy().to_string();

    let exec = fresh_exec();
    for command in [
        format!("git -C \"{repo}\" init -q -b main"),
        format!("git -C \"{repo}\" config user.email ci@demeteo.test"),
        format!("git -C \"{repo}\" config user.name CI"),
        format!("git -C \"{repo}\" commit -q --allow-empty -m init"),
    ] {
        exec.run_command("local", &command)
            .await
            .unwrap_or_else(|e| panic!("setup failed: {command}: {e}"));
    }

    let conn = Connection::open_in_memory().expect("opens database");
    let db = Arc::new(SqliteAdapter::new(conn).expect("creates database"))
        as Arc<dyn AppSettingsRepository>;
    (
        project_root.to_string_lossy().to_string(),
        repo,
        GitOpsHelper::new(db, Arc::new(LocalSubprocessAdapter::new())),
    )
}

fn scratch(suffix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "demeteo_test_{suffix}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after the Unix epoch")
            .as_millis()
    ))
}

#[tokio::test]
async fn a_workspace_under_a_terminal_worktrees_directory_still_withholds_a_running_steps_checkout()
{
    // The reported bug end to end: every path below carries a
    // `terminal-worktrees` component, which is all the rule this replaced
    // looked for.
    let base =
        scratch("terminal_listing_named_ancestor").join(crate::paths::TERMINAL_WORKTREES_SUBDIR);
    let (project_root, repo, helper) = make_project_repo_at(&base).await;

    fresh_exec()
        .run_command("local", &format!("git -C \"{repo}\" branch feature/one"))
        .await
        .expect("creates the feature branch a subtask is cut from");
    helper
        .provision_subtask_worktree(None, &repo, "feature/one", "s-1")
        .await
        .expect("provisions a pipeline-owned worktree");
    // A linked worktree outside the area carrying an ordinary branch. The
    // subtask one above would be refused by the branch guard alone, so without
    // this entry the assertion below would hold with the anchor removed.
    fresh_exec()
        .run_command(
            "local",
            &format!(
                "git -C \"{repo}\" worktree add -q -b plain/elsewhere \"{}\"",
                base.join("outside-the-area").to_string_lossy()
            ),
        )
        .await
        .expect("adds an out-of-area worktree");
    let mine = helper
        .create_terminal_worktree(
            None,
            &repo,
            &project_root,
            &terminal_request("terminal/mine", "mine"),
        )
        .await
        .expect("creates a terminal worktree");

    let listed = helper
        .list_terminal_worktrees(None, &repo, &project_root)
        .await
        .expect("lists terminal locations");

    assert_eq!(
        listed.iter().map(|w| w.path.as_str()).collect::<Vec<_>>(),
        [mine.worktree.path.as_str()],
        "a checkout a running step owns is force-removed under whoever opened a shell in it"
    );
    assert_eq!(
        helper.list_worktrees(None, &repo).await.unwrap().len(),
        3,
        "all three worktrees exist; only one of them is a terminal location"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[tokio::test]
async fn a_terminal_worktree_is_listed_when_the_project_root_is_reached_through_a_symlink() {
    use std::os::unix::fs::symlink;

    let base = scratch("terminal_listing_symlink");
    let physical = base.join("real");
    let logical = base.join("link");
    std::fs::create_dir_all(&physical).expect("creates the physical base");
    symlink(&physical, &logical).expect("creates the logical base");

    let (_, _, helper) = make_project_repo_at(&physical).await;
    // Drive the port entirely through the logical spelling, as configuration
    // would: git resolves it away, so nothing derived from these strings can be
    // compared against what git reports back.
    let logical_root = logical.join("project").to_string_lossy().to_string();
    let logical_repo = format!("{logical_root}/{}/app", crate::paths::REPOS_SUBDIR);

    let created = helper
        .create_terminal_worktree(
            None,
            &logical_repo,
            &logical_root,
            &terminal_request("terminal/linked", "one"),
        )
        .await
        .expect("creates through the symlinked root");

    let listed = helper
        .list_terminal_worktrees(None, &logical_repo, &logical_root)
        .await
        .expect("lists through the symlinked root");

    assert_eq!(
        listed.iter().map(|w| w.path.as_str()).collect::<Vec<_>>(),
        [created.worktree.path.as_str()],
        "the area must be anchored on what git resolved, not on the configured root"
    );
    // The premise: git resolved the link away, so nothing built from the
    // logical strings above can be compared against what it reports back.
    assert!(
        created.worktree.path.contains("/real/") && !created.worktree.path.contains("/link/"),
        "git must report the physical path, or this test proves nothing: {}",
        created.worktree.path
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn a_repository_outside_its_project_root_is_an_error_not_an_empty_listing() {
    let base = scratch("terminal_listing_unanchored");
    let (_, repo, helper) = make_project_repo_at(&base).await;

    let error = helper
        .list_terminal_worktrees(None, &repo, "/somewhere/else")
        .await
        .expect_err("an underivable area must not read as a healthy empty listing");

    assert!(error.contains("no terminal area"), "{error}");
    let _ = std::fs::remove_dir_all(&base);
}

#[tokio::test]
async fn legacy_terminal_worktrees_are_unregistered_and_current_ones_left_alone() {
    let (project_root, repo, helper) = make_project_repo("terminal_worktree_legacy").await;
    let root = project_root.to_string_lossy().to_string();
    let exec = fresh_exec();
    let kept = helper
        .create_terminal_worktree(
            None,
            &repo,
            &root,
            &terminal_request("terminal/kept", "kept"),
        )
        .await
        .expect("creates a current-location worktree");

    // The old location, built the way the abandoned code built it: a hidden
    // sibling of the checkout inside `repos/`.
    let legacy_area = std::path::Path::new(&repo)
        .parent()
        .expect("the checkout has a parent")
        .join(format!(
            ".{}.demeteo-terminal-worktrees",
            std::path::Path::new(&repo)
                .file_name()
                .expect("the checkout has a name")
                .to_string_lossy()
        ));
    let legacy = legacy_area.join("stale");
    std::fs::create_dir_all(&legacy_area).expect("creates the legacy area");
    exec.run_command(
        "local",
        &format!(
            "git -C \"{repo}\" worktree add -b terminal/stale \"{}\"",
            legacy.to_string_lossy()
        ),
    )
    .await
    .expect("registers a worktree at the legacy location");

    let removed = helper
        .cleanup_legacy_terminal_worktrees(None, &repo)
        .await
        .expect("cleans the legacy location");

    assert_eq!(removed, 1, "the one legacy worktree must be reported");
    assert!(
        !legacy_area.exists(),
        "the legacy area itself must be gone, not just its registration"
    );
    let surviving = helper.list_worktrees(None, &repo).await.unwrap();
    assert_eq!(
        surviving
            .iter()
            .map(|worktree| worktree.path.as_str())
            .collect::<Vec<_>>(),
        [kept.worktree.path.as_str()],
        "Git must forget the legacy worktree and keep the current-location one"
    );

    let _ = exec
        .run_command(
            "local",
            &format!(
                "git -C \"{repo}\" worktree remove --force \"{}\"",
                kept.worktree.path
            ),
        )
        .await;
    let _ = std::fs::remove_dir_all(&project_root);
}

#[tokio::test]
async fn terminal_worktree_collision_is_reported_without_reusing_the_worktree() {
    let (project_root, repo, helper) = make_project_repo("terminal_worktree_collision").await;
    let root = project_root.to_string_lossy().to_string();
    let created = helper
        .create_terminal_worktree(
            None,
            &repo,
            &root,
            &terminal_request("terminal/first", "shared"),
        )
        .await
        .expect("creates initial worktree");

    let error = helper
        .create_terminal_worktree(
            None,
            &repo,
            &root,
            &terminal_request("terminal/second", "shared"),
        )
        .await
        .expect_err("an occupied destination must not be reused");
    assert!(error.contains("destination already exists"), "{error}");
    assert_eq!(
        helper.list_worktrees(None, &repo).await.unwrap().len(),
        1,
        "collision handling must neither remove nor create worktrees"
    );

    let exec = fresh_exec();
    let _ = exec
        .run_command(
            "local",
            &format!(
                "git -C \"{repo}\" worktree remove --force \"{}\"",
                created.worktree.path
            ),
        )
        .await;
    let _ = std::fs::remove_dir_all(&project_root);
}

/// A project whose clone is absent — never bootstrapped, or a workspace wiped
/// under it — reaches every terminal operation as a bare `git` complaint about
/// a directory. The picker shows that string and nothing else, so the port has
/// to name the missing clone and the way back.
///
/// Asserted through [`WorktreeOpsPort`] rather than the inherent methods: that
/// is the surface the application calls, and it is where the restatement lives.
#[tokio::test]
async fn a_missing_clone_is_named_as_such_by_every_terminal_operation() {
    let project_root = std::env::temp_dir().join(format!(
        "demeteo_terminal_missing_clone_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after the Unix epoch")
            .as_millis()
    ));
    let repo = project_root
        .join(crate::paths::REPOS_SUBDIR)
        .join("never-cloned")
        .to_string_lossy()
        .to_string();
    let root = project_root.to_string_lossy().to_string();
    let conn = Connection::open_in_memory().expect("opens database");
    let db = Arc::new(SqliteAdapter::new(conn).expect("creates database"))
        as Arc<dyn AppSettingsRepository>;
    let port: Arc<dyn WorktreeOpsPort> = Arc::new(GitOpsHelper::new(
        db,
        Arc::new(LocalSubprocessAdapter::new()),
    ));

    // A chosen base is the case Git explains worst: the start-point probe fails
    // against the missing directory and reports the base as the thing at fault.
    let create = port
        .create_terminal_worktree(
            None,
            &repo,
            &root,
            &TerminalWorktreeRequest {
                branch: "terminal/session".to_string(),
                base_branch: Some("main".to_string()),
                worktree_name: "session".to_string(),
            },
        )
        .await
        .expect_err("a missing clone cannot produce a worktree");
    let list = port
        .list_terminal_worktrees(None, &repo, &root)
        .await
        .expect_err("a missing clone has no listing");
    let branches = port
        .list_terminal_branches(None, &repo)
        .await
        .expect_err("a missing clone has no branches");
    let remove = port
        .remove_terminal_worktree(None, &repo, &root, &format!("{repo}-session"), false)
        .await
        .expect_err("a missing clone owns no worktree to remove");

    for error in [&create, &list, &branches, &remove] {
        assert!(
            error.contains(&repo) && error.contains("has not been cloned"),
            "the failure must name the missing clone: {error}"
        );
        assert!(
            error.contains("Re-run Bootstrap"),
            "the failure must name the way back: {error}"
        );
    }
    // The original wording is retained, but behind the restatement rather than
    // as the whole message: a base branch that "exists neither on origin nor
    // locally" is what a reader acts on first if it leads.
    assert!(
        create.starts_with(&repo),
        "the base branch must not lead the report of an absent repository: {create}"
    );

    let _ = std::fs::remove_dir_all(&project_root);
}

#[tokio::test]
async fn terminal_worktree_propagates_git_failures_without_cleanup() {
    let temp = std::env::temp_dir().join(format!(
        "demeteo_terminal_worktree_failure_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after the Unix epoch")
            .as_millis()
    ));
    let non_repo = temp
        .join(crate::paths::REPOS_SUBDIR)
        .join("not-a-repository");
    std::fs::create_dir_all(&non_repo).expect("creates non-repository directory");
    let conn = Connection::open_in_memory().expect("opens database");
    let db = Arc::new(SqliteAdapter::new(conn).expect("creates database"))
        as Arc<dyn AppSettingsRepository>;
    let helper = GitOpsHelper::new(db, Arc::new(LocalSubprocessAdapter::new()));

    let error = helper
        .create_terminal_worktree(
            None,
            &non_repo.to_string_lossy(),
            &temp.to_string_lossy(),
            &terminal_request("terminal/failure", "session"),
        )
        .await
        .expect_err("git failures must be surfaced");
    assert!(error.contains("git worktree add"), "{error}");
    assert!(
        !expected_area(&temp, &non_repo.to_string_lossy())
            .join("session")
            .exists(),
        "a failed add must not leave a reused or cleaned-up destination"
    );
    let _ = std::fs::remove_dir_all(&temp);
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

    // Pre-create the exact path provision_subtask_worktree would use, as an
    // orphan dir NOT registered with git. Derived rather than spelled out: the
    // segment is shortened on Windows, and what that segment *is* is pinned by
    // `shortening_touches_the_worktree_suffix_and_nothing_else`.
    let wt_path = worktree_dir_on(&repo, "sub-orphan", "local");
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
    let wt_path = worktree_dir_on(&repo, "sub-reg", "local");
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
    let wt_path = worktree_dir_on(&repo, "sub-chmod", "local");
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
// Cache sharing is a symlink, and Demeteo makes none on Windows — `ln -s`
// under Git for Windows needs a privilege a desktop user usually lacks and
// silently copies otherwise, and a junction carries a reparse tag git follows
// transparently. `share_dependency_caches` records the stated gap; these three
// cover the mechanism that only exists where the link does.
#[cfg(unix)]
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
#[cfg(unix)]
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
#[cfg(unix)]
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
        .find(|w| w.path.ends_with(baseline_suffix(&repo).as_str()))
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
        !listed
            .iter()
            .any(|w| w.path.ends_with(baseline_suffix(&repo).as_str())),
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

    let orphan = worktree_dir_on(&repo, "baseline", "local");
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

// ── Teardown, path budget, and cache exclusion ───────────────────────────────
//
// The ordering tests drive the free functions directly against a double that
// records every verb in one ordered list. Order is the whole content of
// `reclaim_worktree_path`, and no test that only inspects the filesystem
// afterwards can see it: a wrong order still ends with an empty directory on
// Linux, and only fails on the platform none of these tests run on.

use super::{
    delete_worktree_residue, exclude_file_with, link_dependency_caches_cmd, reclaim_worktree_path,
    restore_write_access_cmd, shareable_cache_names, shareable_cache_probe_cmd, target_path_join,
    worktree_dir, worktree_dir_on,
};
use crate::ports::execution::{ProgramRequest, SftpEntry};
use std::sync::Mutex;

/// Records every port call in one ordered list and answers only the two
/// questions the teardown asks, so a step that visits the wrong verb — or the
/// right ones in the wrong order — fails rather than passes quietly.
struct TeardownExec {
    log: Mutex<Vec<String>>,
    /// What `remove_dir_all` reports.
    removed: bool,
    /// Whether the path is still there when the teardown asks afterwards.
    residue: bool,
    /// Whether `git worktree remove` refuses — a locked worktree.
    git_remove_fails: bool,
}

impl TeardownExec {
    fn new(removed: bool, residue: bool) -> Self {
        Self {
            log: Mutex::new(Vec::new()),
            removed,
            residue,
            git_remove_fails: false,
        }
    }
    fn with_locked_worktree() -> Self {
        Self {
            git_remove_fails: true,
            ..Self::new(true, false)
        }
    }
    fn seen(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
    fn note(&self, line: String) {
        self.log.lock().unwrap().push(line);
    }
}

#[async_trait::async_trait]
impl ExecutionPort for TeardownExec {
    async fn test_connection(&self, _m: &str) -> Result<(), String> {
        Err("unscripted test_connection".into())
    }
    async fn run_program(&self, _m: &str, request: ProgramRequest) -> Result<String, String> {
        let line = format!("{} {}", request.executable, request.args.join(" "));
        self.note(line.clone());
        if self.git_remove_fails && line.contains("worktree remove") {
            return Err("fatal: cannot remove a locked working tree".into());
        }
        Ok(String::new())
    }
    async fn run_command(&self, _m: &str, cmd: &str) -> Result<String, String> {
        self.note(format!("sh: {cmd}"));
        Ok(String::new())
    }
    async fn remove_dir_all(&self, _m: &str, path: &str) -> Result<(), String> {
        self.note(format!("remove_dir_all {path}"));
        if self.removed {
            Ok(())
        } else {
            Err(format!("Failed to remove directory '{path}': busy"))
        }
    }
    async fn get_metadata(&self, _m: &str, path: &str) -> Result<SftpEntry, String> {
        self.note(format!("get_metadata {path}"));
        if self.residue {
            Ok(SftpEntry {
                name: path.to_string(),
                path: path.to_string(),
                is_dir: true,
                size: 0,
                modified: 0,
            })
        } else {
            Err("No such file or directory".into())
        }
    }
    async fn read_file(&self, _m: &str, _p: &str) -> Result<String, String> {
        Err("unscripted read_file".into())
    }
    async fn write_file(&self, _m: &str, _p: &str, _c: &str) -> Result<(), String> {
        Err("unscripted write_file".into())
    }
    async fn write_file_bytes(&self, _m: &str, _p: &str, _c: &[u8]) -> Result<(), String> {
        Err("unscripted write_file_bytes".into())
    }
    async fn list_dir(&self, _m: &str, _p: &str) -> Result<Vec<SftpEntry>, String> {
        Err("unscripted list_dir".into())
    }
    async fn setup_worktree(&self, _m: &str, _r: &str, _b: &str, _s: &str) -> Result<(), String> {
        Err("unscripted setup_worktree".into())
    }
    async fn resolve_home(&self, _m: &str) -> Result<String, String> {
        Err("unscripted resolve_home".into())
    }
    async fn resolve_user(&self, _m: &str) -> Result<String, String> {
        Err("unscripted resolve_user".into())
    }
    async fn control_rpc(
        &self,
        _m: &str,
        _method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("unscripted control_rpc".into())
    }
    fn spawn_interactive(
        &self,
        _m: &str,
        _b: &str,
        _a: &[String],
        _c: &str,
        _e: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("unscripted spawn_interactive".into())
    }
}

/// Write access is restored before git is asked to remove anything, the removal
/// is doubly forced, the prune follows it, and only then is the residue
/// deleted. Every one of those four is there because the step before it would
/// otherwise fail.
#[tokio::test]
async fn teardown_restores_write_access_before_git_and_prunes_before_deleting() {
    let exec = TeardownExec::new(true, false);
    reclaim_worktree_path(&exec, "local", "/repo", "/repo_wt_a")
        .await
        .expect("a removable worktree is torn down");

    assert_eq!(
        exec.seen(),
        vec![
            "sh: chmod -R u+w /repo_wt_a 2>/dev/null || true".to_string(),
            "git -C /repo worktree remove --force --force /repo_wt_a".to_string(),
            "git -C /repo worktree prune".to_string(),
            "remove_dir_all /repo_wt_a".to_string(),
        ]
    );
}

/// A single `-f` **refuses a locked worktree**, and a lock is what a crashed or
/// killed step leaves behind — so the doubled force is the difference between
/// teardown working and teardown failing exactly when it is needed.
#[tokio::test]
async fn the_worktree_removal_is_forced_twice() {
    let exec = TeardownExec::new(true, false);
    let _ = reclaim_worktree_path(&exec, "local", "/repo", "/repo_wt_a").await;

    let removal = exec
        .seen()
        .into_iter()
        .find(|line| line.contains("worktree remove"))
        .expect("the teardown asks git to remove the worktree");
    assert_eq!(
        removal.matches("--force").count(),
        2,
        "a locked worktree needs `--force --force`: {removal}"
    );
}

/// The prune runs even when the removal failed — that is precisely when there
/// is an administrative entry left to prune, and an entry left behind fails the
/// next `add` of the same destination with "already used by worktree".
#[tokio::test]
async fn the_prune_still_runs_when_the_directory_cannot_be_deleted() {
    let exec = TeardownExec::new(false, true);
    let error = reclaim_worktree_path(&exec, "local", "/repo", "/repo_wt_a")
        .await
        .expect_err("a surviving directory is a failure");

    assert!(exec
        .seen()
        .iter()
        .any(|line| line.ends_with("worktree prune")));
    assert!(
        error.starts_with("/repo_wt_a could not be deleted"),
        "the message names the path first, for the cleanup queue: {error}"
    );
}

/// The prune is what makes the *name* reusable, and a `remove` that refused is
/// exactly when there is an administrative entry left to prune — so it cannot
/// be conditional on that removal, and the delete after it cannot be either.
#[tokio::test]
async fn a_refused_removal_is_still_pruned_and_still_deleted() {
    let exec = TeardownExec::with_locked_worktree();
    reclaim_worktree_path(&exec, "local", "/repo", "/repo_wt_a")
        .await
        .expect("git refusing to deregister does not stop the teardown");

    let seen = exec.seen();
    assert!(seen.iter().any(|line| line.ends_with("worktree prune")));
    assert!(seen.iter().any(|line| line == "remove_dir_all /repo_wt_a"));
}

/// `ExecutionPort::remove_dir_all` reports an absent path as an error, and a
/// teardown reaches it with the directory already gone often enough that the
/// error alone cannot be the verdict. The path is asked about directly instead
/// of the message being matched on.
#[tokio::test]
async fn a_directory_that_is_already_gone_is_not_a_teardown_failure() {
    let exec = TeardownExec::new(false, false);
    delete_worktree_residue(&exec, "local", "/repo_wt_a")
        .await
        .expect("an absent directory is the outcome teardown wanted");
    assert_eq!(
        exec.seen(),
        vec![
            "remove_dir_all /repo_wt_a".to_string(),
            "get_metadata /repo_wt_a".to_string(),
        ]
    );
}

/// A successful delete must not spend a round trip proving it.
#[tokio::test]
async fn a_successful_delete_asks_nothing_further() {
    let exec = TeardownExec::new(true, true);
    delete_worktree_residue(&exec, "local", "/repo_wt_a")
        .await
        .unwrap();
    assert_eq!(exec.seen(), vec!["remove_dir_all /repo_wt_a".to_string()]);
}

/// The Unix write-restore is the inverse of the Unix `chmod a-w` fence. On a
/// Windows-local target the fence is an ACL that `restore_artifact_scope` has
/// already lifted, so the POSIX `chmod` would walk the tree to change nothing —
/// while a Windows desktop driving a **remote** machine still faces a real
/// `chmod a-w` fence there and must undo it.
#[test]
fn the_posix_write_restore_is_skipped_only_for_a_windows_local_target() {
    assert!(crate::paths::windows_host_target(true, "local"));
    assert!(!crate::paths::windows_host_target(true, "build-box"));
    assert!(!crate::paths::windows_host_target(false, "local"));

    assert!(restore_write_access_cmd("/wt", false).is_some());
    assert_eq!(restore_write_access_cmd("/wt", true), None);
}

/// Shortening is a Windows-only measure and it changes only the worktree
/// suffix, so every path an existing installation already resolves is
/// byte-identical.
#[test]
fn shortening_touches_the_worktree_suffix_and_nothing_else() {
    let full = worktree_dir(
        "/w/projects/p1781624953648/repos/app",
        "f-1-step-s-implement",
        false,
    );
    assert_eq!(
        full,
        "/w/projects/p1781624953648/repos/app_wt_f-1-step-s-implement"
    );

    let short = worktree_dir(
        "/w/projects/p1781624953648/repos/app",
        "f-1-step-s-implement",
        true,
    );
    assert!(
        short.starts_with("/w/projects/p1781624953648/repos/app_wt_"),
        "only the suffix moves: {short}"
    );
    assert_eq!(
        short.len(),
        "/w/projects/p1781624953648/repos/app_wt_".len() + crate::paths::SHORT_SEGMENT_LEN,
        "the suffix is a fixed-width segment: {short}"
    );
}

/// Demeteo's ids are `<tag><wall-clock millis>`, so the first eight characters
/// are the *high* digits of a timestamp and change once every ~16 minutes. A
/// literal prefix would give two features created the same afternoon one shared
/// worktree directory — which
/// `test_provision_subtask_worktree_same_repo_two_features_do_not_collide`
/// records as data loss, not as a name clash.
#[test]
fn ids_that_differ_only_in_their_tail_get_different_segments() {
    let a = crate::paths::short_path_segment("f-1781624953648-step-s-implement");
    let b = crate::paths::short_path_segment("f-1781624953649-step-s-implement");
    let c = crate::paths::short_path_segment("f-1781624953648-step-s-validate");
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.len(), crate::paths::SHORT_SEGMENT_LEN);
    assert!(a
        .chars()
        .all(|ch| ch.is_ascii_hexdigit() && !ch.is_uppercase()));
}

/// The directory outlives the process that made it, so the segment has to be
/// the same in the next build too. Pinned as a literal: a change of algorithm
/// strands every worktree a previous build left behind, and nothing else would
/// notice.
#[test]
fn the_segment_is_pinned_across_builds() {
    assert_eq!(crate::paths::short_path_segment(""), "cbf29ce4");
    assert_eq!(
        crate::paths::short_path_segment("f-1-step-s-implement"),
        "8734e820"
    );
}

/// The probe's answer reaches a shell command and a git exclude file, so it is
/// read back as entries of the compiled-in list rather than as strings. A login
/// banner, a git warning, or anything else on that stream names nothing.
#[test]
fn the_cache_probe_answer_is_matched_against_the_known_names() {
    let answer = "node_modules\n  target  \nWelcome to Ubuntu\n../../etc\n";
    assert_eq!(
        shareable_cache_names(answer),
        vec!["node_modules", "target"]
    );
    assert!(shareable_cache_names("").is_empty());
}

/// `run_command` treats any non-zero exit as `Err` and would discard the whole
/// answer, and the loop's exit status is its last `if`'s.
#[test]
fn the_cache_probe_cannot_exit_non_zero_on_its_last_test() {
    assert!(shareable_cache_probe_cmd("/repo").ends_with("; true"));
}

/// Only the names the probe cleared are linked — the gate is not re-spelled in
/// the linking command, so the two cannot disagree about `vendor/`.
#[test]
fn only_the_probed_names_are_linked() {
    let cmd = link_dependency_caches_cmd("/repo", "/wt", "/cache", &["node_modules"]);
    assert!(cmd.contains("for d in node_modules;"));
    assert!(!cmd.contains("vendor"));
    assert!(!cmd.contains("check-ignore"));
}

/// The exclude file belongs to a repository Demeteo cloned but whose contents
/// it does not own, so entries are appended and a user's own lines survive.
#[test]
fn exclusions_are_appended_and_never_repeated() {
    let existing = "# my own\n*.log\n";
    let first = exclude_file_with(existing, &["node_modules", "target"])
        .expect("two new names are appended");
    assert!(
        first.starts_with(existing),
        "the user's lines survive: {first}"
    );
    assert!(first.contains("\nnode_modules\n"));
    assert!(first.ends_with("target\n"));

    assert_eq!(
        exclude_file_with(&first, &["node_modules", "target"]),
        None,
        "a second provisioning of the same repository writes nothing"
    );
    assert!(exclude_file_with(&first, &["venv"]).is_some());
}

/// An exclude file whose last line has no newline must not have the next entry
/// welded onto it.
#[test]
fn an_unterminated_exclude_file_still_gets_a_separate_line() {
    let updated = exclude_file_with("*.log", &["target"]).expect("a new name is appended");
    assert!(
        updated.lines().any(|line| line == "*.log"),
        "the unterminated line stays whole: {updated:?}"
    );
    assert!(updated.lines().any(|line| line == "target"));
}

/// `git rev-parse` may or may not answer with a trailing separator, and a
/// doubled one is a path SFTP will not resolve.
///
/// The other half of this — that the join never uses the *host's* separator,
/// which is what would send `…\info\exclude` to a Linux machine from a Windows
/// desktop — is not observable from a Linux test, since `Path::join` agrees
/// here. It is stated on the function instead.
#[test]
fn a_target_path_is_joined_without_doubling_a_separator() {
    assert_eq!(
        target_path_join("/repo/.git", "info/exclude"),
        "/repo/.git/info/exclude"
    );
    assert_eq!(
        target_path_join("/repo/.git/", "info/exclude"),
        "/repo/.git/info/exclude"
    );
    assert_eq!(
        target_path_join("C:/r/.git", "info/exclude"),
        "C:/r/.git/info/exclude"
    );
}

/// End to end: provisioning a worktree against a checkout that has a gitignored
/// `node_modules` records the name in the clone's own `.git/info/exclude`, so
/// the symlink it then creates is invisible to `git add -A` without any
/// pathspec.
#[tokio::test]
async fn provisioning_records_the_shared_caches_in_the_clone_exclude_file() {
    let (dir, helper) = make_repo("wt_exclude_entry").await;
    let repo = dir.to_string_lossy().to_string();
    let exec = fresh_exec();

    exec.write_file("local", &format!("{repo}/.gitignore"), "node_modules/\n")
        .await
        .unwrap();
    exec.write_file(
        "local",
        &format!("{repo}/node_modules/dep/index.js"),
        "//x\n",
    )
    .await
    .unwrap();
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{repo}\" add -A && git -C \"{repo}\" commit -m ignore"),
        )
        .await;

    let wt = helper
        .provision_subtask_worktree(None, &repo, "main", "sub-exclude")
        .await
        .expect("provisioning succeeds");

    let exclude = std::fs::read_to_string(format!("{repo}/.git/info/exclude")).unwrap();
    assert!(
        exclude.lines().any(|line| line == "node_modules"),
        "the shared cache is excluded slashlessly, got: {exclude:?}"
    );
    assert!(
        !exclude.lines().any(|line| line == "vendor"),
        "a name the checkout does not have is not excluded, got: {exclude:?}"
    );

    let untracked = exec
        .run_command("local", &format!("git -C \"{wt}\" status --porcelain"))
        .await
        .unwrap();
    assert!(
        !untracked.contains("node_modules"),
        "the linked cache is ignored rather than staged, got: {untracked:?}"
    );

    let _ = helper
        .cleanup_subtask_worktree(None, &repo, "main", "sub-exclude")
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(feature_cache_dir(&repo, "main"));
}

/// The last component of a detached baseline worktree, which is what git
/// replays: the leading directories are physical in git's answer and logical
/// here, and on macOS `/var` and `/private/var` name the same directory and
/// compare unequal.
fn baseline_suffix(repo: &str) -> String {
    std::path::Path::new(&worktree_dir_on(repo, "baseline", "local"))
        .file_name()
        .expect("a worktree directory has a name")
        .to_string_lossy()
        .into_owned()
}
