use super::GitOpsHelper;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;

/// Helper: create a fresh git repo in a temp dir and return (repo_dir, git_ops).
pub(super) async fn make_repo(suffix: &str) -> (PathBuf, GitOpsHelper) {
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

/// Set up two local repos and wire them together as fake
/// origin/main. The "remote" is a regular working tree that
/// we push to via a bare-clone URL; the "local" is a normal
/// working tree that we sync from. Both start with the same
/// initial commit. The caller mutates each side to set up the
/// upstream/feature divergence before calling
/// `sync_feature_with_upstream`.
pub(super) async fn make_two_repos(suffix: &str) -> (PathBuf, PathBuf, GitOpsHelper) {
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

/// Small helper: `git rev-parse <ref>` in `dir`, trimmed to the bare SHA.
pub(super) async fn rev_parse(exec: &LocalSubprocessAdapter, dir: &str, rev: &str) -> String {
    exec.run_command("local", &format!("git -C \"{dir}\" rev-parse {rev}"))
        .await
        .unwrap()
        .trim()
        .to_string()
}

/// Build a repo whose `feature/f-sq` branch is checked out and carries
/// `n` commits on top of `main`, each adding one file. Returns the repo path.
pub(super) async fn make_repo_with_feature_commits(
    suffix: &str,
    n: usize,
) -> (PathBuf, GitOpsHelper, String) {
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

/// Convenience: a fresh [LocalSubprocessAdapter] for inline-use tests.
pub(super) fn fresh_exec() -> LocalSubprocessAdapter {
    LocalSubprocessAdapter::new()
}

/// Convenience: build origin/local two-repo layout and advance `origin/main`
/// by one commit with the given message. Used by the `ensure_default_branch_*`
/// tests.
pub(super) async fn two_repos_with_origin_ahead(
    suffix: &str,
    advance_msg: &str,
) -> (PathBuf, PathBuf, GitOpsHelper, String) {
    let (local_dir, remote_dir, helper) = make_two_repos(suffix).await;
    let remote = remote_dir.to_string_lossy().to_string();
    let exec = LocalSubprocessAdapter::new();
    exec.write_file("local", &format!("{remote}/README.md"), "origin advanced")
        .await
        .unwrap();
    let _ = exec
        .run_command(
            "local",
            &format!("git -C \"{remote}\" commit -am {advance_msg}"),
        )
        .await;
    let remote = remote_dir.to_string_lossy().to_string();
    (local_dir, remote_dir, helper, remote)
}
